//! Windows: аргументтер, файлдар, уақыт, кездейсоқ сандар (Linux нұсқасымен бірдей мінез-құлық).

use crate::fmt::Sb;
use crate::list;
use crate::str::*;
use crate::sys::*;
use crate::win;

// ------------------------------------------------------------------ UTF-8 <-> UTF-16

pub const WCAP: usize = 4096;

/// UTF-8 жолын нөлмен аяқталған UTF-16 буферіне аударады; тым ұзын болса `None`.
pub fn to_wide(s: &[u8], out: &mut [u16; WCAP]) -> Option<()> {
    let mut n = 0;
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        let (cp, len) = if b < 0x80 {
            (b as u32, 1)
        } else if b >= 0xF0 && i + 3 < s.len() {
            (((b as u32 & 7) << 18) | ((s[i + 1] as u32 & 63) << 12) | ((s[i + 2] as u32 & 63) << 6) | (s[i + 3] as u32 & 63), 4)
        } else if b >= 0xE0 && i + 2 < s.len() {
            (((b as u32 & 15) << 12) | ((s[i + 1] as u32 & 63) << 6) | (s[i + 2] as u32 & 63), 3)
        } else if b >= 0xC0 && i + 1 < s.len() {
            (((b as u32 & 31) << 6) | (s[i + 1] as u32 & 63), 2)
        } else {
            (0xFFFD, 1)
        };
        i += len;
        if n + 2 >= WCAP {
            return None;
        }
        if cp >= 0x10000 {
            let c = cp - 0x10000;
            out[n] = 0xD800 + (c >> 10) as u16;
            out[n + 1] = 0xDC00 + (c & 0x3FF) as u16;
            n += 2;
        } else {
            out[n] = cp as u16;
            n += 1;
        }
    }
    out[n] = 0;
    Some(())
}

unsafe fn wpath(s: Str, out: &mut [u16; WCAP]) {
    if to_wide(bytes(s), out).is_none() {
        fatal("файл жолы тым ұзын".as_bytes());
    }
}

fn push_utf8(sb: &mut Sb, cp: u32) {
    let mut b = [0u8; 4];
    let n = if cp < 0x80 {
        b[0] = cp as u8;
        1
    } else if cp < 0x800 {
        b[0] = 0xC0 | (cp >> 6) as u8;
        b[1] = 0x80 | (cp & 63) as u8;
        2
    } else if cp < 0x10000 {
        b[0] = 0xE0 | (cp >> 12) as u8;
        b[1] = 0x80 | ((cp >> 6) & 63) as u8;
        b[2] = 0x80 | (cp & 63) as u8;
        3
    } else {
        b[0] = 0xF0 | (cp >> 18) as u8;
        b[1] = 0x80 | ((cp >> 12) & 63) as u8;
        b[2] = 0x80 | ((cp >> 6) & 63) as u8;
        b[3] = 0x80 | (cp & 63) as u8;
        4
    };
    sb.push_bytes(&b[..n]);
}

// ------------------------------------------------------------------ аргументтер

/// Командалық жол (`GetCommandLineW`) -> аргументтер (MSVC ережелері: тырнақша мен `\"`), 0-шісі өткізіледі.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_args() -> list::List {
    unsafe {
        let l = list::list_new(4, 0);
        let mut p = win::get_command_line();
        let mut index = 0;
        loop {
            while *p == b' ' as u16 || *p == b'\t' as u16 {
                p = p.add(1);
            }
            if *p == 0 {
                break;
            }
            let mut sb = Sb::new();
            let mut quoted = false;
            loop {
                let c = *p;
                if c == 0 || (!quoted && (c == b' ' as u16 || c == b'\t' as u16)) {
                    break;
                }
                if c == b'\\' as u16 {
                    let mut n = 0;
                    while *p == b'\\' as u16 {
                        n += 1;
                        p = p.add(1);
                    }
                    if *p == b'"' as u16 {
                        for _ in 0..n / 2 {
                            sb.push(b'\\');
                        }
                        if n % 2 == 1 {
                            sb.push(b'"');
                            p = p.add(1);
                        }
                    } else {
                        for _ in 0..n {
                            sb.push(b'\\');
                        }
                    }
                    continue;
                }
                if c == b'"' as u16 {
                    quoted = !quoted;
                    p = p.add(1);
                    continue;
                }
                let mut cp = c as u32;
                p = p.add(1);
                if (0xD800..0xDC00).contains(&cp) && (0xDC00..0xE000).contains(&(*p as u32)) {
                    cp = 0x10000 + ((cp - 0xD800) << 10) + (*p as u32 - 0xDC00);
                    p = p.add(1);
                }
                push_utf8(&mut sb, cp);
            }
            if index > 0 {
                let s = from_bytes(sb.bytes());
                list::list_push(l, s as u64);
            }
            sb.discard();
            index += 1;
        }
        l
    }
}

// ------------------------------------------------------------------ файлдар

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_APPEND_DATA: u32 = 4;
const OPEN_EXISTING: u32 = 3;
const CREATE_ALWAYS: u32 = 2;
const OPEN_ALWAYS: u32 = 4;
const INVALID: usize = usize::MAX;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_read(path: Str) -> Str {
    unsafe {
        let mut w = [0u16; WCAP];
        wpath(path, &mut w);
        let h = win::create_file(w.as_ptr(), GENERIC_READ, 1, OPEN_EXISTING);
        if h == INVALID {
            let mut m = Sb::new();
            m.push_bytes("файлды ашу мүмкін емес: ".as_bytes());
            m.push_bytes(bytes(path));
            fatal(m.bytes());
        }
        let mut sb = Sb::new();
        let mut buf = [0u8; 65536];
        loop {
            let mut n = 0u32;
            if win::read_file(h, buf.as_mut_ptr(), buf.len() as u32, &mut n) == 0 || n == 0 {
                break;
            }
            sb.push_bytes(&buf[..n as usize]);
        }
        win::close_handle(h);
        sb.finish()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_write(path: Str, text: Str, append: i64) {
    unsafe {
        let mut w = [0u16; WCAP];
        wpath(path, &mut w);
        let (access, disp) = if append != 0 { (FILE_APPEND_DATA, OPEN_ALWAYS) } else { (GENERIC_WRITE, CREATE_ALWAYS) };
        let h = win::create_file(w.as_ptr(), access, 1, disp);
        if h == INVALID {
            let mut m = Sb::new();
            m.push_bytes("файлға жазу мүмкін емес: ".as_bytes());
            m.push_bytes(bytes(path));
            fatal(m.bytes());
        }
        let b = bytes(text);
        let mut off = 0;
        while off < b.len() {
            let mut n = 0u32;
            let chunk = (b.len() - off).min(1 << 20) as u32;
            if win::write_file(h, b.as_ptr().add(off), chunk, &mut n) == 0 || n == 0 {
                break;
            }
            off += n as usize;
        }
        win::close_handle(h);
    }
}

/// Файл бар ма.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_exists(path: Str) -> i64 {
    unsafe {
        let mut w = [0u16; WCAP];
        wpath(path, &mut w);
        (win::file_attributes(w.as_ptr()) != 0xFFFF_FFFF) as i64
    }
}

/// Файлды жояды; жоқ болса — үнсіз.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_delete(path: Str) {
    unsafe {
        let mut w = [0u16; WCAP];
        wpath(path, &mut w);
        win::delete_file(w.as_ptr());
    }
}

/// Файлды қайта атайды (бар болса — ауыстырады); сәтсіз болса — қате.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_rename(from: Str, to: Str) {
    unsafe {
        let mut a = [0u16; WCAP];
        let mut b = [0u16; WCAP];
        wpath(from, &mut a);
        wpath(to, &mut b);
        if !win::move_file(a.as_ptr(), b.as_ptr()) {
            let mut m = Sb::new();
            m.push_bytes("файлды ауыстыру мүмкін емес: ".as_bytes());
            m.push_bytes(bytes(from));
            fatal(m.bytes());
        }
    }
}

/// Қалта жасайды; бар болса — үнсіз.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_make_dir(path: Str) {
    unsafe {
        let mut w = [0u16; WCAP];
        wpath(path, &mut w);
        win::create_directory(w.as_ptr());
    }
}

// ------------------------------------------------------------------ уақыт, кездейсоқ сандар

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_time() -> f64 {
    unsafe {
        // FILETIME: 1601-01-01-ден бері 100 нс; Unix эпохасына ауыстырамыз
        let t = win::system_time_100ns();
        (t.saturating_sub(116_444_736_000_000_000)) as f64 * 1e-7
    }
}

static mut RNG: u64 = 0;

fn next_u64() -> u64 {
    unsafe {
        if RNG == 0 {
            RNG = win::system_time_100ns().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ win::perf_counter() | 1;
        }
        RNG = RNG.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = RNG;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_seed(n: i64) {
    unsafe {
        RNG = (n as u64) ^ 0x1234_5678_9ABC_DEF1;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_rand() -> f64 {
    (next_u64() >> 11) as f64 / 9007199254740992.0
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_rand_range(a: i64, b: i64) -> i64 {
    if b < a {
        fatal("`кездейсоқ_аралық`: төменгі шек жоғарғыдан үлкен".as_bytes());
    }
    let span = (b as i128 - a as i128 + 1) as u128;
    a.wrapping_add((next_u64() as u128 % span) as i64)
}
