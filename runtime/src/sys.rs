//! Жүйелік қабат: syscall-дар, mem* функциялары, шығару буфері, аварияның тоқтауы, сигналдар.

use core::arch::{asm, global_asm};

pub const SYS_READ: usize = 0;
pub const SYS_WRITE: usize = 1;
pub const SYS_MMAP: usize = 9;
pub const SYS_MPROTECT: usize = 10;
pub const SYS_MUNMAP: usize = 11;
pub const SYS_RT_SIGACTION: usize = 13;
pub const SYS_SIGALTSTACK: usize = 131;
pub const SYS_EXIT_GROUP: usize = 231;

#[inline(always)]
pub unsafe fn sys3(n: usize, a: usize, b: usize, c: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a, in("rsi") b, in("rdx") c,
             lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

#[inline(always)]
pub unsafe fn sys6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a, in("rsi") b, in("rdx") c,
             in("r10") d, in("r8") e, in("r9") f, lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// Жадты ОЖ-дан сұрайды (нөлдермен толтырылған). Сәтсіз болса 0.
pub unsafe fn os_map(size: usize, noreserve: bool) -> usize {
    const PROT_RW: usize = 3;
    const MAP_PRIVATE: usize = 0x2;
    const MAP_ANON: usize = 0x20;
    const MAP_NORESERVE: usize = 0x4000;
    let flags = MAP_PRIVATE | MAP_ANON | if noreserve { MAP_NORESERVE } else { 0 };
    let r = unsafe { sys6(SYS_MMAP, 0, size, PROT_RW, flags, usize::MAX, 0) };
    if r < 0 && r > -4096 {
        0
    } else {
        r as usize
    }
}

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

// ------------------------------------------------------------------ шығару

const OUT_CAP: usize = 1 << 16;
static mut OUT_BUF: [u8; OUT_CAP] = [0; OUT_CAP];
static mut OUT_LEN: usize = 0;

pub fn write_all(fd: usize, mut p: *const u8, mut n: usize) {
    while n > 0 {
        let r = unsafe { sys3(SYS_WRITE, fd, p as usize, n) };
        if r <= 0 {
            if r == -4 {
                continue; // EINTR
            }
            return;
        }
        p = unsafe { p.add(r as usize) };
        n -= r as usize;
    }
}

pub fn out_flush() {
    unsafe {
        if OUT_LEN > 0 {
            let n = OUT_LEN;
            OUT_LEN = 0;
            write_all(1, core::ptr::addr_of!(OUT_BUF) as *const u8, n);
        }
    }
}

pub fn out_write(bytes: &[u8]) {
    unsafe {
        if bytes.len() > OUT_CAP - OUT_LEN {
            out_flush();
            if bytes.len() >= OUT_CAP {
                write_all(1, bytes.as_ptr(), bytes.len());
                return;
            }
        }
        let dst = (core::ptr::addr_of_mut!(OUT_BUF) as *mut u8).add(OUT_LEN);
        memcpy(dst, bytes.as_ptr(), bytes.len());
        OUT_LEN += bytes.len();
    }
}

pub fn err_write(bytes: &[u8]) {
    write_all(2, bytes.as_ptr(), bytes.len());
}

pub fn exit(code: i32) -> ! {
    out_flush();
    unsafe {
        sys3(SYS_EXIT_GROUP, code as usize, 0, 0);
    }
    loop {}
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

pub fn fatal_at(msg: &[u8], rip: usize, rbp: usize) -> ! {
    out_flush();
    unsafe {
        print_trace(rip, rbp);
    }
    err_write("Қате: ".as_bytes());
    err_write(msg);
    err_write(b"\n");
    exit(1)
}

/// Қате туралы хабарлама stderr-ге (трассамен), процесс 1 кодымен аяқталады.
#[inline(never)]
pub fn fatal(msg: &[u8]) -> ! {
    let rbp: usize;
    unsafe {
        asm!("mov {}, rbp", out(reg) rbp, options(nomem, nostack, preserves_flags));
    }
    fatal_at(msg, 0, rbp)
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

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fatal("ішкі рантайм қатесі".as_bytes())
}

// ------------------------------------------------------------------ сигналдар

pub static mut STACK_GUARD_LO: usize = 0;
pub static mut STACK_GUARD_HI: usize = 0;

global_asm!(
    ".globl tlp_sigreturn",
    "tlp_sigreturn:",
    "mov rax, 15",
    "syscall",
);

unsafe extern "C" {
    fn tlp_sigreturn();
}

#[repr(C)]
struct KSigaction {
    handler: usize,
    flags: u64,
    restorer: usize,
    mask: u64,
}

#[repr(C)]
struct StackT {
    sp: usize,
    flags: i32,
    size: usize,
}

unsafe extern "C" fn on_signal(sig: i32, info: *const u8, ctx: *const u8) {
    let addr = unsafe { *(info.add(16) as *const usize) };
    unsafe {
        let rbp = *(ctx.add(120) as *const usize);
        let rip = *(ctx.add(168) as *const usize);
        if sig == 11 || sig == 7 {
            if addr >= STACK_GUARD_LO && addr < STACK_GUARD_HI {
                fatal_at("стек толып кетті (рекурсия тым терең)".as_bytes(), rip, rbp);
            }
            if addr < 4096 * 16 {
                fatal_at("нөлдік көрсеткішке (жоқ) қатынау".as_bytes(), rip, rbp);
            }
            fatal_at("жадқа жарамсыз қатынау".as_bytes(), rip, rbp);
        } else if sig == 8 {
            fatal_at("нөлге бөлу немесе бүтін санның асып кетуі".as_bytes(), rip, rbp);
        }
    }
    fatal("күтпеген сигнал".as_bytes());
}

static mut ALT_STACK: [u8; 65536] = [0; 65536];

pub fn install_signals() {
    unsafe {
        let st = StackT { sp: core::ptr::addr_of_mut!(ALT_STACK) as usize, flags: 0, size: 65536 };
        sys3(SYS_SIGALTSTACK, &st as *const _ as usize, 0, 0);
        const SA_SIGINFO: u64 = 4;
        const SA_ONSTACK: u64 = 0x0800_0000;
        const SA_RESTORER: u64 = 0x0400_0000;
        const SA_NODEFER: u64 = 0x4000_0000;
        let sa = KSigaction {
            handler: on_signal as usize,
            flags: SA_SIGINFO | SA_ONSTACK | SA_RESTORER | SA_NODEFER,
            restorer: tlp_sigreturn as usize,
            mask: 0,
        };
        for sig in [11usize, 7, 8] {
            sys6(SYS_RT_SIGACTION, sig, &sa as *const _ as usize, 0, 8, 0, 0);
        }
    }
}
