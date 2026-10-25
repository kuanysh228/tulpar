//! Тізімдер: [ұзындық][сыйымдылық][деректер көрсеткіші][жалаулар]; элемент — 8 байт
//! (`логика`/`байт` тізімдері үшін 1 байт). Жалаулар: бит0 — noscan, бит1 — байттық элементтер.

use crate::gc;
use crate::str::*;
use crate::sys::*;

pub type List = *mut u64;

pub unsafe fn list_new(cap: usize, flags: i64) -> List {
    let l = gc::alloc(32, false) as *mut u64;
    let cap = if cap < 4 { 4 } else { cap };
    let esz = if flags & 2 != 0 { 1 } else { 8 };
    let d = gc::alloc(cap * esz, flags & 1 != 0) as u64;
    *l.add(1) = cap as u64;
    *l.add(2) = d;
    *l.add(3) = flags as u64;
    l
}

#[inline(always)]
pub unsafe fn len(l: List) -> usize {
    *l as usize
}
#[inline(always)]
pub unsafe fn data(l: List) -> *mut u64 {
    *l.add(2) as *mut u64
}
#[inline(always)]
pub unsafe fn esz(l: List) -> usize {
    if *l.add(3) & 2 != 0 { 1 } else { 8 }
}
#[inline(always)]
pub unsafe fn get(l: List, i: usize) -> u64 {
    if esz(l) == 1 { *(data(l) as *const u8).add(i) as u64 } else { *data(l).add(i) }
}
#[inline(always)]
pub unsafe fn set(l: List, i: usize, v: u64) {
    if esz(l) == 1 { *(data(l) as *mut u8).add(i) = v as u8 } else { *data(l).add(i) = v }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_new_n(n: i64, flags: i64) -> List {
    let l = list_new(n as usize, flags);
    *l = n as u64;
    l
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_push(l: List, v: u64) {
    let n = *l;
    let cap = *l.add(1);
    if n == cap {
        let ncap = cap * 2;
        let e = esz(l);
        let nd = gc::alloc(ncap as usize * e, *l.add(3) & 1 != 0) as *mut u64;
        memcpy(nd as *mut u8, data(l) as *const u8, n as usize * e);
        *l.add(2) = nd as u64;
        *l.add(1) = ncap;
    }
    set(l, n as usize, v);
    *l = n + 1;
}

pub unsafe fn list_push(l: List, v: u64) {
    tlp_list_push(l, v)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_pop(l: List) -> u64 {
    let n = *l;
    if n == 0 {
        fatal("`ал`: тізім бос".as_bytes());
    }
    let v = get(l, n as usize - 1);
    set(l, n as usize - 1, 0);
    *l = n - 1;
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_clear(l: List) {
    let n = *l as usize;
    memset(data(l) as *mut u8, 0, n * esz(l));
    *l = 0;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_reverse(l: List) {
    let n = *l as usize;
    let mut i = 0;
    while i < n / 2 {
        let t = get(l, i);
        set(l, i, get(l, n - 1 - i));
        set(l, n - 1 - i, t);
        i += 1;
    }
}

unsafe fn merge_sort<F: Fn(u64, u64) -> bool>(d: *mut u64, n: usize, less: F) {
    if n < 2 {
        return;
    }
    // кіші бөліктер: insertion sort
    const RUN: usize = 16;
    let mut s = 0;
    while s < n {
        let e = (s + RUN).min(n);
        let mut i = s + 1;
        while i < e {
            let x = *d.add(i);
            let mut j = i;
            while j > s && less(x, *d.add(j - 1)) {
                *d.add(j) = *d.add(j - 1);
                j -= 1;
            }
            *d.add(j) = x;
            i += 1;
        }
        s = e;
    }
    if n <= RUN {
        return;
    }
    let tmp = gc::mem_alloc(n * 8) as *mut u64;
    let mut src = d;
    let mut dst = tmp;
    let mut width = RUN;
    while width < n {
        let mut lo = 0;
        while lo < n {
            let mid = (lo + width).min(n);
            let hi = (lo + 2 * width).min(n);
            let (mut i, mut j, mut k) = (lo, mid, lo);
            while i < mid && j < hi {
                if less(*src.add(j), *src.add(i)) {
                    *dst.add(k) = *src.add(j);
                    j += 1;
                } else {
                    *dst.add(k) = *src.add(i);
                    i += 1;
                }
                k += 1;
            }
            while i < mid {
                *dst.add(k) = *src.add(i);
                i += 1;
                k += 1;
            }
            while j < hi {
                *dst.add(k) = *src.add(j);
                j += 1;
                k += 1;
            }
            lo = hi;
        }
        core::mem::swap(&mut src, &mut dst);
        width *= 2;
    }
    if src != d {
        memcpy(d as *mut u8, src as *const u8, n * 8);
    }
    gc::mem_free(tmp as *mut u8);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_sort(l: List, kind: i64) {
    let n = *l as usize;
    let d = data(l);
    if esz(l) == 1 {
        // санау сұрыптауы
        let mut cnt = [0usize; 256];
        let b = d as *mut u8;
        for i in 0..n {
            cnt[*b.add(i) as usize] += 1;
        }
        let mut k = 0;
        for v in 0..256 {
            for _ in 0..cnt[v] {
                *b.add(k) = v as u8;
                k += 1;
            }
        }
        return;
    }
    match kind {
        0 => merge_sort(d, n, |a, b| (a as i64) < (b as i64)),
        1 => merge_sort(d, n, |a, b| f64::from_bits(a) < f64::from_bits(b)),
        _ => merge_sort(d, n, |a, b| tlp_str_cmp(a as Str, b as Str) < 0),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_concat(a: List, b: List) -> List {
    let (na, nb) = (*a as usize, *b as usize);
    let e = esz(a);
    let r = list_new(na + nb, *a.add(3) as i64);
    memcpy(data(r) as *mut u8, data(a) as *const u8, na * e);
    memcpy((data(r) as *mut u8).add(na * e), data(b) as *const u8, nb * e);
    *r = (na + nb) as u64;
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_repeat(a: List, n: i64) -> List {
    let na = *a as usize;
    let times = if n < 0 { 0 } else { n as usize };
    let total = match na.checked_mul(times) {
        Some(t) if t < (1 << 36) => t,
        _ => fatal("тізімді қайталау тым үлкен".as_bytes()),
    };
    let e = esz(a);
    let r = list_new(total, *a.add(3) as i64);
    if e == 1 && na == 1 {
        memset(data(r) as *mut u8, *(data(a) as *const u8) as i32, times);
    } else {
        for i in 0..times {
            memcpy((data(r) as *mut u8).add(i * na * e), data(a) as *const u8, na * e);
        }
    }
    *r = total as u64;
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_slice(a: List, lo: i64, hi: i64) -> List {
    let (l, h) = norm_slice(*a as i64, lo, hi);
    let e = esz(a);
    let r = list_new(h - l, *a.add(3) as i64);
    memcpy(data(r) as *mut u8, (data(a) as *const u8).add(l * e), (h - l) * e);
    *r = (h - l) as u64;
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_contains(l: List, v: u64, kind: i64) -> i64 {
    let n = *l as usize;
    for i in 0..n {
        let x = get(l, i);
        let eq = match kind {
            0 => x == v,
            1 => tlp_str_eq(x as Str, v as Str) != 0,
            _ => f64::from_bits(x) == f64::from_bits(v),
        };
        if eq {
            return 1;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_panic_index(i: i64, len: i64) -> ! {
    let mut sb = crate::fmt::Sb::new();
    sb.push_bytes("индекс шектен тыс: ".as_bytes());
    let mut b = [0u8; 24];
    sb.push_bytes(fmt_i64(i, &mut b));
    sb.push_bytes(" (ұзындығы ".as_bytes());
    sb.push_bytes(fmt_i64(len, &mut b));
    sb.push_bytes(")".as_bytes());
    fatal(sb.bytes())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_index(l: List, v: u64, kind: i64) -> i64 {
    let n = *l as usize;
    for i in 0..n {
        let x = get(l, i);
        let eq = match kind {
            0 => x == v,
            1 => tlp_str_eq(x as Str, v as Str) != 0,
            _ => f64::from_bits(x) == f64::from_bits(v),
        };
        if eq {
            return i as i64;
        }
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_remove(l: List, i: i64) -> u64 {
    let n = *l as i64;
    let idx = if i < 0 { i + n } else { i };
    if idx < 0 || idx >= n {
        tlp_panic_index(i, n);
    }
    let e = esz(l);
    let v = get(l, idx as usize);
    let base = data(l) as *mut u8;
    memmove(base.add(idx as usize * e), base.add((idx as usize + 1) * e), (n as usize - idx as usize - 1) * e);
    set(l, n as usize - 1, 0);
    *l = (n - 1) as u64;
    v
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_insert(l: List, i: i64, v: u64) {
    let n = *l as i64;
    let idx = if i < 0 { (i + n).max(0) } else { i.min(n) };
    tlp_list_push(l, 0);
    let e = esz(l);
    let base = data(l) as *mut u8;
    memmove(base.add((idx as usize + 1) * e), base.add(idx as usize * e), (n as usize - idx as usize) * e);
    set(l, idx as usize, v);
}

/// 0 — қосынды, 1 — мин, 2 — макс (бүтін)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_fold_i(l: List, op: i64) -> i64 {
    let n = *l as usize;
    if op != 0 && n == 0 {
        fatal("`мин`/`макс`: тізім бос".as_bytes());
    }
    let d = data(l) as *const i64;
    let mut acc = if op == 0 { 0 } else { *d };
    for i in 0..n {
        let x = *d.add(i);
        acc = match op {
            0 => acc.wrapping_add(x),
            1 => if x < acc { x } else { acc },
            _ => if x > acc { x } else { acc },
        };
    }
    acc
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_fold_f(l: List, op: i64) -> f64 {
    let n = *l as usize;
    if op != 0 && n == 0 {
        fatal("`мин`/`макс`: тізім бос".as_bytes());
    }
    let d = data(l) as *const f64;
    let mut acc = if op == 0 { 0.0 } else { *d };
    for i in 0..n {
        let x = *d.add(i);
        acc = match op {
            0 => acc + x,
            1 => if x < acc { x } else { acc },
            _ => if x > acc { x } else { acc },
        };
    }
    acc
}

/// Тізімді `keys` тізімі бойынша тұрақты сұрыптайды (kind: 0 бүтін, 1 нақты, 2 мәтін).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_list_sort_by(l: List, keys: List, kind: i64) {
    let n = *l as usize;
    if n < 2 {
        return;
    }
    let idx = gc::mem_alloc(n * 8) as *mut u64;
    for i in 0..n {
        *idx.add(i) = i as u64;
    }
    let kd = data(keys);
    match kind {
        0 => merge_sort(idx, n, |a, b| (*kd.add(a as usize) as i64) < (*kd.add(b as usize) as i64)),
        1 => merge_sort(idx, n, |a, b| f64::from_bits(*kd.add(a as usize)) < f64::from_bits(*kd.add(b as usize))),
        _ => merge_sort(idx, n, |a, b| tlp_str_cmp(*kd.add(a as usize) as Str, *kd.add(b as usize) as Str) < 0),
    }
    let e = esz(l);
    let tmp = gc::mem_alloc(n * e) as *mut u8;
    memcpy(tmp, data(l) as *const u8, n * e);
    let base = data(l) as *mut u8;
    for k in 0..n {
        let src = *idx.add(k) as usize;
        memcpy(base.add(k * e), tmp.add(src * e), e);
    }
    gc::mem_free(tmp);
    gc::mem_free(idx as *mut u8);
}
