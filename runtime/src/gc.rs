//! Жад: консервативті mark-sweep GC (өлшем кластары бойынша spans) + қолмен басқарылатын аллокатор.
//!
//! * Үйінді (heap) бір үлкен виртуал аймақ, 4 КБ беттерге бөлінген.
//! * Бет кестесі әр беттен span метадеректеріне сілтейді; бұл ішкі көрсеткіштерді де табуға мүмкіндік береді.
//! * Кіші объектілер (<= 8 КБ) өлшем кластарына арналған span-дарда, үлкендері — жеке span.
//! * Түбірлер: стек (қауіпсіз регистрлерімен), глобал айнымалылар аймағы.
//! * `қолмен:` аллокатор (`tlp_mem_alloc`/`tlp_mem_free`) GC-ден тыс span-дарды қолданады.

use crate::sys::*;
use core::arch::global_asm;
use core::ptr;

const PAGE_SHIFT: usize = 12;
const PAGE: usize = 1 << PAGE_SHIFT;
const MAX_SMALL: usize = 8192;
const NCLASS: usize = 30;
const SIZES: [usize; NCLASS] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024, 1280, 1536, 1792, 2048, 2560, 3072, 4096,
    5120, 6144, 8192,
];
const MIN_THRESHOLD: usize = 4 << 20;

const K_FREE: u8 = 0;
const K_SMALL: u8 = 1;
const K_LARGE: u8 = 2;

#[repr(C)]
struct Span {
    start: usize,
    /// кіші: элемент өлшемі; үлкен: жалпы байт
    elem: usize,
    free: usize,
    npages: u32,
    nelems: u32,
    bump: u32,
    next: u32,
    kind: u8,
    cls: u8,
    noscan: bool,
    manual: bool,
    zeroed: bool,
    marked_large: bool,
    marks: [u64; 16],
}

static mut HEAP_START: usize = 0;
static mut HEAP_END: usize = 0;
static mut HEAP_TOP: usize = 0;
static mut PAGE_TABLE: *mut u32 = ptr::null_mut();
static mut SPANS: *mut Span = ptr::null_mut();
static mut SPAN_COUNT: u32 = 0;
static mut SPAN_FREE: u32 = 0;
static mut FREE_RUNS: *mut [usize; 2] = ptr::null_mut();
static mut FREE_RUN_COUNT: usize = 0;
const FREE_RUN_CAP: usize = GC_FREE_RUN_CAP;
static mut FREE_PAGES: usize = 0;

/// [0] — GC, scan; [1] — GC, noscan; [2] — қолмен (көрсеткіштерсіз); [3] — қолмен (GC сілтемелері бар: түбір)
static mut CUR: [[u32; NCLASS]; 4] = [[0; NCLASS]; 4];
static mut PARTIAL: [[u32; NCLASS]; 4] = [[0; NCLASS]; 4];
static mut MFREE: [[usize; NCLASS]; 2] = [[0; NCLASS]; 2];
static mut CLASS_OF: [u8; 65] = [0; 65];

static mut ALLOC_SINCE: usize = 0;
static mut THRESHOLD: usize = MIN_THRESHOLD;
static mut LIVE_BYTES: usize = 0;
pub static mut GC_COUNT: usize = 0;
static mut GC_ON: bool = false;

pub static mut STACK_TOP: usize = 0;
static mut GLOBALS_LO: usize = 0;
static mut GLOBALS_HI: usize = 0;

static mut MARK_STACK: *mut usize = ptr::null_mut();
static mut MARK_SP: usize = 0;
const MARK_CAP: usize = GC_MARK_CAP;

pub unsafe fn init(globals_lo: usize, globals_hi: usize) {
    unsafe {
        let mut size = os_heap_hint().min(GC_HEAP_MAX);
        let mut base = 0;
        while size >= GC_HEAP_MIN {
            base = os_map(size, true);
            if base != 0 {
                break;
            }
            size >>= 1;
        }
        if base == 0 {
            fatal("үйінді үшін жад бөлу мүмкін болмады".as_bytes());
        }
        HEAP_START = (base + PAGE - 1) & !(PAGE - 1);
        HEAP_END = base + size;
        HEAP_TOP = HEAP_START;
        let npages = (HEAP_END - HEAP_START) >> PAGE_SHIFT;
        PAGE_TABLE = os_map(npages * 4, true) as *mut u32;
        SPANS = os_map(npages * core::mem::size_of::<Span>(), true) as *mut Span;
        FREE_RUNS = os_map(FREE_RUN_CAP * 16, true) as *mut [usize; 2];
        MARK_STACK = os_map(MARK_CAP * 8, true) as *mut usize;
        if PAGE_TABLE.is_null() || SPANS.is_null() || FREE_RUNS.is_null() || MARK_STACK.is_null() {
            fatal("GC метадеректері үшін жад жеткіліксіз".as_bytes());
        }
        let mut c = 0;
        for i in 0..65 {
            while SIZES[c] < i * 16 {
                c += 1;
            }
            CLASS_OF[i] = c as u8;
        }
        GLOBALS_LO = globals_lo;
        GLOBALS_HI = globals_hi;
        GC_ON = true;
    }
}

// ------------------------------------------------------------------ беттер

unsafe fn alloc_pages(n: usize) -> (usize, bool) {
    unsafe {
        for i in 0..FREE_RUN_COUNT {
            let run = FREE_RUNS.add(i);
            if (*run)[1] >= n {
                let start = (*run)[0];
                (*run)[0] += n;
                (*run)[1] -= n;
                if (*run)[1] == 0 {
                    ptr::copy(FREE_RUNS.add(i + 1), FREE_RUNS.add(i), FREE_RUN_COUNT - i - 1);
                    FREE_RUN_COUNT -= 1;
                }
                FREE_PAGES -= n;
                return (HEAP_START + (start << PAGE_SHIFT), false);
            }
        }
        let bytes = n << PAGE_SHIFT;
        if HEAP_TOP + bytes > HEAP_END {
            fatal("жад таусылды".as_bytes());
        }
        let a = HEAP_TOP;
        HEAP_TOP += bytes;
        (a, true)
    }
}

unsafe fn free_pages(addr: usize, n: usize) {
    unsafe {
        let page = (addr - HEAP_START) >> PAGE_SHIFT;
        // сұрыпталған жерді табу
        let mut lo = 0;
        let mut hi = FREE_RUN_COUNT;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if (*FREE_RUNS.add(mid))[0] < page {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut start = page;
        let mut len = n;
        let mut at = lo;
        // алдыңғымен біріктіру
        if at > 0 {
            let prev = *FREE_RUNS.add(at - 1);
            if prev[0] + prev[1] == start {
                start = prev[0];
                len += prev[1];
                at -= 1;
                ptr::copy(FREE_RUNS.add(at + 1), FREE_RUNS.add(at), FREE_RUN_COUNT - at - 1);
                FREE_RUN_COUNT -= 1;
            }
        }
        // келесімен біріктіру
        if at < FREE_RUN_COUNT {
            let next = *FREE_RUNS.add(at);
            if start + len == next[0] {
                len += next[1];
                ptr::copy(FREE_RUNS.add(at + 1), FREE_RUNS.add(at), FREE_RUN_COUNT - at - 1);
                FREE_RUN_COUNT -= 1;
            }
        }
        FREE_PAGES += n;
        // үйіндінің ұшына тірелсе — ОЖ-ге қайтарамыз
        if HEAP_START + ((start + len) << PAGE_SHIFT) == HEAP_TOP {
            let a = HEAP_START + (start << PAGE_SHIFT);
            os_release(a, len << PAGE_SHIFT); // беттер нөлге айналады
            HEAP_TOP = a;
            FREE_PAGES -= len;
            return;
        }
        if FREE_RUN_COUNT >= FREE_RUN_CAP {
            return; // метадерек толды: беттер жоғалады (өте сирек)
        }
        ptr::copy(FREE_RUNS.add(at), FREE_RUNS.add(at + 1), FREE_RUN_COUNT - at);
        *FREE_RUNS.add(at) = [start, len];
        FREE_RUN_COUNT += 1;
    }
}

unsafe fn take_span() -> u32 {
    unsafe {
        if SPAN_FREE != 0 {
            let i = SPAN_FREE - 1;
            SPAN_FREE = (*SPANS.add(i as usize)).next;
            return i;
        }
        let i = SPAN_COUNT;
        SPAN_COUNT += 1;
        i
    }
}

unsafe fn new_span(npages: usize, kind: u8, cls: usize, elem: usize, nelems: usize, noscan: bool, manual: bool) -> u32 {
    unsafe {
        let (addr, fresh) = alloc_pages(npages);
        let idx = take_span();
        let s = SPANS.add(idx as usize);
        *s = Span {
            start: addr,
            elem,
            free: 0,
            npages: npages as u32,
            nelems: nelems as u32,
            bump: if kind == K_LARGE { 1 } else { 0 },
            next: 0,
            kind,
            cls: cls as u8,
            noscan,
            manual,
            zeroed: fresh,
            marked_large: false,
            marks: [0; 16],
        };
        let p0 = (addr - HEAP_START) >> PAGE_SHIFT;
        for i in 0..npages {
            *PAGE_TABLE.add(p0 + i) = idx + 1;
        }
        idx
    }
}

unsafe fn release_span(idx: u32) {
    unsafe {
        let s = SPANS.add(idx as usize);
        let p0 = ((*s).start - HEAP_START) >> PAGE_SHIFT;
        let n = (*s).npages as usize;
        for i in 0..n {
            *PAGE_TABLE.add(p0 + i) = 0;
        }
        let start = (*s).start;
        (*s).kind = K_FREE;
        (*s).next = SPAN_FREE;
        SPAN_FREE = idx + 1;
        free_pages(start, n);
    }
}

// ------------------------------------------------------------------ бөлу

#[inline(always)]
unsafe fn zero(p: *mut u8, n: usize) {
    unsafe {
        if n <= 256 {
            let w = p as *mut u64;
            for i in 0..n / 8 {
                ptr::write_volatile(w.add(i), 0);
            }
        } else {
            memset(p, 0, n);
        }
    }
}

fn class_for(size: usize) -> usize {
    unsafe {
        if size <= 1024 {
            CLASS_OF[(size + 15) >> 4] as usize
        } else {
            let mut c = 20;
            while SIZES[c] < size {
                c += 1;
            }
            c
        }
    }
}

fn span_pages(elem: usize) -> usize {
    let bytes = if elem * 8 > 16384 { elem * 8 } else { 16384 };
    (bytes + PAGE - 1) >> PAGE_SHIFT
}

unsafe fn alloc_small(kind: usize, cls: usize) -> *mut u8 {
    unsafe {
        let elem = SIZES[cls];
        loop {
            let cur = CUR[kind][cls];
            if cur != 0 {
                let s = SPANS.add(cur as usize - 1);
                if (*s).free != 0 {
                    let p = (*s).free as *mut usize;
                    (*s).free = *p;
                    zero(p as *mut u8, elem);
                    return p as *mut u8;
                }
                if (*s).bump < (*s).nelems {
                    let p = ((*s).start + (*s).bump as usize * elem) as *mut u8;
                    (*s).bump += 1;
                    if !(*s).zeroed {
                        zero(p, elem);
                    }
                    return p;
                }
            }
            let part = PARTIAL[kind][cls];
            if part != 0 {
                PARTIAL[kind][cls] = (*SPANS.add(part as usize - 1)).next;
                CUR[kind][cls] = part;
                continue;
            }
            let pages = span_pages(elem);
            let nelems = (pages << PAGE_SHIFT) / elem;
            let idx = new_span(pages, K_SMALL, cls, elem, nelems, kind == 1 || kind == 2, kind >= 2);
            CUR[kind][cls] = idx + 1;
        }
    }
}

unsafe fn alloc_large(size: usize, noscan: bool, manual: bool) -> *mut u8 {
    unsafe {
        let pages = (size + PAGE - 1) >> PAGE_SHIFT;
        let idx = new_span(pages, K_LARGE, 0, pages << PAGE_SHIFT, 1, noscan, manual);
        let s = SPANS.add(idx as usize);
        if !(*s).zeroed {
            memset((*s).start as *mut u8, 0, pages << PAGE_SHIFT);
        }
        (*s).start as *mut u8
    }
}

/// GC үйіндісінен нөлденген жад. `noscan` — ішінде көрсеткіштер жоқ.
pub fn alloc(size: usize, noscan: bool) -> *mut u8 {
    unsafe {
        let size = if size == 0 { 8 } else { size };
        if size > MAX_SMALL {
            let bytes = (size + PAGE - 1) & !(PAGE - 1);
            ALLOC_SINCE += bytes;
            if ALLOC_SINCE >= THRESHOLD && GC_ON {
                tlp_gc_collect();
            }
            return alloc_large(size, noscan, false);
        }
        let cls = class_for(size);
        ALLOC_SINCE += SIZES[cls];
        if ALLOC_SINCE >= THRESHOLD && GC_ON {
            tlp_gc_collect();
        }
        alloc_small(noscan as usize, cls)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_alloc(size: usize, flags: usize) -> *mut u8 {
    alloc(size, flags & 1 != 0)
}

// ------------------------------------------------------------------ қолмен басқару

pub fn mem_alloc(size: usize) -> *mut u8 {
    mem_alloc_ex(size, false)
}

/// `scan` — блоктың ішінде GC үйіндісіне сілтемелер болуы мүмкін (GC оны түбір ретінде қарайды).
pub fn mem_alloc_ex(size: usize, scan: bool) -> *mut u8 {
    unsafe {
        let size = if size == 0 { 8 } else { size };
        if size > MAX_SMALL {
            return alloc_large(size, !scan, true);
        }
        let cls = class_for(size);
        let k = scan as usize;
        if MFREE[k][cls] != 0 {
            let p = MFREE[k][cls] as *mut usize;
            MFREE[k][cls] = *p;
            zero(p as *mut u8, SIZES[cls]);
            return p as *mut u8;
        }
        alloc_small(2 + k, cls)
    }
}

pub fn mem_free(p: *mut u8) {
    unsafe {
        if p.is_null() {
            return;
        }
        let a = p as usize;
        if a < HEAP_START || a >= HEAP_TOP {
            fatal("`босат`: көрсеткіш `бөл` арқылы алынбаған".as_bytes());
        }
        let e = *PAGE_TABLE.add((a - HEAP_START) >> PAGE_SHIFT);
        if e == 0 {
            fatal("`босат`: көрсеткіш `бөл` арқылы алынбаған".as_bytes());
        }
        let s = SPANS.add(e as usize - 1);
        if !(*s).manual {
            fatal("`босат`: бұл объектті GC басқарады, қолмен босатуға болмайды".as_bytes());
        }
        if (*s).kind == K_LARGE {
            if a != (*s).start {
                fatal("`босат`: көрсеткіш блоктың басын көрсетпейді".as_bytes());
            }
            release_span(e - 1);
        } else {
            let off = a - (*s).start;
            if off % (*s).elem != 0 {
                fatal("`босат`: көрсеткіш блоктың басын көрсетпейді".as_bytes());
            }
            let cls = (*s).cls as usize;
            let k = (!(*s).noscan) as usize;
            if k == 1 {
                zero(a as *mut u8, (*s).elem); // ескі сілтемелер GC-ні ұстап тұрмасын
            }
            *(a as *mut usize) = MFREE[k][cls];
            MFREE[k][cls] = a;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_mem_alloc(size: usize, flags: usize) -> *mut u8 {
    mem_alloc_ex(size, flags & 1 != 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_mem_free(p: *mut u8) {
    mem_free(p)
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_heap_bytes() -> usize {
    unsafe { HEAP_TOP - HEAP_START - (FREE_PAGES << PAGE_SHIFT) }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_gc_count() -> usize {
    unsafe { GC_COUNT }
}

// ------------------------------------------------------------------ жинау

global_asm!(
    ".globl tlp_gc_collect",
    "tlp_gc_collect:",
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "sub rsp, 8",
    "mov rdi, rsp",
    "call tlp_gc_collect_impl",
    "add rsp, 8",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "ret",
);

unsafe extern "C" {
    pub fn tlp_gc_collect();
}

#[inline(always)]
unsafe fn mark_ptr(v: usize) {
    unsafe {
        if v < HEAP_START || v >= HEAP_TOP {
            return;
        }
        let e = *PAGE_TABLE.add((v - HEAP_START) >> PAGE_SHIFT);
        if e == 0 {
            return;
        }
        let s = SPANS.add(e as usize - 1);
        if (*s).manual {
            return;
        }
        let (start, size);
        if (*s).kind == K_SMALL {
            let idx = ((v - (*s).start) as u32) / ((*s).elem as u32);
            if idx >= (*s).bump {
                return;
            }
            let w = (idx >> 6) as usize;
            let bit = 1u64 << (idx & 63);
            if (*s).marks[w] & bit != 0 {
                return;
            }
            (*s).marks[w] |= bit;
            start = (*s).start + idx as usize * (*s).elem;
            size = (*s).elem;
        } else {
            if (*s).marked_large {
                return;
            }
            (*s).marked_large = true;
            start = (*s).start;
            size = (*s).elem;
        }
        if !(*s).noscan {
            if MARK_SP + 2 > MARK_CAP {
                fatal("GC белгілеу стегі толып кетті".as_bytes());
            }
            *MARK_STACK.add(MARK_SP) = start;
            *MARK_STACK.add(MARK_SP + 1) = size;
            MARK_SP += 2;
        }
    }
}

unsafe fn mark_range(lo: usize, hi: usize) {
    unsafe {
        let mut p = (lo + 7) & !7;
        while p + 8 <= hi {
            mark_ptr(*(p as *const usize));
            p += 8;
        }
    }
}

unsafe fn drain() {
    unsafe {
        while MARK_SP > 0 {
            MARK_SP -= 2;
            let start = *MARK_STACK.add(MARK_SP);
            let size = *MARK_STACK.add(MARK_SP + 1);
            mark_range(start, start + size);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_gc_collect_impl(sp: usize) {
    unsafe {
        GC_COUNT += 1;
        MARK_SP = 0;
        mark_range(sp, STACK_TOP);
        drain();
        mark_range(GLOBALS_LO, GLOBALS_HI);
        drain();
        // қолмен блоктар (GC сілтемелері бар) — түбірлер
        for i in 0..SPAN_COUNT {
            let s = SPANS.add(i as usize);
            if (*s).kind != K_FREE && (*s).manual && !(*s).noscan {
                let used = if (*s).kind == K_LARGE { (*s).elem } else { (*s).bump as usize * (*s).elem };
                mark_range((*s).start, (*s).start + used);
            }
        }
        drain();

        // себу
        for k in 0..2 {
            for c in 0..NCLASS {
                CUR[k][c] = 0;
                PARTIAL[k][c] = 0;
            }
        }
        let mut live_bytes = 0usize;
        let n = SPAN_COUNT;
        for i in 0..n {
            let s = SPANS.add(i as usize);
            if (*s).kind == K_FREE || (*s).manual {
                continue;
            }
            if (*s).kind == K_LARGE {
                if (*s).marked_large {
                    (*s).marked_large = false;
                    live_bytes += (*s).elem;
                } else {
                    release_span(i);
                }
                continue;
            }
            let elem = (*s).elem;
            let bump = (*s).bump as usize;
            let mut live = 0usize;
            let mut head = 0usize;
            let mut idx = bump;
            while idx > 0 {
                idx -= 1;
                if (*s).marks[idx >> 6] >> (idx & 63) & 1 != 0 {
                    live += 1;
                } else {
                    let p = (*s).start + idx * elem;
                    *(p as *mut usize) = head;
                    head = p;
                }
            }
            for w in 0..(((bump + 63) >> 6).min(16)) {
                (*s).marks[w] = 0;
            }
            if live == 0 {
                release_span(i);
                continue;
            }
            (*s).free = head;
            live_bytes += live * elem;
            if head != 0 || (*s).bump < (*s).nelems {
                let kind = (*s).noscan as usize;
                (*s).next = PARTIAL[kind][(*s).cls as usize];
                PARTIAL[kind][(*s).cls as usize] = i + 1;
            }
        }
        LIVE_BYTES = live_bytes;
        ALLOC_SINCE = 0;
        THRESHOLD = if live_bytes > MIN_THRESHOLD { live_bytes } else { MIN_THRESHOLD };
    }
}
