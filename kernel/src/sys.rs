//! Ядро рантаймының жүйелік қабаты: консоль (COM1 + VGA мәтін), жад, енгізу, аварияның тоқтауы.
//! Linux нұсқасының (`runtime/src/sys.rs`) орнын басады; ортақ модульдер сол атаулармен қолданады.

use crate::arch::{inb, outb};
use core::arch::asm;

pub use crate::common::*;

pub const GC_HEAP_MAX: usize = 1 << 36;
pub const GC_HEAP_MIN: usize = 1 << 20;
pub const GC_MARK_CAP: usize = 1 << 20;
pub const GC_FREE_RUN_CAP: usize = 1 << 14;

pub static mut STACK_GUARD_LO: usize = 0;
pub static mut STACK_GUARD_HI: usize = 0;

// ------------------------------------------------------------------ жад

static mut MEM_CUR: usize = 0;
static mut MEM_END: usize = 0;

/// Ядродан кейінгі бос RAM аймағын `os_map` үшін белгілейді (физикалық = виртуал, бірлікті бейнелеу).
pub unsafe fn mem_init(lo: usize, hi: usize) {
    MEM_CUR = lo;
    MEM_END = hi;
}

pub fn mem_free_bytes() -> usize {
    unsafe { MEM_END - MEM_CUR }
}

/// Жадты бөлшектемей-ақ тізбектей береді (қайтарылмайды); жаңа RAM нөлдермен толы деп есептеледі.
pub unsafe fn os_map(size: usize, _noreserve: bool) -> usize {
    let size = (size + 4095) & !4095;
    match MEM_CUR.checked_add(size) {
        Some(e) if e <= MEM_END => {
            let p = MEM_CUR;
            MEM_CUR = e;
            p
        }
        _ => 0,
    }
}

/// GC үйіндісі мен оның метадеректері (бет кестесі, span-дар, белгі стегі) бірге сыйатындай үйінді көлемі.
pub fn os_heap_hint() -> usize {
    let avail = unsafe { MEM_END - MEM_CUR };
    let fixed = GC_MARK_CAP * 8 + GC_FREE_RUN_CAP * 16 + (1 << 16);
    let pages = avail.saturating_sub(fixed) / (4096 + 4 + 256);
    pages * 4096
}

pub unsafe fn os_release(addr: usize, len: usize) {
    memset(addr as *mut u8, 0, len);
}

// ------------------------------------------------------------------ консоль

const COM1: u16 = 0x3F8;
const VGA: *mut u16 = 0xB8000 as *mut u16;
const COLS: usize = 80;
const ROWS: usize = 25;
const ATTR: u16 = 0x0700;

static mut VX: usize = 0;
static mut VY: usize = 0;
static mut UTF_SKIP: u32 = 0;

pub unsafe fn console_init() {
    outb(COM1 + 1, 0x00); // үзілістер өшірулі
    outb(COM1 + 3, 0x80); // DLAB
    outb(COM1, 0x01); // 115200 бод
    outb(COM1 + 1, 0x00);
    outb(COM1 + 3, 0x03); // 8N1
    // FIFO қосылмайды: ол қосылғанда UART алдын ала келген байтты тастайды (құбырмен берілген енгізу үшін маңызды)
    outb(COM1 + 4, 0x03); // DTR | RTS
    for i in 0..COLS * ROWS {
        core::ptr::write_volatile(VGA.add(i), ATTR | b' ' as u16);
    }
    VX = 0;
    VY = 0;
    vga_cursor();
}

fn serial_put(c: u8) {
    unsafe {
        let mut spins = 0u32;
        while inb(COM1 + 5) & 0x20 == 0 && spins < 100_000 {
            spins += 1;
        }
        outb(COM1, c);
    }
}

unsafe fn vga_cursor() {
    let pos = (VY * COLS + VX) as u16;
    outb(0x3D4, 0x0F);
    outb(0x3D5, pos as u8);
    outb(0x3D4, 0x0E);
    outb(0x3D5, (pos >> 8) as u8);
}

unsafe fn vga_scroll() {
    for i in 0..COLS * (ROWS - 1) {
        let v = core::ptr::read_volatile(VGA.add(i + COLS));
        core::ptr::write_volatile(VGA.add(i), v);
    }
    for i in COLS * (ROWS - 1)..COLS * ROWS {
        core::ptr::write_volatile(VGA.add(i), ATTR | b' ' as u16);
    }
    VY = ROWS - 1;
}

unsafe fn vga_put(c: u8) {
    // UTF-8: VGA мәтін режимі тек CP437 көрсетеді, сондықтан ASCII емес таңба `?` болып көрінеді
    if UTF_SKIP > 0 && c & 0xC0 == 0x80 {
        UTF_SKIP -= 1;
        return;
    }
    UTF_SKIP = 0;
    let c = if c >= 0xC0 {
        UTF_SKIP = if c >= 0xF0 { 3 } else if c >= 0xE0 { 2 } else { 1 };
        b'?'
    } else if c >= 0x80 {
        return;
    } else {
        c
    };
    match c {
        b'\n' => {
            VX = 0;
            VY += 1;
        }
        b'\r' => VX = 0,
        8 => {
            if VX > 0 {
                VX -= 1;
            }
        }
        b'\t' => VX = (VX + 8) & !7,
        c if c >= 0x20 => {
            core::ptr::write_volatile(VGA.add(VY * COLS + VX), ATTR | c as u16);
            VX += 1;
        }
        _ => {}
    }
    if VX >= COLS {
        VX = 0;
        VY += 1;
    }
    if VY >= ROWS {
        vga_scroll();
    }
}

/// Консольға (COM1 және VGA-ға) жазады.
pub fn console_write(bytes: &[u8]) {
    unsafe {
        for &c in bytes {
            if c == b'\n' {
                serial_put(b'\r');
            }
            serial_put(c);
            vga_put(c);
        }
        vga_cursor();
    }
}

pub fn write_all(_fd: usize, p: *const u8, n: usize) {
    console_write(unsafe { core::slice::from_raw_parts(p, n) });
}

pub fn out_flush() {}

pub fn out_write(bytes: &[u8]) {
    console_write(bytes);
}

pub fn err_write(bytes: &[u8]) {
    console_write(bytes);
}

// ------------------------------------------------------------------ енгізу

const SCANCODES: &[u8; 58] = b"\0\x1b1234567890-=\x08\tqwertyuiop[]\n\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
const SCANCODES_SHIFT: &[u8; 58] = b"\0\x1b!@#$%^&*()_+\x08\tQWERTYUIOP{}\n\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";

static mut SHIFT: bool = false;
static mut CTRL: bool = false;

/// Блоктаусыз: COM1-ден немесе PS/2 пернетақтасынан келген келесі таңба (ASCII), болмаса `None`.
pub fn poll_char() -> Option<u8> {
    unsafe {
        if inb(COM1 + 5) & 1 != 0 {
            return Some(inb(COM1));
        }
        while inb(0x64) & 1 != 0 {
            let status = inb(0x64);
            let sc = inb(0x60);
            if status & 0x20 != 0 {
                continue; // тінтуір деректері
            }
            match sc {
                0x2A | 0x36 => SHIFT = true,
                0xAA | 0xB6 => SHIFT = false,
                0x1D => CTRL = true,
                0x9D => CTRL = false,
                s if s & 0x80 != 0 || s as usize >= SCANCODES.len() => {}
                s => {
                    let c = if SHIFT { SCANCODES_SHIFT[s as usize] } else { SCANCODES[s as usize] };
                    if c == 0 {
                        continue;
                    }
                    if CTRL && c.is_ascii_alphabetic() {
                        return Some(c.to_ascii_lowercase() - b'a' + 1);
                    }
                    return Some(c);
                }
            }
        }
        None
    }
}

/// Бір жол оқиды (эхо мен Backspace-пен). Ctrl-D бос жолда — файл соңы.
pub unsafe fn stdin_read(buf: *mut u8, cap: usize) -> isize {
    let mut n = 0usize;
    loop {
        let c = loop {
            if let Some(c) = poll_char() {
                break c;
            }
            asm!("pause", options(nomem, nostack, preserves_flags));
        };
        match c {
            b'\r' | b'\n' => {
                console_write(b"\n");
                *buf.add(n) = b'\n';
                return (n + 1) as isize;
            }
            0x08 | 0x7F => {
                if n > 0 {
                    while n > 0 && *buf.add(n - 1) & 0xC0 == 0x80 {
                        n -= 1;
                    }
                    n -= 1;
                    console_write(b"\x08 \x08");
                }
            }
            0x04 if n == 0 => return 0,
            c => {
                if n + 2 < cap {
                    *buf.add(n) = c;
                    n += 1;
                    console_write(&[c]);
                }
            }
        }
    }
}

// ------------------------------------------------------------------ шығу және қателер

pub fn exit(code: i32) -> ! {
    crate::arch::power_off(code)
}

pub fn fatal_at(msg: &[u8], rip: usize, rbp: usize) -> ! {
    unsafe {
        print_trace(rip, rbp);
    }
    err_write("Қате: ".as_bytes());
    err_write(msg);
    err_write(b"\n");
    exit(1)
}

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
