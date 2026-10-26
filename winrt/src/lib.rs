//! Tulpar Windows рантаймы (.exe): GC, мәтіндер, тізімдер, сөздіктер — `runtime/` модульдерімен бірдей;
//! ОС қабаты (`sys.rs`, `misc.rs`) kernel32 арқылы жазылған. libc қолданылмайды.
//!
//! Бағдарлама ELF ретінде `win.ld` скриптімен 0x140000000 адресіне жиналады да, `compiler/src/pe.rs`
//! оны PE файлына орайды. Ішкі шақыру ережесі бәрінде System V (Windows API-ге `extern "win64"` арқылы шақырылады).
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

pub mod http;
pub mod misc;
pub mod net;
pub mod sys;
pub mod tls;
pub mod win;

use core::arch::asm;

unsafe extern "C" {
    fn tlp_main();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_exit(code: i64) -> ! {
    sys::exit(code as i32)
}

/// Бағдарламаның кірісі (`_start`-тан шақырылады, Windows негізгі ағынының стегінде).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_rt_start(_init_sp: usize, globals_lo: usize, globals_hi: usize, class_table: *const *const u8, info: *const usize) -> ! {
    win::keep_imports();
    sys::INFO = info;
    sys::init_console();
    sys::install_handlers();
    gc::init(globals_lo, globals_hi);
    fmt::set_class_table(class_table);

    // ағын стегінің жоғарғы шегі (TEB.StackBase): GC стекті осыған дейін сканерлейді
    let top: usize;
    asm!("mov {}, gs:[8]", out(reg) top, options(nomem, nostack, preserves_flags));
    gc::STACK_TOP = top & !15;

    tlp_main();
    tlp_exit(0)
}
