//! Қосымша: командалық жол аргументтері, файлдар, уақыт, кездейсоқ сандар.

use crate::fmt::Sb;
use crate::list;
use crate::str::*;
use crate::sys::*;

pub static mut ARGV_SP: usize = 0;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_args() -> list::List {
    let l = list::list_new(4, 0);
    let sp = ARGV_SP as *const usize;
    let argc = *sp;
    let argv = sp.add(1);
    // 0-ші аргумент — бағдарламаның аты, оны өткізіп жібереміз
    for i in 1..argc {
        let p = *argv.add(i) as *const u8;
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        let s = from_bytes(core::slice::from_raw_parts(p, n));
        list::list_push(l, s as u64);
    }
    l
}

unsafe fn cpath(s: Str) -> Sb {
    let mut sb = Sb::new();
    sb.push_bytes(bytes(s));
    sb.push(0);
    sb
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_read(path: Str) -> Str {
    let p = cpath(path);
    let fd = sys3(2, p.bytes().as_ptr() as usize, 0, 0); // open O_RDONLY
    if fd < 0 {
        let mut m = Sb::new();
        m.push_bytes("файлды ашу мүмкін емес: ".as_bytes());
        m.push_bytes(bytes(path));
        p.discard();
        fatal(m.bytes());
    }
    p.discard();
    let mut sb = Sb::new();
    let mut buf = [0u8; 65536];
    loop {
        let r = sys3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len());
        if r == -4 {
            continue;
        }
        if r <= 0 {
            break;
        }
        sb.push_bytes(&buf[..r as usize]);
    }
    sys3(3, fd as usize, 0, 0);
    sb.finish()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_write(path: Str, text: Str, append: i64) {
    let p = cpath(path);
    // O_WRONLY|O_CREAT|(O_APPEND|O_TRUNC)
    let flags = 1 | 0o100 | if append != 0 { 0o2000 } else { 0o1000 };
    let fd = sys3(2, p.bytes().as_ptr() as usize, flags, 0o644);
    if fd < 0 {
        let mut m = Sb::new();
        m.push_bytes("файлға жазу мүмкін емес: ".as_bytes());
        m.push_bytes(bytes(path));
        p.discard();
        fatal(m.bytes());
    }
    p.discard();
    let b = bytes(text);
    write_all(fd as usize, b.as_ptr(), b.len());
    sys3(3, fd as usize, 0, 0);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_time() -> f64 {
    let mut ts = [0i64; 2];
    sys3(228, 0, ts.as_mut_ptr() as usize, 0);
    ts[0] as f64 + ts[1] as f64 * 1e-9
}

static mut RNG: u64 = 0;

fn next_u64() -> u64 {
    unsafe {
        if RNG == 0 {
            let mut ts = [0i64; 2];
            sys3(228, 1, ts.as_mut_ptr() as usize, 0);
            RNG = (ts[0] as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (ts[1] as u64) | 1;
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
