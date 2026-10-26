//! Платформаға тәуелсіз бөліктер: mem*, модуль ақпараты (трасса), сандарды форматтау.
//! Linux рантаймы (`sys.rs`) да, ядро рантаймы (`kernel/`) да қолданады.

use crate::sys::{err_write, STACK_GUARD_LO};
use core::arch::asm;

// ------------------------------------------------------------------ mem*

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    unsafe {
        asm!("rep movsb", inout("rcx") n => _, inout("rdi") d => _, inout("rsi") s => _, options(nostack, preserves_flags));
    }
    d
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    if (d as usize) <= (s as usize) || (d as usize) >= (s as usize) + n {
        unsafe { memcpy(d, s, n) }
    } else {
        unsafe {
            asm!("std", "rep movsb", "cld", inout("rcx") n => _, inout("rdi") d.add(n).wrapping_sub(1) => _,
                 inout("rsi") s.add(n).wrapping_sub(1) => _, options(nostack));
        }
        d
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(d: *mut u8, c: i32, n: usize) -> *mut u8 {
    unsafe {
        asm!("rep stosb", inout("rcx") n => _, inout("rdi") d => _, in("al") c as u8, options(nostack, preserves_flags));
    }
    d
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let (x, y) = unsafe { (core::ptr::read_volatile(a.add(i)), core::ptr::read_volatile(b.add(i))) };
        if x != y {
            return x as i32 - y as i32;
        }
        i += 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    unsafe { memcmp(a, b, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut n = 0;
    while core::ptr::read_volatile(s.add(n)) != 0 {
        n += 1;
    }
    n
}


/// Модуль ақпараты (`tlp_info`): linetab, linetab_end, functab, functab_end, filetab, text_end
pub static mut INFO: *const usize = core::ptr::null();

unsafe fn cstr<'a>(p: *const u8) -> &'a [u8] {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    core::slice::from_raw_parts(p, n)
}

/// pc бойынша (файл, жол, функция аты); тек Tulpar кодында.
unsafe fn lookup(pc: usize) -> Option<(&'static [u8], u32, &'static [u8])> {
    if INFO.is_null() {
        return None;
    }
    let (lt, lte, ft, fte, filetab, text_end) = (*INFO, *INFO.add(1), *INFO.add(2), *INFO.add(3), *INFO.add(4), *INFO.add(5));
    if lt == lte || pc >= text_end || pc < *(lt as *const usize) {
        return None;
    }
    // линиялар: 16 байт [addr u64][file u32][line u32]
    let (mut lo, mut hi) = (0usize, (lte - lt) / 16);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if *((lt + mid * 16) as *const usize) <= pc {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let e = (lt + lo * 16) as *const u32;
    let (file, line) = (*e.add(2) as usize, *e.add(3));
    let (mut flo, mut fhi) = (0usize, (fte - ft) / 16);
    while fhi - flo > 1 {
        let mid = (flo + fhi) / 2;
        if *((ft + mid * 16) as *const usize) <= pc {
            flo = mid;
        } else {
            fhi = mid;
        }
    }
    let name = cstr(*((ft + flo * 16 + 8) as *const *const u8));
    let fname = cstr(*((filetab + file * 8) as *const *const u8));
    Some((fname, line, name))
}

/// rip (0 болса — жоқ) және rbp тізбегі бойынша трассаны басады.
pub unsafe fn print_trace(rip: usize, mut rbp: usize) {
    let mut frames: [(usize, bool); 48] = [(0, false); 48];
    let mut n = 0;
    if rip != 0 {
        frames[n] = (rip, true);
        n += 1;
    }
    let top = crate::gc::STACK_TOP;
    let mut steps = 0;
    while n < 48 && steps < 200_000_000 && rbp != 0 && rbp % 8 == 0 && rbp + 16 <= top && rbp >= STACK_GUARD_LO {
        let ret = *((rbp + 8) as *const usize);
        let next = *(rbp as *const usize);
        if lookup(ret.wrapping_sub(1)).is_some() {
            frames[n] = (ret, false);
            n += 1;
        }
        if next <= rbp {
            break;
        }
        rbp = next;
        steps += 1;
    }
    if n == 0 {
        return;
    }
    err_write("Шақырулар тізбегі (соңғысы төменде):\n".as_bytes());
    let mut i = n;
    while i > 0 {
        i -= 1;
        let (pc, exact) = frames[i];
        if let Some((file, line, name)) = lookup(if exact { pc } else { pc - 1 }) {
            let mut b = [0u8; 24];
            err_write(b"  \"");
            err_write(file);
            err_write("\", ".as_bytes());
            err_write(fmt_u64(line as u64, &mut b));
            err_write("-жол, ".as_bytes());
            err_write(name);
            err_write(b"\n");
        }
    }
}


pub fn fmt_u64(mut v: u64, buf: &mut [u8; 24]) -> &[u8] {
    let mut i = 24;
    if v == 0 {
        i -= 1;
        buf[i] = b'0';
    }
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    &buf[i..]
}

pub fn fmt_i64(v: i64, buf: &mut [u8; 24]) -> &[u8] {
    if v >= 0 {
        return fmt_u64(v as u64, buf);
    }
    let mut i = 24;
    let mut u = (v as i128).unsigned_abs() as u64;
    while u > 0 {
        i -= 1;
        buf[i] = b'0' + (u % 10) as u8;
        u /= 10;
    }
    i -= 1;
    buf[i] = b'-';
    &buf[i..]
}

