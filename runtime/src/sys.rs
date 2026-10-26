//! Жүйелік қабат: syscall-дар, mem* функциялары, шығару буфері, аварияның тоқтауы, сигналдар.

use core::arch::{asm, global_asm};

pub use crate::common::*;

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

/// Үйінді мен GC метадеректерінің өлшемдері (ядро рантаймында кішірек).
pub const GC_HEAP_MAX: usize = 1 << 36;
pub const GC_HEAP_MIN: usize = 1 << 26;
pub const GC_MARK_CAP: usize = 1 << 27;
pub const GC_FREE_RUN_CAP: usize = 1 << 20;

/// Үйінді үшін ОЖ бере алатын шамамен көлем (Linux-та шектеусіз: виртуал жад).
pub fn os_heap_hint() -> usize {
    usize::MAX
}

/// Беттерді ОЖ-ге қайтарады; қайта алғанда олар нөлдермен толы болады.
pub unsafe fn os_release(addr: usize, len: usize) {
    unsafe {
        sys3(28, addr, len, 4); // MADV_DONTNEED
    }
}

/// stdin-нен оқиды; <= 0 — файл соңы немесе қате.
pub unsafe fn stdin_read(buf: *mut u8, cap: usize) -> isize {
    unsafe { sys3(SYS_READ, 0, buf as usize, cap) }
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
            handler: on_signal as *const () as usize,
            flags: SA_SIGINFO | SA_ONSTACK | SA_RESTORER | SA_NODEFER,
            restorer: tlp_sigreturn as *const () as usize,
            mask: 0,
        };
        for sig in [11usize, 7, 8] {
            sys6(SYS_RT_SIGACTION, sig, &sa as *const _ as usize, 0, 8, 0, 0);
        }
    }
}
