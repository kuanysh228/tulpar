//! Сөздіктер: енгізу ретін сақтайтын ықшам хеш-кесте (entries + index).
//!
//! Нысан: [0] len, [1] used, [2] ecap, [3] entries*, [4] icap, [5] index*, [6] kind (0 — бүтін, 1 — мәтін), [7] noscan
//! Entry: (hash, key, val), өшірілген: hash == DELETED.

use crate::gc;
use crate::list;
use crate::str::*;
use crate::sys::*;

pub type Dict = *mut u64;

const DELETED: u64 = u64::MAX;
const TOMB: u32 = u32::MAX;

#[inline(always)]
unsafe fn entries(d: Dict) -> *mut u64 {
    *d.add(3) as *mut u64
}
#[inline(always)]
unsafe fn index(d: Dict) -> *mut u32 {
    *d.add(5) as *mut u32
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

unsafe fn hash_of(d: Dict, k: u64) -> u64 {
    let h = if *d.add(6) == 0 {
        mix(k)
    } else {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes(k as *const u64) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        mix(h)
    };
    h & 0x7fff_ffff_ffff_ffff
}

#[inline(always)]
unsafe fn key_eq(d: Dict, a: u64, b: u64) -> bool {
    if *d.add(6) == 0 {
        a == b
    } else {
        tlp_str_eq(a as Str, b as Str) != 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_new(kind: i64, flags: i64) -> Dict {
    let d = gc::alloc(64, false) as *mut u64;
    let noscan = flags & 1 != 0;
    let e = gc::alloc(8 * 24, noscan) as u64;
    let ix = gc::alloc(16 * 4, true) as u64;
    *d.add(2) = 8;
    *d.add(3) = e;
    *d.add(4) = 16;
    *d.add(5) = ix;
    *d.add(6) = kind as u64;
    *d.add(7) = noscan as u64;
    d
}

/// (entry индексі немесе usize::MAX, енгізуге арналған index слоты)
unsafe fn lookup(d: Dict, k: u64, h: u64) -> (usize, usize) {
    let mask = *d.add(4) as usize - 1;
    let ix = index(d);
    let en = entries(d);
    let mut i = h as usize & mask;
    let mut first_tomb = usize::MAX;
    loop {
        let e = *ix.add(i);
        if e == 0 {
            return (usize::MAX, if first_tomb != usize::MAX { first_tomb } else { i });
        }
        if e == TOMB {
            if first_tomb == usize::MAX {
                first_tomb = i;
            }
        } else {
            let ent = en.add((e as usize - 1) * 3);
            if *ent == h && key_eq(d, *ent.add(1), k) {
                return (e as usize - 1, i);
            }
        }
        i = (i + 1) & mask;
    }
}

unsafe fn rebuild(d: Dict) {
    let len = *d as usize;
    let used = *d.add(1) as usize;
    let mut ecap = 8usize;
    while ecap < len * 2 {
        ecap *= 2;
    }
    let mut icap = 16usize;
    while icap < ecap * 2 {
        icap *= 2;
    }
    let noscan = *d.add(7) != 0;
    let ne = gc::alloc(ecap * 24, noscan) as *mut u64;
    let nix = gc::alloc(icap * 4, true) as *mut u32;
    let oe = entries(d);
    let mask = icap - 1;
    let mut n = 0usize;
    for i in 0..used {
        let ent = oe.add(i * 3);
        if *ent == DELETED {
            continue;
        }
        let dst = ne.add(n * 3);
        *dst = *ent;
        *dst.add(1) = *ent.add(1);
        *dst.add(2) = *ent.add(2);
        let mut s = *ent as usize & mask;
        while *nix.add(s) != 0 {
            s = (s + 1) & mask;
        }
        *nix.add(s) = n as u32 + 1;
        n += 1;
    }
    *d.add(1) = n as u64;
    *d.add(2) = ecap as u64;
    *d.add(3) = ne as u64;
    *d.add(4) = icap as u64;
    *d.add(5) = nix as u64;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_set(d: Dict, k: u64, v: u64) {
    let h = hash_of(d, k);
    let (found, slot) = lookup(d, k, h);
    if found != usize::MAX {
        *entries(d).add(found * 3 + 2) = v;
        return;
    }
    let used = *d.add(1);
    if used == *d.add(2) || (used + 1) * 3 > *d.add(4) * 2 {
        rebuild(d);
        let (_, slot2) = lookup(d, k, h);
        append(d, h, k, v, slot2);
    } else {
        append(d, h, k, v, slot);
    }
}

unsafe fn append(d: Dict, h: u64, k: u64, v: u64, slot: usize) {
    let used = *d.add(1) as usize;
    let ent = entries(d).add(used * 3);
    *ent = h;
    *ent.add(1) = k;
    *ent.add(2) = v;
    *index(d).add(slot) = used as u32 + 1;
    *d.add(1) = used as u64 + 1;
    *d += 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_get(d: Dict, k: u64) -> u64 {
    let h = hash_of(d, k);
    let (found, _) = lookup(d, k, h);
    if found == usize::MAX {
        let mut sb = crate::fmt::Sb::new();
        sb.push_bytes("сөздікте кілт табылмады: ".as_bytes());
        if *d.add(6) == 0 {
            let mut b = [0u8; 24];
            sb.push_bytes(fmt_i64(k as i64, &mut b));
        } else {
            sb.push_bytes(b"'");
            sb.push_bytes(bytes(k as *const u64));
            sb.push_bytes(b"'");
        }
        fatal(sb.bytes());
    }
    *entries(d).add(found * 3 + 2)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_get_or(d: Dict, k: u64, def: u64) -> u64 {
    let h = hash_of(d, k);
    let (found, _) = lookup(d, k, h);
    if found == usize::MAX {
        def
    } else {
        *entries(d).add(found * 3 + 2)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_has(d: Dict, k: u64) -> i64 {
    let h = hash_of(d, k);
    (lookup(d, k, h).0 != usize::MAX) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_del(d: Dict, k: u64) {
    let h = hash_of(d, k);
    let (found, slot) = lookup(d, k, h);
    if found == usize::MAX {
        return;
    }
    let ent = entries(d).add(found * 3);
    *ent = DELETED;
    *ent.add(1) = 0;
    *ent.add(2) = 0;
    *index(d).add(slot) = TOMB;
    *d -= 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_len(d: Dict) -> i64 {
    *d as i64
}

/// `pos`-тан бастап келесі тірі жазбаның индексі, жоқ болса -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_next(d: Dict, pos: i64) -> i64 {
    let used = *d.add(1) as i64;
    let en = entries(d);
    let mut i = pos;
    while i < used {
        if *en.add(i as usize * 3) != DELETED {
            return i;
        }
        i += 1;
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_key_at(d: Dict, pos: i64) -> u64 {
    *entries(d).add(pos as usize * 3 + 1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_val_at(d: Dict, pos: i64) -> u64 {
    *entries(d).add(pos as usize * 3 + 2)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_dict_list(d: Dict, which: i64, flags: i64) -> list::List {
    let n = *d as usize;
    let l = list::list_new(n, flags);
    let mut pos = 0;
    loop {
        pos = tlp_dict_next(d, pos);
        if pos < 0 {
            break;
        }
        let v = if which == 0 { tlp_dict_key_at(d, pos) } else { tlp_dict_val_at(d, pos) };
        list::list_push(l, v);
        pos += 1;
    }
    l
}

/// 0 — бірігу, 1 — қиылысу, 2 — айырма
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_set_op(a: Dict, b: Dict, op: i64) -> Dict {
    let r = tlp_dict_new(*a.add(6) as i64, *a.add(7) as i64);
    let mut pos = 0;
    loop {
        pos = tlp_dict_next(a, pos);
        if pos < 0 {
            break;
        }
        let k = tlp_dict_key_at(a, pos);
        let in_b = tlp_dict_has(b, k) != 0;
        if op == 0 || (op == 1 && in_b) || (op == 2 && !in_b) {
            tlp_dict_set(r, k, 0);
        }
        pos += 1;
    }
    if op == 0 {
        let mut pos = 0;
        loop {
            pos = tlp_dict_next(b, pos);
            if pos < 0 {
                break;
            }
            tlp_dict_set(r, tlp_dict_key_at(b, pos), 0);
            pos += 1;
        }
    }
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_set_from_list(l: list::List, kind: i64, flags: i64) -> Dict {
    let d = tlp_dict_new(kind, flags);
    for i in 0..list::len(l) {
        tlp_dict_set(d, list::get(l, i), 0);
    }
    d
}
