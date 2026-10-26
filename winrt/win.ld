/* Windows x64 .exe үшін жинау скрипті. Нәтиже ELF `pe.rs` арқылы PE файлына ауыстырылады:
   әр PT_LOAD сегменті — бір PE секциясы (ImageBase = 0x140000000, RVA = vaddr - ImageBase). */
ENTRY(_start)

PHDRS
{
    text  PT_LOAD FLAGS(5);
    rdata PT_LOAD FLAGS(4);
    data  PT_LOAD FLAGS(6);
}

SECTIONS
{
    . = 0x140001000;

    .text : ALIGN(4K) {
        *(.text .text.*)
    } :text

    .rdata : ALIGN(4K) {
        *(.rodata .rodata.*)
    } :rdata

    .data : ALIGN(4K) {
        *(.data .data.*)
    } :data

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment)
        *(.note*)
    }
}
