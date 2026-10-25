//! Мәтіндер: [байт ұзындығы: u64][таңба саны: u64][байттар...]. Өзгермейтін, UTF-8, GC-де `noscan`.
//! Индекстеу мен кесу таңбалар (Unicode code point) бойынша; ASCII мәтін үшін O(1).

use crate::gc;
use crate::list;
use crate::sys::*;
use core::fmt::Write;

pub type Str = *mut u64;

/// Байт ұзындығы `len`; таңба саны уақытша `len` (ASCII), қажет болса `fix_chars` шақырыңыз.
pub unsafe fn str_alloc(len: usize) -> Str {
    let p = gc::alloc(16 + len, true) as *mut u64;
    *p = len as u64;
    *p.add(1) = len as u64;
    p
}

pub unsafe fn fix_chars(s: Str) {
    *s.add(1) = char_count(bytes(s)) as u64;
}

#[inline(always)]
pub unsafe fn nchars(s: *const u64) -> usize {
    if s.is_null() { 0 } else { *s.add(1) as usize }
}

pub unsafe fn bytes<'a>(s: *const u64) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    core::slice::from_raw_parts((s as *const u8).add(16), *s as usize)
}

pub unsafe fn data(s: Str) -> *mut u8 {
    (s as *mut u8).add(16)
}

pub unsafe fn from_bytes(b: &[u8]) -> Str {
    let s = str_alloc(b.len());
    memcpy(data(s), b.as_ptr(), b.len());
    fix_chars(s);
    s
}

pub unsafe fn from_str(b: &str) -> Str {
    from_bytes(b.as_bytes())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_concat(a: Str, b: Str) -> Str {
    let (x, y) = (bytes(a), bytes(b));
    if x.is_empty() && !b.is_null() {
        return b;
    }
    if y.is_empty() && !a.is_null() {
        return a;
    }
    let r = str_alloc(x.len() + y.len());
    memcpy(data(r), x.as_ptr(), x.len());
    memcpy(data(r).add(x.len()), y.as_ptr(), y.len());
    *r.add(1) = (nchars(a) + nchars(b)) as u64;
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_eq(a: Str, b: Str) -> i64 {
    if a == b {
        return 1;
    }
    let (x, y) = (bytes(a), bytes(b));
    (x.len() == y.len() && memcmp(x.as_ptr(), y.as_ptr(), x.len()) == 0) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_cmp(a: Str, b: Str) -> i64 {
    let (x, y) = (bytes(a), bytes(b));
    let n = x.len().min(y.len());
    let c = memcmp(x.as_ptr(), y.as_ptr(), n);
    if c != 0 {
        return if c < 0 { -1 } else { 1 };
    }
    if x.len() < y.len() {
        -1
    } else if x.len() > y.len() {
        1
    } else {
        0
    }
}

pub fn norm_slice(len: i64, lo: i64, hi: i64) -> (usize, usize) {
    let mut lo = if lo < 0 { lo + len } else { lo };
    let mut hi = if hi < 0 { hi + len } else { hi };
    lo = lo.clamp(0, len);
    hi = hi.clamp(0, len);
    if hi < lo {
        hi = lo;
    }
    (lo as usize, hi as usize)
}

/// Таңба индексінен байт ығысуына.
fn byte_off(b: &[u8], ascii: bool, ci: usize) -> usize {
    if ascii {
        return ci;
    }
    let mut off = 0;
    let mut k = 0;
    while k < ci && off < b.len() {
        off += char_len_at(b, off);
        k += 1;
    }
    off
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_slice(s: Str, lo: i64, hi: i64) -> Str {
    let b = bytes(s);
    let nc = nchars(s);
    let (l, h) = norm_slice(nc as i64, lo, hi);
    if l == 0 && h == nc && !s.is_null() {
        return s;
    }
    let ascii = nc == b.len();
    let bl = byte_off(b, ascii, l);
    let bh = bl + byte_off(&b[bl..], ascii, h - l);
    let r = str_alloc(bh - bl);
    memcpy(data(r), b.as_ptr().add(bl), bh - bl);
    *r.add(1) = (h - l) as u64;
    r
}

static ASCII_TABLE: [[u64; 3]; 128] = {
    let mut t = [[1u64, 1u64, 0u64]; 128];
    let mut i = 0;
    while i < 128 {
        t[i][2] = i as u64;
        i += 1;
    }
    t
};

/// `s[i]` — i-ші таңба (мәтін ретінде).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_index(s: Str, i: i64) -> Str {
    let b = bytes(s);
    let nc = nchars(s) as i64;
    let idx = if i < 0 { i + nc } else { i };
    if idx < 0 || idx >= nc {
        crate::list::tlp_panic_index(i, nc);
    }
    if nc as usize == b.len() {
        let c = b[idx as usize];
        return &ASCII_TABLE[c as usize] as *const [u64; 3] as *mut u64;
    }
    let off = byte_off(b, false, idx as usize);
    from_bytes(&b[off..off + char_len_at(b, off)])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_repeat(s: Str, n: i64) -> Str {
    let b = bytes(s);
    if n <= 0 || b.is_empty() {
        return from_bytes(&[]);
    }
    let total = match b.len().checked_mul(n as usize) {
        Some(t) if t < (1 << 40) => t,
        _ => fatal("мәтінді қайталау тым үлкен".as_bytes()),
    };
    let r = str_alloc(total);
    let mut off = 0;
    for _ in 0..n {
        memcpy(data(r).add(off), b.as_ptr(), b.len());
        off += b.len();
    }
    *r.add(1) = (nchars(s) * n as usize) as u64;
    r
}

fn char_len_at(b: &[u8], pos: usize) -> usize {
    let c = b[pos];
    let n = if c < 0x80 {
        1
    } else if c >= 0xF0 {
        4
    } else if c >= 0xE0 {
        3
    } else if c >= 0xC0 {
        2
    } else {
        1
    };
    n.min(b.len() - pos)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_char_len(s: Str, pos: i64) -> i64 {
    char_len_at(bytes(s), pos as usize) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_char(s: Str, pos: i64) -> Str {
    let b = bytes(s);
    let n = char_len_at(b, pos as usize);
    from_bytes(&b[pos as usize..pos as usize + n])
}

// ------------------------------------------------------------------ регистр

fn decode(b: &[u8], i: usize) -> (u32, usize) {
    let c = b[i] as u32;
    let n = char_len_at(b, i);
    let cp = match n {
        1 => c,
        2 => ((c & 0x1F) << 6) | (b[i + 1] as u32 & 0x3F),
        3 => ((c & 0x0F) << 12) | ((b[i + 1] as u32 & 0x3F) << 6) | (b[i + 2] as u32 & 0x3F),
        _ => ((c & 0x07) << 18) | ((b[i + 1] as u32 & 0x3F) << 12) | ((b[i + 2] as u32 & 0x3F) << 6) | (b[i + 3] as u32 & 0x3F),
    };
    (cp, n)
}

fn encode(cp: u32, out: &mut [u8; 4]) -> usize {
    if cp < 0x80 {
        out[0] = cp as u8;
        1
    } else if cp < 0x800 {
        out[0] = 0xC0 | (cp >> 6) as u8;
        out[1] = 0x80 | (cp & 0x3F) as u8;
        2
    } else if cp < 0x10000 {
        out[0] = 0xE0 | (cp >> 12) as u8;
        out[1] = 0x80 | ((cp >> 6) & 0x3F) as u8;
        out[2] = 0x80 | (cp & 0x3F) as u8;
        3
    } else {
        out[0] = 0xF0 | (cp >> 18) as u8;
        out[1] = 0x80 | ((cp >> 12) & 0x3F) as u8;
        out[2] = 0x80 | ((cp >> 6) & 0x3F) as u8;
        out[3] = 0x80 | (cp & 0x3F) as u8;
        4
    }
}

fn to_lower(c: u32) -> u32 {
    match c {
        0x41..=0x5A => c + 32,
        0x410..=0x42F => c + 32,
        0x400..=0x40F => c + 0x50,
        0x460..=0x481 | 0x48A..=0x4BF | 0x4D0..=0x52F if c % 2 == 0 => c + 1,
        0x4C1..=0x4CD if c % 2 == 1 => c + 1,
        0x4C0 => 0x4CF,
        _ => c,
    }
}

fn to_upper(c: u32) -> u32 {
    match c {
        0x61..=0x7A => c - 32,
        0x430..=0x44F => c - 32,
        0x450..=0x45F => c - 0x50,
        0x460..=0x481 | 0x48A..=0x4BF | 0x4D0..=0x52F if c % 2 == 1 => c - 1,
        0x4C1..=0x4CE if c % 2 == 0 => c - 1,
        0x4CF => 0x4C0,
        _ => c,
    }
}

unsafe fn map_case(s: Str, f: fn(u32) -> u32) -> Str {
    let b = bytes(s);
    let r = str_alloc(b.len() * 2 + 4);
    let mut out = 0;
    let mut i = 0;
    while i < b.len() {
        let (cp, n) = decode(b, i);
        let m = f(cp);
        let mut tmp = [0u8; 4];
        let k = if m == cp { 0 } else { encode(m, &mut tmp) };
        if m == cp {
            memcpy(data(r).add(out), b.as_ptr().add(i), n);
            out += n;
        } else {
            memcpy(data(r).add(out), tmp.as_ptr(), k);
            out += k;
        }
        i += n;
    }
    *r = out as u64;
    fix_chars(r);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_upper(s: Str) -> Str {
    map_case(s, to_upper)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_lower(s: Str) -> Str {
    map_case(s, to_lower)
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_strip(s: Str) -> Str {
    let b = bytes(s);
    let mut l = 0;
    let mut h = b.len();
    while l < h && is_ws(b[l]) {
        l += 1;
    }
    while h > l && is_ws(b[h - 1]) {
        h -= 1;
    }
    if l == 0 && h == b.len() && !s.is_null() {
        return s;
    }
    from_bytes(&b[l..h])
}

fn find_in(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
    if n.is_empty() {
        return Some(from.min(h.len()));
    }
    if n.len() > h.len() {
        return None;
    }
    let last = h.len() - n.len();
    let first = n[0];
    let mut i = from;
    while i <= last {
        if h[i] == first && h[i..i + n.len()] == *n {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_find(s: Str, sub: Str) -> i64 {
    let b = bytes(s);
    match find_in(b, bytes(sub), 0) {
        Some(i) => char_count(&b[..i]) as i64,
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_contains(hay: Str, needle: Str) -> i64 {
    find_in(bytes(hay), bytes(needle), 0).is_some() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_starts(s: Str, p: Str) -> i64 {
    bytes(s).starts_with(bytes(p)) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_ends(s: Str, p: Str) -> i64 {
    bytes(s).ends_with(bytes(p)) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_split(s: Str, sep: Str) -> list::List {
    let (b, sp) = (bytes(s), bytes(sep));
    if sp.is_empty() {
        fatal("`бөлу`: бөлгіш бос болмауы керек".as_bytes());
    }
    let l = list::list_new(4, 0);
    let mut start = 0;
    loop {
        match find_in(b, sp, start) {
            Some(i) => {
                let part = from_bytes(&b[start..i]);
                list::list_push(l, part as u64);
                start = i + sp.len();
            }
            None => {
                let part = from_bytes(&b[start..]);
                list::list_push(l, part as u64);
                break;
            }
        }
    }
    l
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_join(sep: Str, l: list::List) -> Str {
    let sp = bytes(sep);
    let n = list::len(l);
    let d = list::data(l);
    let mut total = 0;
    for i in 0..n {
        total += bytes(*d.add(i) as *const u64).len();
    }
    if n > 0 {
        total += sp.len() * (n - 1);
    }
    let r = str_alloc(total);
    let mut off = 0;
    for i in 0..n {
        if i > 0 {
            memcpy(data(r).add(off), sp.as_ptr(), sp.len());
            off += sp.len();
        }
        let p = bytes(*d.add(i) as *const u64);
        memcpy(data(r).add(off), p.as_ptr(), p.len());
        off += p.len();
    }
    fix_chars(r);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_replace(s: Str, a: Str, b: Str) -> Str {
    let (h, from, to) = (bytes(s), bytes(a), bytes(b));
    if from.is_empty() {
        return s;
    }
    let mut sb = crate::fmt::Sb::new();
    let mut start = 0;
    while let Some(i) = find_in(h, from, start) {
        sb.push_bytes(&h[start..i]);
        sb.push_bytes(to);
        start = i + from.len();
    }
    sb.push_bytes(&h[start..]);
    sb.finish()
}

// ------------------------------------------------------------------ түрлендіру

pub struct BufW {
    pub buf: [u8; 512],
    pub n: usize,
}
impl BufW {
    pub fn new() -> BufW {
        BufW { buf: [0; 512], n: 0 }
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.n]
    }
}
impl Write for BufW {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        if self.n + b.len() > self.buf.len() {
            return Err(core::fmt::Error);
        }
        self.buf[self.n..self.n + b.len()].copy_from_slice(b);
        self.n += b.len();
        Ok(())
    }
}

/// Python-ға ұқсас float көрінісі.
pub fn fmt_float(x: f64, w: &mut BufW) {
    if x.is_nan() {
        let _ = w.write_str("nan");
        return;
    }
    if x.is_infinite() {
        let _ = w.write_str(if x < 0.0 { "-inf" } else { "inf" });
        return;
    }
    let mut tmp = BufW::new();
    let _ = write!(tmp, "{:?}", x);
    let s = tmp.as_bytes();
    // Python стилі: оң дәрежеге `+`, дәреже кемінде 2 таңба: 1e+16, 1.5e-07
    if let Some(e) = s.iter().position(|&c| c == b'e') {
        let _ = w.write_str(core::str::from_utf8(&s[..=e]).unwrap());
        let mut rest = &s[e + 1..];
        if rest.first() == Some(&b'-') {
            let _ = w.write_str("-");
            rest = &rest[1..];
        } else {
            let _ = w.write_str("+");
        }
        if rest.len() < 2 {
            let _ = w.write_str("0");
        }
        let _ = w.write_str(core::str::from_utf8(rest).unwrap());
    } else {
        let _ = w.write_str(core::str::from_utf8(s).unwrap());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_from_int(v: i64) -> Str {
    let mut buf = [0u8; 24];
    from_bytes(fmt_i64(v, &mut buf))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_from_float(x: f64) -> Str {
    let mut w = BufW::new();
    fmt_float(x, &mut w);
    from_bytes(w.as_bytes())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_from_bool(b: i64) -> Str {
    from_str(if b != 0 { "рас" } else { "жалған" })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_fixed(x: f64, prec: i64) -> Str {
    let mut w = BufW::new();
    if x.is_nan() {
        let _ = w.write_str("nan");
    } else if x.is_infinite() {
        let _ = w.write_str(if x < 0.0 { "-inf" } else { "inf" });
    } else if x.abs() >= 1e200 {
        fmt_float(x, &mut w);
    } else {
        let p = prec.clamp(0, 40) as usize;
        let _ = write!(w, "{:.*}", p, x);
    }
    from_bytes(w.as_bytes())
}

pub fn char_count(b: &[u8]) -> usize {
    b.iter().filter(|&&c| (c & 0xC0) != 0x80).count()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_pad(s: Str, width: i64, align: i64) -> Str {
    let b = bytes(s);
    let n = char_count(b) as i64;
    if width <= n {
        return s;
    }
    let pad = (width - n) as usize;
    let (left, right) = match align {
        0 => (0, pad),
        1 => (pad, 0),
        _ => (pad / 2, pad - pad / 2),
    };
    let r = str_alloc(b.len() + pad);
    let d = data(r);
    memset(d, b' ' as i32, left);
    memcpy(d.add(left), b.as_ptr(), b.len());
    memset(d.add(left + b.len()), b' ' as i32, right);
    *r.add(1) = width as u64;
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_to_int(s: Str) -> i64 {
    let b = bytes(s);
    let mut i = 0;
    let mut h = b.len();
    while i < h && is_ws(b[i]) {
        i += 1;
    }
    while h > i && is_ws(b[h - 1]) {
        h -= 1;
    }
    let mut neg = false;
    if i < h && (b[i] == b'-' || b[i] == b'+') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    let mut ok = i < h;
    while i < h {
        let c = b[i];
        if c == b'_' && i > 0 {
            i += 1;
            continue;
        }
        if !c.is_ascii_digit() {
            ok = false;
            break;
        }
        v = match v.checked_mul(10).and_then(|x| if neg { x.checked_sub((c - b'0') as i64) } else { x.checked_add((c - b'0') as i64) }) {
            Some(x) => x,
            None => {
                ok = false;
                break;
            }
        };
        i += 1;
    }
    if !ok {
        let mut sb = crate::fmt::Sb::new();
        sb.push_bytes("мәтінді бүтін санға айналдыру мүмкін емес: '".as_bytes());
        sb.push_bytes(b);
        sb.push_bytes(b"'");
        fatal(sb.bytes());
    }
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_to_float(s: Str) -> f64 {
    let b = bytes(s);
    let mut i = 0;
    let mut h = b.len();
    while i < h && is_ws(b[i]) {
        i += 1;
    }
    while h > i && is_ws(b[h - 1]) {
        h -= 1;
    }
    if let Ok(t) = core::str::from_utf8(&b[i..h]) {
        if let Ok(v) = t.parse::<f64>() {
            return v;
        }
    }
    let mut sb = crate::fmt::Sb::new();
    sb.push_bytes("мәтінді нақты санға айналдыру мүмкін емес: '".as_bytes());
    sb.push_bytes(b);
    sb.push_bytes(b"'");
    fatal(sb.bytes())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_ord(s: Str) -> i64 {
    let b = bytes(s);
    if b.is_empty() {
        fatal("`код`: бос мәтін".as_bytes());
    }
    decode(b, 0).0 as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_chr(n: i64) -> Str {
    if !(0..=0x10FFFF).contains(&n) || (0xD800..=0xDFFF).contains(&n) {
        fatal("`таңба`: жарамсыз Unicode коды".as_bytes());
    }
    let mut tmp = [0u8; 4];
    let k = encode(n as u32, &mut tmp);
    from_bytes(&tmp[..k])
}

fn each_char(b: &[u8], mut f: impl FnMut(u32) -> bool) -> bool {
    let mut i = 0;
    while i < b.len() {
        let (cp, n) = decode(b, i);
        if !f(cp) {
            return false;
        }
        i += n;
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_isdigit(s: Str) -> i64 {
    let b = bytes(s);
    (!b.is_empty() && each_char(b, |c| (0x30..=0x39).contains(&c))) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_isalpha(s: Str) -> i64 {
    let b = bytes(s);
    (!b.is_empty() && each_char(b, |c| char::from_u32(c).map_or(false, |ch| ch.is_alphabetic()))) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_split_ws(s: Str) -> list::List {
    let b = bytes(s);
    let l = list::list_new(4, 0);
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && is_ws(b[i]) {
            i += 1;
        }
        let st = i;
        while i < b.len() && !is_ws(b[i]) {
            i += 1;
        }
        if i > st {
            list::list_push(l, from_bytes(&b[st..i]) as u64);
        }
    }
    l
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_str_count(s: Str, sub: Str) -> i64 {
    let (h, n) = (bytes(s), bytes(sub));
    if n.is_empty() {
        return char_count(h) as i64 + 1;
    }
    let mut c = 0;
    let mut from = 0;
    while let Some(i) = find_in(h, n, from) {
        c += 1;
        from = i + n.len();
    }
    c
}
