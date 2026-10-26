//! Tulpar ядро рантаймы: Multiboot жүктелуі, long mode, GDT/IDT/PIC, консоль (COM1 + VGA), GC үйіндісі.
//!
//! Тіл рантаймының (`runtime/`) GC, мәтін, тізім, сөздік модульдері өзгеріссіз қолданылады;
//! ОЖ қабаты (`sys.rs`) бұл жерде жалаң темір үшін қайта жазылған.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn, static_mut_refs, clippy::missing_safety_doc)]

#[path = "../../runtime/src/common.rs"]
pub mod common;
#[path = "../../runtime/src/dict.rs"]
pub mod dict;
#[path = "../../runtime/src/fmt.rs"]
pub mod fmt;
#[path = "../../runtime/src/gc.rs"]
pub mod gc;
#[path = "../../runtime/src/list.rs"]
pub mod list;
#[path = "../../runtime/src/math.rs"]
pub mod math;
#[path = "../../runtime/src/str.rs"]
pub mod str;

pub mod arch;
pub mod misc;
pub mod sys;

use core::arch::global_asm;

unsafe extern "C" {
    fn tlp_main();
    static tlp_module_desc: [usize; 4];
    static __kernel_end: u8;
    static kernel_stack_top: u8;
    static kernel_stack: u8;
}

// ------------------------------------------------------------------ жүктеуші (Multiboot 1)
//
// GRUB немесе `qemu -kernel` 32 биттік қорғалған режимде `_start`-ты шақырады (eax = 0x2BADB002, ebx = жүктеу ақпараты).
// Мұнда бет кестелері құрылады (төменгі 4 ГБ бірлікті бейнеленеді, 0-бет жоқ: нөлдік көрсеткіш қатесін ұстау үшін),
// long mode қосылады, SSE қосылады, содан кейін `tlp_kernel_start` шақырылады.
global_asm!(
    r#"
.section .multiboot, "a"
.balign 4
    .long 0x1BADB002
    .long 0x00000003
    .long -(0x1BADB002 + 0x00000003)

.section .bss.boot, "aw", @nobits
.balign 4096
boot_pml4:  .zero 4096
boot_pdpt:  .zero 4096
boot_pd:    .zero 16384
boot_pt0:   .zero 4096
boot_pt1:   .zero 4096
boot_mbi:   .zero 8
boot_magic: .zero 8

.section .bss.stack, "aw", @nobits
.balign 2097152
.globl kernel_stack
kernel_stack:
    .zero 1048576
.globl kernel_stack_top
kernel_stack_top:

.section .boot, "ax"
.balign 16
boot_gdt:
    .quad 0
    .quad 0x00209A0000000000
    .quad 0x0000920000000000
boot_gdt_ptr:
    .word 23
    .long boot_gdt

.code32
.globl _start
_start:
    cli
    mov dword ptr [boot_mbi], ebx
    mov dword ptr [boot_magic], eax
    mov esp, offset kernel_stack_top

    // бет кестелерін нөлдеу
    mov edi, offset boot_pml4
    mov ecx, (4096 + 4096 + 16384 + 4096 + 4096) / 4
    xor eax, eax
    cld
    rep stosd

    mov eax, offset boot_pdpt
    or eax, 3
    mov dword ptr [boot_pml4], eax

    xor ecx, ecx
1:
    mov eax, ecx
    shl eax, 12
    add eax, offset boot_pd
    or eax, 3
    mov dword ptr [boot_pdpt + ecx*8], eax
    inc ecx
    cmp ecx, 4
    jne 1b

    // 2 МБ-тық беттермен 4 ГБ; 3 ГБ-тан жоғарысы (MMIO) кэштелмейді
    xor ecx, ecx
2:
    mov eax, ecx
    shl eax, 21
    or eax, 0x83
    cmp ecx, 1536
    jb 3f
    or eax, 0x18
3:
    mov dword ptr [boot_pd + ecx*8], eax
    inc ecx
    cmp ecx, 2048
    jne 2b

    // бірінші 2 МБ — 4 КБ беттермен; 0-бет жоқ
    mov eax, offset boot_pt0
    or eax, 3
    mov dword ptr [boot_pd], eax
    xor ecx, ecx
4:
    mov eax, ecx
    shl eax, 12
    or eax, 3
    mov dword ptr [boot_pt0 + ecx*8], eax
    inc ecx
    cmp ecx, 512
    jne 4b
    mov dword ptr [boot_pt0], 0

    // стектің ең төменгі беті жоқ (қорғаныс беті): стек толса #PF, ал үнсіз жадты бұзу емес.
    // Стек 2 МБ-қа туралған, сондықтан ол орналасқан 2 МБ-тық бет те 4 КБ беттерге бөлінеді (boot_pt1)
    mov edx, offset kernel_stack
    shr edx, 21
    shl edx, 21
    xor ecx, ecx
6:
    mov eax, ecx
    shl eax, 12
    add eax, edx
    or eax, 3
    mov dword ptr [boot_pt1 + ecx*8], eax
    inc ecx
    cmp ecx, 512
    jne 6b
    mov eax, offset kernel_stack
    shr eax, 21
    mov esi, offset boot_pt1
    or esi, 3
    mov dword ptr [boot_pd + eax*8], esi
    mov dword ptr [boot_pt1], 0

    mov eax, cr4
    or eax, 0x620                  // PAE | OSFXSR | OSXMMEXCPT
    mov cr4, eax
    mov eax, offset boot_pml4
    mov cr3, eax
    mov ecx, 0xC0000080
    rdmsr
    or eax, 0x100                  // EFER.LME
    wrmsr
    mov eax, cr0
    and eax, 0xFFFFFFFB            // EM = 0
    or eax, 0x80010023             // PG | WP | NE | MP | PE
    mov cr0, eax
    mov eax, offset boot_gdt_ptr
    lgdt [eax]
    .byte 0xEA                     // jmp 0x08:long_entry
    .long long_entry
    .word 0x08

.code64
long_entry:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    xor eax, eax
    mov fs, ax
    mov gs, ax
    lea rsp, [rip + kernel_stack_top]
    xor ebp, ebp
    mov edi, dword ptr [rip + boot_mbi]
    mov esi, dword ptr [rip + boot_magic]
    call tlp_kernel_start
5:
    cli
    hlt
    jmp 5b
"#
);

/// Ядроның Rust кірісі: консоль, үзілістер, үйінді, содан кейін пайдаланушы бағдарламасы (`tlp_main`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_kernel_start(mbi: u64, magic: u64) -> ! {
    sys::console_init();
    arch::init(mbi, magic);

    let (heap_lo, heap_hi) = arch::heap_bounds(core::ptr::addr_of!(__kernel_end) as usize);
    sys::mem_init(heap_lo, heap_hi);
    misc::calibrate_clock();

    let d = &tlp_module_desc;
    sys::INFO = d[3] as *const usize;
    gc::init(d[0], d[1]);
    fmt::set_class_table(d[2] as *const *const u8);
    gc::STACK_TOP = core::ptr::addr_of!(kernel_stack_top) as usize;
    sys::STACK_GUARD_LO = core::ptr::addr_of!(kernel_stack) as usize;
    sys::STACK_GUARD_HI = sys::STACK_GUARD_LO + 4096;

    tlp_main();
    sys::exit(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_exit(code: i64) -> ! {
    sys::exit(code as i32)
}
