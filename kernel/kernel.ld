/* Tulpar ядросының жинау скрипті: 1 МБ-тан басталады, Multiboot (GRUB / QEMU -kernel) үшін. */
ENTRY(_start)

SECTIONS
{
    . = 1M;

    .boot : ALIGN(4K) {
        KEEP(*(.multiboot))
        KEEP(*(.boot))
    }

    .text : ALIGN(4K) {
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        *(.bss.boot)
        *(.bss .bss.*)
        *(COMMON)
    }

    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment)
        *(.note*)
    }
}
