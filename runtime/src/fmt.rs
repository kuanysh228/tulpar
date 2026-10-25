//! Басып шығару, мәтінге айналдыру (тип дескрипторлары бойынша), енгізу, аварияның хабарламалары.

use crate::dict;
use crate::gc;
use crate::list;
use crate::str::*;
use crate::sys::*;

/// Қолмен жадта өсетін байт буфері.
pub struct Sb {
    p: *mut u8,
    len: usize,
    cap: usize,
}

impl Sb {
    pub fn new() -> Sb {
        Sb { p: gc::mem_alloc(64), len: 0, cap: 64 }
    }
    pub fn push_bytes(&mut self, b: &[u8]) {
        if self.len + b.len() > self.cap {
            let mut ncap = self.cap * 2;
            while ncap < self.len + b.len() {
                ncap *= 2;
            }
            let np = gc::mem_alloc(ncap);
            unsafe {
                memcpy(np, self.p, self.len);
            }
            gc::mem_free(self.p);
            self.p = np;
            self.cap = ncap;
        }
        unsafe {
            memcpy(self.p.add(self.len), b.as_ptr(), b.len());
        }
        self.len += b.len();
    }
    pub fn push(&mut self, c: u8) {
        self.push_bytes(&[c]);
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.p, self.len) }
    }
    pub fn finish(self) -> Str {
        let s = unsafe { from_bytes(self.bytes()) };
        gc::mem_free(self.p);
        s
    }
    pub fn discard(self) {
        gc::mem_free(self.p);
    }
}

static mut CLASS_TABLE: *const *const u8 = core::ptr::null();

pub fn set_class_table(t: *const *const u8) {
    unsafe {
        CLASS_TABLE = t;
    }
}

unsafe fn skip_desc(p: *const u8) -> *const u8 {
    match *p {
        b'L' | b'S' => skip_desc(p.add(1)),
        b'D' => skip_desc(skip_desc(p.add(1))),
        b'C' => p.add(3),
        b'T' => {
            let n = *p.add(1) as usize;
            let mut q = p.add(2);
            for _ in 0..n {
                q = skip_desc(q);
            }
            q
        }
        _ => p.add(1),
    }
}

fn push_hex(sb: &mut Sb, mut v: u64) {
    let mut buf = [0u8; 16];
    let mut i = 16;
    if v == 0 {
        i -= 1;
        buf[i] = b'0';
    }
    while v > 0 {
        i -= 1;
        let d = (v & 15) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        v >>= 4;
    }
    sb.push_bytes(b"0x");
    sb.push_bytes(&buf[i..]);
}

unsafe fn push_quoted(sb: &mut Sb, s: &[u8]) {
    sb.push(b'\'');
    for &c in s {
        match c {
            b'\'' => sb.push_bytes(b"\\'"),
            b'\\' => sb.push_bytes(b"\\\\"),
            b'\n' => sb.push_bytes(b"\\n"),
            b'\t' => sb.push_bytes(b"\\t"),
            b'\r' => sb.push_bytes(b"\\r"),
            _ => sb.push(c),
        }
    }
    sb.push(b'\'');
}

/// `bits` мәнін дескриптор бойынша жазады.
pub unsafe fn repr(sb: &mut Sb, bits: u64, desc: *const u8, quote: bool, depth: u32) {
    if depth > 24 {
        sb.push_bytes(b"...");
        return;
    }
    match *desc {
        b'i' | b'c' => {
            let mut b = [0u8; 24];
            sb.push_bytes(fmt_i64(bits as i64, &mut b));
        }
        b'f' => {
            let mut w = BufW::new();
            fmt_float(f64::from_bits(bits), &mut w);
            sb.push_bytes(w.as_bytes());
        }
        b'b' => sb.push_bytes(if bits != 0 { "рас".as_bytes() } else { "жалған".as_bytes() }),
        b's' => {
            if quote {
                push_quoted(sb, bytes(bits as *const u64));
            } else {
                sb.push_bytes(bytes(bits as *const u64));
            }
        }
        b'p' => push_hex(sb, bits),
        b'L' => {
            if bits == 0 {
                sb.push_bytes("жоқ".as_bytes());
                return;
            }
            let l = bits as list::List;
            let ed = desc.add(1);
            sb.push(b'[');
            let n = list::len(l);
            for i in 0..n {
                if i > 0 {
                    sb.push_bytes(b", ");
                }
                repr(sb, list::get(l, i), ed, true, depth + 1);
            }
            sb.push(b']');
        }
        b'T' => {
            let n = *desc.add(1) as usize;
            let obj = bits as *const u64;
            let mut ed = desc.add(2);
            sb.push(b'(');
            for i in 0..n {
                if i > 0 {
                    sb.push_bytes(b", ");
                }
                repr(sb, *obj.add(i), ed, true, depth + 1);
                ed = skip_desc(ed);
            }
            if n == 1 {
                sb.push(b',');
            }
            sb.push(b')');
        }
        b'S' => {
            if bits == 0 {
                sb.push_bytes("жоқ".as_bytes());
                return;
            }
            let d = bits as dict::Dict;
            if dict::tlp_dict_len(d) == 0 {
                sb.push_bytes("жиын()".as_bytes());
                return;
            }
            sb.push(b'{');
            let mut pos = 0;
            let mut first = true;
            loop {
                pos = dict::tlp_dict_next(d, pos);
                if pos < 0 {
                    break;
                }
                if !first {
                    sb.push_bytes(b", ");
                }
                first = false;
                repr(sb, dict::tlp_dict_key_at(d, pos), desc.add(1), true, depth + 1);
                pos += 1;
            }
            sb.push(b'}');
        }
        b'D' => {
            if bits == 0 {
                sb.push_bytes("жоқ".as_bytes());
                return;
            }
            let d = bits as dict::Dict;
            let kd = desc.add(1);
            let vd = skip_desc(kd);
            sb.push(b'{');
            let mut pos = 0;
            let mut first = true;
            loop {
                pos = dict::tlp_dict_next(d, pos);
                if pos < 0 {
                    break;
                }
                if !first {
                    sb.push_bytes(b", ");
                }
                first = false;
                repr(sb, dict::tlp_dict_key_at(d, pos), kd, true, depth + 1);
                sb.push_bytes(b": ");
                repr(sb, dict::tlp_dict_val_at(d, pos), vd, true, depth + 1);
                pos += 1;
            }
            sb.push(b'}');
        }
        b'C' => {
            if bits == 0 {
                sb.push_bytes("жоқ".as_bytes());
                return;
            }
            let id = u16::from_le_bytes([*desc.add(1), *desc.add(2)]) as usize;
            let mut cd = *CLASS_TABLE.add(id);
            let hdr = *cd as usize;
            if hdr == 1 {
                // иерархия: нақты класс объектінің 0-ші сөзінде
                cd = *CLASS_TABLE.add(*(bits as *const u64) as usize);
            }
            cd = cd.add(3);
            let nl = *cd as usize;
            cd = cd.add(1);
            sb.push_bytes(core::slice::from_raw_parts(cd, nl));
            cd = cd.add(nl);
            let nf = *cd as usize;
            cd = cd.add(1);
            sb.push(b'(');
            let obj = bits as *const u64;
            for i in 0..nf {
                if i > 0 {
                    sb.push_bytes(b", ");
                }
                let fl = *cd as usize;
                cd = cd.add(1);
                sb.push_bytes(core::slice::from_raw_parts(cd, fl));
                cd = cd.add(fl);
                sb.push(b'=');
                repr(sb, *obj.add(i + hdr), cd, true, depth + 1);
                cd = skip_desc(cd);
            }
            sb.push(b')');
        }
        _ => sb.push_bytes("жоқ".as_bytes()),
    }
}

/// Құрылымдық теңдік дескриптор бойынша.
pub unsafe fn deep_eq(a: u64, b: u64, desc: *const u8) -> bool {
    match *desc {
        b'f' => f64::from_bits(a) == f64::from_bits(b),
        b's' => tlp_str_eq(a as Str, b as Str) != 0,
        b'L' => {
            if a == b {
                return true;
            }
            if a == 0 || b == 0 {
                return false;
            }
            let (x, y) = (a as list::List, b as list::List);
            let n = list::len(x);
            if n != list::len(y) {
                return false;
            }
            let ed = desc.add(1);
            for i in 0..n {
                if !deep_eq(list::get(x, i), list::get(y, i), ed) {
                    return false;
                }
            }
            true
        }
        b'T' => {
            if a == b {
                return true;
            }
            let n = *desc.add(1) as usize;
            let (x, y) = (a as *const u64, b as *const u64);
            let mut ed = desc.add(2);
            for i in 0..n {
                if !deep_eq(*x.add(i), *y.add(i), ed) {
                    return false;
                }
                ed = skip_desc(ed);
            }
            true
        }
        b'S' => {
            if a == b {
                return true;
            }
            if a == 0 || b == 0 {
                return false;
            }
            let (x, y) = (a as dict::Dict, b as dict::Dict);
            if dict::tlp_dict_len(x) != dict::tlp_dict_len(y) {
                return false;
            }
            let mut pos = 0;
            loop {
                pos = dict::tlp_dict_next(x, pos);
                if pos < 0 {
                    break;
                }
                if dict::tlp_dict_has(y, dict::tlp_dict_key_at(x, pos)) == 0 {
                    return false;
                }
                pos += 1;
            }
            true
        }
        b'D' => {
            if a == b {
                return true;
            }
            if a == 0 || b == 0 {
                return false;
            }
            let (x, y) = (a as dict::Dict, b as dict::Dict);
            if dict::tlp_dict_len(x) != dict::tlp_dict_len(y) {
                return false;
            }
            let kd = desc.add(1);
            let vd = skip_desc(kd);
            let mut pos = 0;
            loop {
                pos = dict::tlp_dict_next(x, pos);
                if pos < 0 {
                    break;
                }
                let k = dict::tlp_dict_key_at(x, pos);
                if dict::tlp_dict_has(y, k) == 0 {
                    return false;
                }
                if !deep_eq(dict::tlp_dict_val_at(x, pos), dict::tlp_dict_get(y, k), vd) {
                    return false;
                }
                pos += 1;
            }
            true
        }
        _ => a == b,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_deep_eq(a: u64, b: u64, desc: *const u8) -> i64 {
    deep_eq(a, b, desc) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_to_str(bits: u64, desc: *const u8) -> Str {
    let mut sb = Sb::new();
    repr(&mut sb, bits, desc, false, 0);
    sb.finish()
}

// ------------------------------------------------------------------ print

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_print_int(v: i64) {
    let mut b = [0u8; 24];
    out_write(fmt_i64(v, &mut b));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_print_float(x: f64) {
    let mut w = BufW::new();
    fmt_float(x, &mut w);
    out_write(w.as_bytes());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_print_str(s: Str) {
    out_write(bytes(s));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_print_bool(b: i64) {
    out_write(if b != 0 { "рас".as_bytes() } else { "жалған".as_bytes() });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_print_any(bits: u64, desc: *const u8) {
    let mut sb = Sb::new();
    repr(&mut sb, bits, desc, false, 0);
    out_write(sb.bytes());
    sb.discard();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_print_sep() {
    out_write(b" ");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_print_nl() {
    out_write(b"\n");
}

// ------------------------------------------------------------------ енгізу

const IN_CAP: usize = 1 << 16;
static mut IN_BUF: [u8; IN_CAP] = [0; IN_CAP];
static mut IN_POS: usize = 0;
static mut IN_LEN: usize = 0;
static mut IN_EOF: bool = false;

unsafe fn fill() -> bool {
    if IN_EOF {
        return false;
    }
    loop {
        let r = sys3(SYS_READ, 0, core::ptr::addr_of_mut!(IN_BUF) as usize, IN_CAP);
        if r == -4 {
            continue;
        }
        if r <= 0 {
            IN_EOF = true;
            return false;
        }
        IN_POS = 0;
        IN_LEN = r as usize;
        return true;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_input(prompt: Str) -> Str {
    if !prompt.is_null() {
        out_write(bytes(prompt));
    }
    out_flush();
    let mut sb = Sb::new();
    let mut got_any = false;
    loop {
        if IN_POS >= IN_LEN && !fill() {
            break;
        }
        got_any = true;
        let buf = core::slice::from_raw_parts((core::ptr::addr_of!(IN_BUF) as *const u8).add(IN_POS), IN_LEN - IN_POS);
        match buf.iter().position(|&c| c == b'\n') {
            Some(i) => {
                sb.push_bytes(&buf[..i]);
                IN_POS += i + 1;
                let mut b = sb.bytes();
                if b.last() == Some(&b'\r') {
                    b = &b[..b.len() - 1];
                }
                let s = from_bytes(b);
                sb.discard();
                return s;
            }
            None => {
                sb.push_bytes(buf);
                IN_POS = IN_LEN;
            }
        }
    }
    if !got_any || sb.bytes().is_empty() {
        sb.discard();
        fatal("енгізу аяқталды (EOF)".as_bytes());
    }
    let s = from_bytes(sb.bytes());
    sb.discard();
    s
}

// ------------------------------------------------------------------ аварияның хабарламалары

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_panic_str(s: Str) -> ! {
    fatal(bytes(s))
}

unsafe fn class_parent(id: usize) -> usize {
    let cd = *CLASS_TABLE.add(id);
    u16::from_le_bytes([*cd.add(1), *cd.add(2)]) as usize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_isa(obj: u64, target: i64) -> i64 {
    if obj == 0 {
        return 0;
    }
    let mut id = *(obj as *const u64) as usize;
    loop {
        if id == target as usize {
            return 1;
        }
        let p = class_parent(id);
        if p == 0xFFFF {
            return 0;
        }
        id = p;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_downcast(obj: u64, target: i64) -> u64 {
    if obj != 0 && tlp_isa(obj, target) == 0 {
        let mut sb = Sb::new();
        sb.push_bytes("түрлендіру мүмкін емес: объект ".as_bytes());
        let cd = *CLASS_TABLE.add(*(obj as *const u64) as usize);
        let nl = *cd.add(3) as usize;
        sb.push_bytes(core::slice::from_raw_parts(cd.add(4), nl));
        sb.push_bytes(" класынан, ал күтілген ".as_bytes());
        let td = *CLASS_TABLE.add(target as usize);
        let tl = *td.add(3) as usize;
        sb.push_bytes(core::slice::from_raw_parts(td.add(4), tl));
        fatal(sb.bytes());
    }
    obj
}
