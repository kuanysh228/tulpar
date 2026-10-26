//! Ядро рантаймы: аргументтер, уақыт (TSC), кездейсоқ сандар; файл және желі функциялары қолжетімсіз.

use crate::arch::{inb, outb, rdtsc};
use crate::list;
use crate::str::Str;
use crate::sys::*;

static mut TSC_HZ: u64 = 2_000_000_000;
static mut TSC_START: u64 = 0;

/// TSC жиілігін PIT 2-арнасы бойынша өлшейді (~10 мс).
pub unsafe fn calibrate_clock() {
    const COUNTS: u32 = 11932; // 1193182 Гц * 0.01 с
    let gate = inb(0x61);
    outb(0x61, (gate & 0xFC) | 1);
    outb(0x43, 0xB0);
    outb(0x42, COUNTS as u8);
    outb(0x42, (COUNTS >> 8) as u8);
    let t0 = rdtsc();
    let mut spins = 0u32;
    while inb(0x61) & 0x20 == 0 && spins < 50_000_000 {
        spins += 1;
    }
    let t1 = rdtsc();
    outb(0x61, gate);
    if spins < 50_000_000 && t1 > t0 {
        TSC_HZ = ((t1 - t0) as u128 * 1_193_182 / COUNTS as u128) as u64;
    }
    TSC_START = rdtsc();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_time() -> f64 {
    (rdtsc() - TSC_START) as f64 / TSC_HZ as f64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_args() -> list::List {
    list::list_new(4, 0)
}

static mut RNG: u64 = 0;

fn next_u64() -> u64 {
    unsafe {
        if RNG == 0 {
            RNG = rdtsc().wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
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

// ------------------------------------------------------------------ ядрода қолжетімсіз функциялар

const NO_FS: &str = "файл жүйесі ядро режимінде қолжетімсіз";
const NO_NET: &str = "желі ядро режимінде қолжетімсіз (өз драйвер жазыңыз)";

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_read(_p: Str) -> Str {
    fatal(NO_FS.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_write(_p: Str, _t: Str, _a: i64) {
    fatal(NO_FS.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_exists(_p: Str) -> i64 {
    fatal(NO_FS.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_delete(_p: Str) {
    fatal(NO_FS.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_file_rename(_a: Str, _b: Str) {
    fatal(NO_FS.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_make_dir(_p: Str) {
    fatal(NO_FS.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_listen(_p: i64) -> i64 {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_listen_local(_p: i64) -> i64 {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_accept(_s: i64) -> i64 {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_connect(_h: Str, _p: i64) -> i64 {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_read(_s: i64) -> Str {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_write(_s: i64, _t: Str) {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_close(_s: i64) {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_timeout(_s: i64, _ms: i64) {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_http_get(_u: Str) -> Str {
    fatal(NO_NET.as_bytes())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_http_request(_u: Str, _m: Str, _b: Str, _h: *mut u64) -> *mut u64 {
    fatal(NO_NET.as_bytes())
}
