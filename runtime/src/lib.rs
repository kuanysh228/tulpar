//! Tulpar рантаймы: GC, мәтіндер, тізімдер, сөздіктер, енгізу-шығару. libc қолданылмайды.
#![no_std]
#![allow(unsafe_op_in_unsafe_fn, static_mut_refs, clippy::missing_safety_doc)]

pub mod common;
pub mod dict;
pub mod dns;
pub mod http;
pub mod fmt;
pub mod gc;
pub mod list;
pub mod math;
pub mod misc;
pub mod net;
pub mod str;
pub mod sys;

use core::arch::asm;

unsafe extern "C" {
    fn tlp_main();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_exit(code: i64) -> ! {
    sys::exit(code as i32)
}

/// Бағдарламаның кірісі (`_start`-тан шақырылады): үйінді, сигналдар, үлкен стек.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_rt_start(init_sp: usize, globals_lo: usize, globals_hi: usize, class_table: *const *const u8, info: *const usize) -> ! {
    sys::INFO = info;
    misc::ARGV_SP = init_sp;
    sys::install_signals();
    gc::init(globals_lo, globals_hi);
    fmt::set_class_table(class_table);

    // 1 ГБ виртуал стек (физикалық жад қолданылған сайын ғана бөлінеді); төменінде қорғаныш беті
    let mut size = 1usize << 30;
    let mut base = 0;
    while size >= (1 << 20) {
        base = sys::os_map(size, true);
        if base != 0 {
            break;
        }
        size >>= 1;
    }
    if base == 0 {
        sys::fatal("стек үшін жад бөлу мүмкін болмады".as_bytes());
    }
    const GUARD: usize = 1 << 16;
    sys::sys3(sys::SYS_MPROTECT, base, GUARD, 0);
    sys::STACK_GUARD_LO = base.saturating_sub(1 << 16);
    sys::STACK_GUARD_HI = base + GUARD + (1 << 16);
    let top = (base + size) & !15;
    gc::STACK_TOP = top;
    asm!(
        "mov rsp, {top}",
        "xor ebp, ebp",
        "call {main}",
        "xor edi, edi",
        "call {exit}",
        top = in(reg) top,
        main = sym tlp_main,
        exit = sym tlp_exit,
        options(noreturn)
    );
}
