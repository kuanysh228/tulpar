//! x86-64: порттар, GDT/TSS, IDT, PIC, үзіліс өңдеушілері және `tlp_hw_*` ішкі функциялары.
//!
//! Үзіліс кірісі IST1 стегінде орындалады, содан кейін жақтау үзілген стекке (қызыл аймақтан төмен) көшіріледі:
//! Rust рантаймы мен Tulpar коды қызыл аймақты (red zone) қолдана алады, ал өңдеушілер әдеттегі стекте жұмыс істейді
//! (GC стекті дұрыс сканерлейді, трасса дұрыс шығады).

use crate::sys::{self, err_write, fmt_u64};
use core::arch::{asm, global_asm};

// ------------------------------------------------------------------ порттар

#[inline(always)]
pub unsafe fn outb(port: u16, v: u8) {
    asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags));
}
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack, preserves_flags));
    v
}
#[inline(always)]
unsafe fn outw(port: u16, v: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack, preserves_flags));
}
#[inline(always)]
unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    asm!("in ax, dx", in("dx") port, out("ax") v, options(nomem, nostack, preserves_flags));
    v
}
#[inline(always)]
unsafe fn outl(port: u16, v: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack, preserves_flags));
}
#[inline(always)]
unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    asm!("in eax, dx", in("dx") port, out("eax") v, options(nomem, nostack, preserves_flags));
    v
}

// ------------------------------------------------------------------ GDT / TSS

#[repr(C, packed)]
struct Tss {
    _r0: u32,
    rsp: [u64; 3],
    _r1: u64,
    ist: [u64; 7],
    _r2: u64,
    _r3: u16,
    iomap: u16,
}

#[repr(C, align(16))]
struct Stack16k([u8; 16384]);

static mut IST1_STACK: Stack16k = Stack16k([0; 16384]);
static mut IST2_STACK: Stack16k = Stack16k([0; 16384]);
static mut TSS: Tss = Tss { _r0: 0, rsp: [0; 3], _r1: 0, ist: [0; 7], _r2: 0, _r3: 0, iomap: 104 };
static mut GDT: [u64; 5] = [0; 5];

#[repr(C, packed)]
struct DescPtr {
    limit: u16,
    base: u64,
}

unsafe fn gdt_init() {
    TSS.ist[0] = core::ptr::addr_of!(IST1_STACK) as u64 + 16384;
    TSS.ist[1] = core::ptr::addr_of!(IST2_STACK) as u64 + 16384;
    let base = core::ptr::addr_of!(TSS) as u64;
    let limit = (core::mem::size_of::<Tss>() - 1) as u64;
    GDT[0] = 0;
    GDT[1] = 0x00AF_9A00_0000_FFFF; // 0x08: код (64 бит)
    GDT[2] = 0x00CF_9200_0000_FFFF; // 0x10: деректер
    GDT[3] = (limit & 0xFFFF) | ((base & 0xFFFF) << 16) | (((base >> 16) & 0xFF) << 32) | (0x89u64 << 40) | (((limit >> 16) & 0xF) << 48) | (((base >> 24) & 0xFF) << 56);
    GDT[4] = base >> 32; // 0x18: TSS
    let p = DescPtr { limit: (core::mem::size_of_val(&GDT) - 1) as u16, base: core::ptr::addr_of!(GDT) as u64 };
    asm!(
        "lgdt [{p}]",
        "push 0x08",
        "lea {t}, [rip + 2f]",
        "push {t}",
        "retfq",
        "2:",
        "mov ax, 0x10",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "xor eax, eax",
        "mov fs, ax",
        "mov gs, ax",
        "mov ax, 0x18",
        "ltr ax",
        p = in(reg) &p,
        t = out(reg) _,
        out("rax") _,
    );
}

// ------------------------------------------------------------------ IDT

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    lo: u16,
    sel: u16,
    ist: u8,
    attr: u8,
    mid: u16,
    hi: u32,
    _z: u32,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry { lo: 0, sel: 0, ist: 0, attr: 0, mid: 0, hi: 0, _z: 0 }; 256];
static mut HANDLERS: [usize; 256] = [0; 256];

unsafe extern "C" {
    static tlp_isr_stubs: u8;
}

unsafe fn idt_init() {
    let base = core::ptr::addr_of!(tlp_isr_stubs) as usize;
    for v in 0..256usize {
        let a = base + v * 16;
        IDT[v] = IdtEntry {
            lo: a as u16,
            sel: 0x08,
            ist: if v == 8 { 2 } else { 1 },
            attr: 0x8E, // үзіліс қақпасы: IF өшіріледі
            mid: (a >> 16) as u16,
            hi: (a >> 32) as u32,
            _z: 0,
        };
    }
    let p = DescPtr { limit: (core::mem::size_of_val(&IDT) - 1) as u16, base: core::ptr::addr_of!(IDT) as u64 };
    asm!("lidt [{p}]", p = in(reg) &p);
}

// Әр вектор үшін 16 байттық түйреуіш: қате коды жоқ векторларға 0 салады, вектор нөмірін салады, ортақ коды шақырады.
global_asm!(
    r#"
.section .text.isr, "ax"
.balign 16
.globl tlp_isr_stubs
tlp_isr_stubs:
.set vec, 0
.rept 256
.balign 16
.if (vec == 8) || (vec == 10) || (vec == 11) || (vec == 12) || (vec == 13) || (vec == 14) || (vec == 17) || (vec == 21) || (vec == 29) || (vec == 30)
.else
    .byte 0x6A, 0x00
.endif
    .byte 0x68
    .long vec
    jmp tlp_isr_common
.set vec, vec+1
.endr

.balign 16
tlp_isr_common:
    push rax
    push rcx
    cmp qword ptr [rsp + 16], 8
    je 5f
    mov rax, qword ptr [rsp + 56]
    sub rax, 192
    and rax, -16
    mov rcx, qword ptr [rsp + 0]
    mov qword ptr [rax + 0], rcx
    mov rcx, qword ptr [rsp + 8]
    mov qword ptr [rax + 8], rcx
    mov rcx, qword ptr [rsp + 16]
    mov qword ptr [rax + 16], rcx
    mov rcx, qword ptr [rsp + 24]
    mov qword ptr [rax + 24], rcx
    mov rcx, qword ptr [rsp + 32]
    mov qword ptr [rax + 32], rcx
    mov rcx, qword ptr [rsp + 40]
    mov qword ptr [rax + 40], rcx
    mov rcx, qword ptr [rsp + 48]
    mov qword ptr [rax + 48], rcx
    mov rcx, qword ptr [rsp + 56]
    mov qword ptr [rax + 56], rcx
    mov rcx, qword ptr [rsp + 64]
    mov qword ptr [rax + 64], rcx
    mov rsp, rax
5:
    pop rcx
    pop rax
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rbx, rsp
    sub rsp, 528
    and rsp, -16
    fxsave [rsp]
    cld
    mov rdi, rbx
    call tlp_isr_dispatch
    fxrstor [rsp]
    mov rsp, rbx
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16
    iretq
"#
);

#[repr(C)]
pub struct Frame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    vec: u64,
    err: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

fn exception_name(v: u64, cr2: u64) -> &'static str {
    match v {
        0 => "нөлге бөлу немесе бүтін санның асып кетуі",
        1 => "түзету үзілісі (#DB)",
        2 => "бүркемеленбейтін үзіліс (NMI)",
        3 => "тоқтау нүктесі (#BP)",
        4 => "асып кету (#OF)",
        5 => "шектен шығу (#BR)",
        6 => "жарамсыз команда (#UD)",
        7 => "FPU қолжетімсіз (#NM)",
        8 => "қос қате (#DF): көбіне стек толып кетті",
        10 => "жарамсыз TSS (#TS)",
        11 => "сегмент жоқ (#NP)",
        12 => "стек сегментінің қатесі (#SS)",
        13 => "жалпы қорғаныс қатесі (#GP)",
        14 => {
            if cr2 < 4096 * 16 {
                "нөлдік көрсеткішке (жоқ) қатынау"
            } else {
                "жадқа жарамсыз қатынау (бет қатесі #PF)"
            }
        }
        16 => "x87 қатесі (#MF)",
        17 => "туралау қатесі (#AC)",
        18 => "машина тексеру қатесі (#MC)",
        19 => "SIMD қатесі (#XM)",
        _ => "күтпеген ерекше жағдай",
    }
}

fn put_hex(v: u64) {
    let mut b = [0u8; 18];
    b[0] = b'0';
    b[1] = b'x';
    for i in 0..16 {
        let d = ((v >> (60 - i * 4)) & 0xF) as u8;
        b[2 + i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
    }
    err_write(&b);
}

#[unsafe(no_mangle)]
unsafe extern "C" fn tlp_isr_dispatch(f: *mut Frame) {
    let v = (*f).vec as usize;
    let h = HANDLERS[v];
    if h != 0 {
        let func: extern "C" fn(i64, i64) = core::mem::transmute(h);
        func(v as i64, (*f).err as i64);
    } else if v < 32 {
        let cr2: u64;
        asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
        err_write(b"\n[CPU] vec=");
        let mut b = [0u8; 24];
        err_write(fmt_u64(v as u64, &mut b));
        err_write(b" err=");
        put_hex((*f).err);
        err_write(b" rip=");
        put_hex((*f).rip);
        err_write(b" rsp=");
        put_hex((*f).rsp);
        if v == 14 {
            err_write(b" cr2=");
            put_hex(cr2);
        }
        err_write(b"\n");
        let regs: [(&[u8], u64); 15] = [
            (b"rax", (*f).rax), (b"rbx", (*f).rbx), (b"rcx", (*f).rcx), (b"rdx", (*f).rdx), (b"rsi", (*f).rsi),
            (b"rdi", (*f).rdi), (b"rbp", (*f).rbp), (b"r8", (*f).r8), (b"r9", (*f).r9), (b"r10", (*f).r10),
            (b"r11", (*f).r11), (b"r12", (*f).r12), (b"r13", (*f).r13), (b"r14", (*f).r14), (b"r15", (*f).r15),
        ];
        for (i, (n, v)) in regs.iter().enumerate() {
            err_write(n);
            err_write(b"=");
            put_hex(*v);
            err_write(if i % 4 == 3 { b"\n" } else { b" " });
        }
        err_write(b"\n");
        let guard = v == 14 && cr2 as usize >= sys::STACK_GUARD_LO && (cr2 as usize) < sys::STACK_GUARD_HI + 4096;
        let msg = if guard { "стек толып кетті (рекурсия тым терең)" } else { exception_name(v as u64, cr2) };
        sys::fatal_at(msg.as_bytes(), (*f).rip as usize, (*f).rbp as usize);
    }
    if (32..48).contains(&v) {
        pic_eoi((v - 32) as u8);
    }
}

// ------------------------------------------------------------------ PIC

unsafe fn io_wait() {
    outb(0x80, 0);
}

unsafe fn pic_init() {
    outb(0x20, 0x11);
    io_wait();
    outb(0xA0, 0x11);
    io_wait();
    outb(0x21, 0x20);
    io_wait();
    outb(0xA1, 0x28);
    io_wait();
    outb(0x21, 4);
    io_wait();
    outb(0xA1, 2);
    io_wait();
    outb(0x21, 1);
    io_wait();
    outb(0xA1, 1);
    io_wait();
    outb(0x21, 0xFF); // IRQ-лар өшірулі: пайдаланушы порт арқылы қосады
    outb(0xA1, 0xFF);
}

unsafe fn pic_eoi(irq: u8) {
    if irq == 7 {
        outb(0x20, 0x0B);
        if inb(0x20) & 0x80 == 0 {
            return; // жалған үзіліс
        }
    }
    if irq == 15 {
        outb(0xA0, 0x0B);
        if inb(0xA0) & 0x80 == 0 {
            outb(0x20, 0x20);
            return;
        }
    }
    if irq >= 8 {
        outb(0xA0, 0x20);
    }
    outb(0x20, 0x20);
}

// ------------------------------------------------------------------ іске қосу

static mut MBI: u64 = 0;
static mut MAGIC: u64 = 0;

pub unsafe fn init(mbi: u64, magic: u64) {
    MBI = mbi;
    MAGIC = magic;
    asm!("fninit", options(nomem, nostack));
    gdt_init();
    idt_init();
    pic_init();
}

unsafe fn rd32(a: usize) -> usize {
    core::ptr::read_unaligned(a as *const u32) as usize
}

unsafe fn cstr_len(a: usize) -> usize {
    let mut n = 0;
    while *((a + n) as *const u8) != 0 && n < 4096 {
        n += 1;
    }
    n + 1
}

/// Үйінді (heap) шектері `[төмен, жоғары)`. Ядродан кейінгі бос RAM, бірақ Multiboot ақпараты (QEMU оны ядро
/// кескінінің дәл соңына қояды), оның жолдары, карта және модульдер үйіндіге кірмеуі керек. Жоғарғы шек —
/// RAM соңы (Multiboot бойынша), 3 ГБ-тан аспайды (одан жоғарысы MMIO).
pub unsafe fn heap_bounds(kernel_end: usize) -> (usize, usize) {
    let mut lo = kernel_end;
    let mut top = 128usize << 20;
    if MAGIC == 0x2BAD_B002 && MBI != 0 {
        let mbi = MBI as usize;
        let flags = rd32(mbi);
        if flags & 1 != 0 {
            top = 0x10_0000 + rd32(mbi + 8) * 1024; // mem_upper, КБ
        }
        top = top.min(0xC000_0000);
        // ақпарат аймақтары: (бастауы, ұзындығы)
        let mut regions: [(usize, usize); 40] = [(0, 0); 40];
        let mut n = 0;
        let mut add = |a: usize, l: usize| {
            if a != 0 && n < regions.len() {
                regions[n] = (a, l);
                n += 1;
            }
        };
        add(mbi, 116);
        if flags & (1 << 2) != 0 {
            let a = rd32(mbi + 16);
            add(a, cstr_len(a));
        }
        if flags & (1 << 3) != 0 {
            let (cnt, addr) = (rd32(mbi + 20).min(16), rd32(mbi + 24));
            add(addr, cnt * 16);
            for k in 0..cnt {
                let m = addr + k * 16;
                add(rd32(m), rd32(m + 4).saturating_sub(rd32(m)));
                let st = rd32(m + 8);
                add(st, cstr_len(st));
            }
        }
        if flags & (1 << 6) != 0 {
            add(rd32(mbi + 48), rd32(mbi + 44));
        }
        if flags & (1 << 7) != 0 {
            add(rd32(mbi + 56), rd32(mbi + 52));
        }
        if flags & (1 << 9) != 0 {
            let a = rd32(mbi + 64);
            add(a, cstr_len(a));
        }
        // ядродан кейінгі жақын аймақтар — үйіндінің басын жылжытады; алыстары — жоғарғы шекті төмендетеді
        let near = kernel_end + (64 << 20);
        for &(a, l) in &regions[..n] {
            if a + l <= kernel_end {
                continue;
            }
            if a < near {
                lo = lo.max(a + l);
            } else {
                top = top.min(a);
            }
        }
    }
    lo = (lo + 4095) & !4095;
    (lo, top.max(lo))
}

/// Бағдарламаны аяқтайды: QEMU `isa-debug-exit` (0xF4) және ACPI өшіру порты арқылы, болмаса тоқтайды.
pub fn power_off(code: i32) -> ! {
    unsafe {
        outl(0xF4, code as u32);
        outw(0x604, 0x2000); // QEMU ACPI PM1a_CNT: S5
        outw(0xB004, 0x2000); // ескі QEMU/Bochs
        loop {
            asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}

// ------------------------------------------------------------------ Tulpar-дан шақырылатын ішкі функциялар

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_in8(p: u64) -> i64 {
    inb(p as u16) as i64
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_in16(p: u64) -> i64 {
    inw(p as u16) as i64
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_in32(p: u64) -> i64 {
    inl(p as u16) as i64
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_out8(p: u64, v: u64) {
    outb(p as u16, v as u8)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_out16(p: u64, v: u64) {
    outw(p as u16, v as u16)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_out32(p: u64, v: u64) {
    outl(p as u16, v as u32)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_rd8(a: u64) -> i64 {
    core::ptr::read_volatile(a as *const u8) as i64
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_rd16(a: u64) -> i64 {
    core::ptr::read_volatile(a as *const u16) as i64
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_rd32(a: u64) -> i64 {
    core::ptr::read_volatile(a as *const u32) as i64
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_rd64(a: u64) -> i64 {
    core::ptr::read_volatile(a as *const u64) as i64
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_wr8(a: u64, v: u64) {
    core::ptr::write_volatile(a as *mut u8, v as u8)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_wr16(a: u64, v: u64) {
    core::ptr::write_volatile(a as *mut u16, v as u16)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_wr32(a: u64, v: u64) {
    core::ptr::write_volatile(a as *mut u32, v as u32)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_wr64(a: u64, v: u64) {
    core::ptr::write_volatile(a as *mut u64, v)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_hlt() {
    asm!("hlt", options(nomem, nostack, preserves_flags));
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_sti() {
    asm!("sti", options(nomem, nostack));
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_cli() {
    asm!("cli", options(nomem, nostack));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_cr_read(n: u64) -> i64 {
    let v: u64;
    match n {
        0 => asm!("mov {}, cr0", out(reg) v, options(nomem, nostack, preserves_flags)),
        2 => asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)),
        3 => asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)),
        4 => asm!("mov {}, cr4", out(reg) v, options(nomem, nostack, preserves_flags)),
        _ => sys::fatal("cr_оқы: тек CR0, CR2, CR3, CR4 қолдау табады".as_bytes()),
    }
    v as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_cr_write(n: u64, v: u64) {
    match n {
        0 => asm!("mov cr0, {}", in(reg) v, options(nostack)),
        3 => asm!("mov cr3, {}", in(reg) v, options(nostack)),
        4 => asm!("mov cr4, {}", in(reg) v, options(nostack)),
        _ => sys::fatal("cr_жаз: тек CR0, CR3, CR4 қолдау табады".as_bytes()),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_msr_read(n: u64) -> i64 {
    let (lo, hi): (u32, u32);
    asm!("rdmsr", in("ecx") n as u32, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
    (((hi as u64) << 32) | lo as u64) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_msr_write(n: u64, v: u64) {
    asm!("wrmsr", in("ecx") n as u32, in("eax") v as u32, in("edx") (v >> 32) as u32, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe {
        asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
    }
    ((hi as u64) << 32) | lo as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_hw_rdtsc() -> i64 {
    rdtsc() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_cpuid(leaf: u64, reg: u64) -> i64 {
    let r = core::arch::x86_64::__cpuid_count(leaf as u32, 0);
    (match reg {
        0 => r.eax,
        1 => r.ebx,
        2 => r.ecx,
        _ => r.edx,
    }) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_invlpg(a: u64) {
    asm!("invlpg [{}]", in(reg) a, options(nostack, preserves_flags));
}

/// `үзіліс_орнату(нөмір, өңдеуші)`: өңдеуші `(нөмір, қате_коды)` қабылдайды; 32–47 — IRQ 0–15 (EOI автоматты).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_set_handler(v: u64, f: u64) {
    if v >= 256 {
        sys::fatal("үзіліс_орнату: нөмір 0..255 болуы керек".as_bytes());
    }
    HANDLERS[v as usize] = f as usize;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_hw_mbi() -> i64 {
    MBI as i64
}

/// Блоктаусыз пернетақта/COM1 оқу: ASCII коды, жоқ болса 0.
#[unsafe(no_mangle)]
pub extern "C" fn tlp_hw_key() -> i64 {
    sys::poll_char().unwrap_or(0) as i64
}
