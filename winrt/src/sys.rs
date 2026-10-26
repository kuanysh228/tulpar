//! Windows жүйелік қабаты: kernel32 арқылы жад, консоль, енгізу, ерекше жағдайлар.
//! Linux нұсқасының (`runtime/src/sys.rs`) орнын басады; ортақ модульдер сол атаулармен қолданады.

use crate::win;
use core::arch::asm;

pub use crate::common::*;

pub const GC_HEAP_MAX: usize = 1 << 31;
pub const GC_HEAP_MIN: usize = 1 << 26;
pub const GC_MARK_CAP: usize = 1 << 22;
pub const GC_FREE_RUN_CAP: usize = 1 << 18;

pub static mut STACK_GUARD_LO: usize = 0;
pub static mut STACK_GUARD_HI: usize = 0;

// ------------------------------------------------------------------ жад

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_DECOMMIT: u32 = 0x4000;
const PAGE_READWRITE: u32 = 4;

/// Жадты ОЖ-дан сұрайды (нөлдермен толтырылған, бірден қолдануға дайын). Сәтсіз болса 0.
pub unsafe fn os_map(size: usize, _noreserve: bool) -> usize {
    unsafe { win::virtual_alloc(0, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) }
}

/// Үйіндінің адрестік аймағын резервтейді; бөліктері `os_commit` арқылы іске қосылады.
pub unsafe fn os_reserve(size: usize) -> usize {
    unsafe { win::virtual_alloc(0, size, MEM_RESERVE, PAGE_READWRITE) }
}

pub unsafe fn os_commit(addr: usize, len: usize) -> bool {
    unsafe { win::virtual_alloc(addr, len, MEM_COMMIT, PAGE_READWRITE) != 0 }
}

pub fn os_heap_hint() -> usize {
    usize::MAX
}

/// Беттерді ОЖ-ге қайтарады; қайта іске қосқанда олар нөлдермен толы болады.
pub unsafe fn os_release(addr: usize, len: usize) {
    unsafe {
        win::virtual_free(addr, len, MEM_DECOMMIT);
    }
}

// ------------------------------------------------------------------ шығару

const OUT_CAP: usize = 1 << 16;
static mut OUT_BUF: [u8; OUT_CAP] = [0; OUT_CAP];
static mut OUT_LEN: usize = 0;
static mut H_OUT: usize = 0;
static mut H_ERR: usize = 0;
static mut H_IN: usize = 0;

pub unsafe fn init_console() {
    unsafe {
        H_IN = win::get_std_handle(-10i32 as u32);
        H_OUT = win::get_std_handle(-11i32 as u32);
        H_ERR = win::get_std_handle(-12i32 as u32);
        win::set_console_cp(65001); // UTF-8
    }
}

pub fn write_all(fd: usize, mut p: *const u8, mut n: usize) {
    unsafe {
        let h = if fd == 2 { H_ERR } else { H_OUT };
        while n > 0 {
            let chunk = n.min(1 << 20) as u32;
            let mut w = 0u32;
            if win::write_file(h, p, chunk, &mut w) == 0 || w == 0 {
                return;
            }
            p = p.add(w as usize);
            n -= w as usize;
        }
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
    unsafe { win::exit_process(code as u32) }
}

/// stdin-нен оқиды (консольде жол-жолымен); `\r` алынып тасталады. <= 0 — файл соңы немесе қате.
pub unsafe fn stdin_read(buf: *mut u8, cap: usize) -> isize {
    unsafe {
        loop {
            let mut n = 0u32;
            if win::read_file(H_IN, buf, cap.min(1 << 20) as u32, &mut n) == 0 || n == 0 {
                return 0;
            }
            let mut j = 0;
            for i in 0..n as usize {
                let c = *buf.add(i);
                if c != b'\r' {
                    *buf.add(j) = c;
                    j += 1;
                }
            }
            if j > 0 {
                return j as isize;
            }
        }
    }
}

// ------------------------------------------------------------------ қателер

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

/// Ерекше жағдайлар (SEH, vectored): нөлге бөлу, жадқа қатынау, стек толуы -> қазақша хабарлама мен трасса.
unsafe extern "win64" fn on_exception(ep: *const u8) -> i32 {
    unsafe {
        let rec = *(ep as *const *const u8);
        let ctx = *(ep.add(8) as *const *const u8);
        let code = *(rec as *const u32);
        let rip = *(ctx.add(0xF8) as *const usize);
        let rbp = *(ctx.add(0xA0) as *const usize);
        match code {
            0xC000_0005 => {
                let addr = *(rec.add(40) as *const usize);
                if addr < 4096 * 16 {
                    fatal_at("нөлдік көрсеткішке (жоқ) қатынау".as_bytes(), rip, rbp);
                }
                fatal_at("жадқа жарамсыз қатынау".as_bytes(), rip, rbp);
            }
            0xC000_00FD => fatal_at("стек толып кетті (рекурсия тым терең)".as_bytes(), rip, rbp),
            0xC000_0094 | 0xC000_0095 => {
                fatal_at("нөлге бөлу немесе бүтін санның асып кетуі".as_bytes(), rip, rbp)
            }
            0xC000_001D | 0xC000_0096 => fatal_at("жарамсыз команда".as_bytes(), rip, rbp),
            _ => 0, // EXCEPTION_CONTINUE_SEARCH
        }
    }
}

pub unsafe fn install_handlers() {
    unsafe {
        win::add_exception_handler(on_exception);
    }
}
