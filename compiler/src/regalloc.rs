//! Сызықтық сканерлеу регистр бөлгіші (Poletto & Sarkar) + циклдің тереңдігі бойынша салмақтар.

use crate::ir::*;

pub const RAX: u8 = 0;
pub const RCX: u8 = 1;
pub const RDX: u8 = 2;
pub const RBX: u8 = 3;
pub const RSP: u8 = 4;
pub const RBP: u8 = 5;
pub const RSI: u8 = 6;
pub const RDI: u8 = 7;
pub const R8: u8 = 8;
pub const R9: u8 = 9;
pub const R10: u8 = 10;
pub const R11: u8 = 11;
pub const R12: u8 = 12;
pub const R13: u8 = 13;
pub const R14: u8 = 14;
pub const R15: u8 = 15;

pub const CALLEE_SAVED: [u8; 5] = [RBX, R12, R13, R14, R15];
const INT_CALLER: [u8; 4] = [RSI, RDI, R8, R9];
const FLT_REGS: [u8; 12] = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loc {
    None,
    Reg(u8),
    /// rbp-ға қатысты ығысу
    Stack(i32),
}

pub struct Alloc {
    pub locs: Vec<Loc>,
    pub frame: u32,
    pub saved: Vec<u8>,
    pub order: Vec<BlockId>,
}

struct Bits {
    w: Vec<u64>,
}
impl Bits {
    fn new(n: usize) -> Bits {
        Bits { w: vec![0; (n + 63) / 64] }
    }
    fn set(&mut self, i: usize) {
        self.w[i / 64] |= 1 << (i % 64);
    }
    fn clear(&mut self, i: usize) {
        self.w[i / 64] &= !(1 << (i % 64));
    }
    fn get(&self, i: usize) -> bool {
        self.w[i / 64] >> (i % 64) & 1 == 1
    }
}

#[derive(Clone)]
struct Interval {
    v: u32,
    start: u32,
    end: u32,
    weight: f64,
    crosses_call: bool,
    class: RC,
    hint: Option<u32>,
    /// тікелей регистр ұсынысы (шақыру аргументі / параметр)
    rhint: Option<u8>,
}

pub fn allocate(f: &IrFunc, spill_all: bool) -> Alloc {
    let nv = f.classes.len();
    let order = f.layout();
    let nb = f.blocks.len();

    // позициялар
    let mut bstart = vec![0u32; nb];
    let mut bend = vec![0u32; nb];
    let mut pos = 0u32;
    for &b in &order {
        bstart[b as usize] = pos;
        pos += f.blocks[b as usize].insts.len() as u32 + 1;
        bend[b as usize] = pos - 1;
    }
    let total = pos;

    // use/def
    let mut usep: Vec<Bits> = (0..nb).map(|_| Bits::new(nv)).collect();
    let mut defp: Vec<Bits> = (0..nb).map(|_| Bits::new(nv)).collect();
    for &b in &order {
        let bl = &f.blocks[b as usize];
        let (u, d) = (&mut usep[b as usize], &mut defp[b as usize]);
        for ins in &bl.insts {
            ins.uses(|v| {
                if !d.get(v as usize) {
                    u.set(v as usize);
                }
            });
            if let Some(v) = ins.def() {
                d.set(v as usize);
            }
        }
        bl.term.uses(|v| {
            if !d.get(v as usize) {
                u.set(v as usize);
            }
        });
    }
    let mut live_in: Vec<Bits> = (0..nb).map(|_| Bits::new(nv)).collect();
    let mut live_out: Vec<Bits> = (0..nb).map(|_| Bits::new(nv)).collect();
    let words = (nv + 63) / 64;
    loop {
        let mut changed = false;
        for &b in order.iter().rev() {
            let bi = b as usize;
            let mut out = vec![0u64; words];
            for s in f.blocks[bi].term.succs() {
                for w in 0..words {
                    out[w] |= live_in[s as usize].w[w];
                }
            }
            let mut inn = vec![0u64; words];
            for w in 0..words {
                inn[w] = usep[bi].w[w] | (out[w] & !defp[bi].w[w]);
            }
            if out != live_out[bi].w || inn != live_in[bi].w {
                live_out[bi].w = out;
                live_in[bi].w = inn;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // цикл тереңдігі
    let mut depth = vec![0u32; total as usize + 1];
    for &b in &order {
        for s in f.blocks[b as usize].term.succs() {
            if bstart[s as usize] <= bstart[b as usize] {
                // артқа секіру: [s.start, b.end] цикл
                for p in bstart[s as usize]..=bend[b as usize] {
                    depth[p as usize] += 1;
                }
            }
        }
    }

    // интервалдар
    let mut start = vec![u32::MAX; nv];
    let mut end = vec![0u32; nv];
    let mut weight = vec![0f64; nv];
    let mut hint: Vec<Option<u32>> = vec![None; nv];
    let mut rhint: Vec<Option<u8>> = vec![None; nv];
    let mut call_args: Vec<(u32, VReg, u8, RC)> = Vec::new(); // (позиция, vreg, аргумент индексі (класс ішінде), класс)
    let mut calls: Vec<u32> = Vec::new();
    let touch = |v: usize, p: u32, start: &mut Vec<u32>, end: &mut Vec<u32>| {
        if p < start[v] {
            start[v] = p;
        }
        if p > end[v] {
            end[v] = p;
        }
    };
    for &b in &order {
        let bl = &f.blocks[b as usize];
        let mut p = bstart[b as usize];
        for ins in &bl.insts {
            let w = 10f64.powi(depth[p as usize].min(6) as i32);
            ins.uses(|v| {
                touch(v as usize, p, &mut start, &mut end);
                weight[v as usize] += w;
            });
            if let Some(d) = ins.def() {
                touch(d as usize, p, &mut start, &mut end);
                weight[d as usize] += w;
                if let Inst::Mov(_, s) = ins {
                    hint[d as usize] = Some(*s);
                }
            }
            match ins {
                Inst::Call { args, .. } | Inst::CallInd { args, .. } => {
                    calls.push(p);
                    let (mut ni, mut nf) = (0u8, 0u8);
                    for &a in args {
                        if f.classes[a as usize] == RC::F {
                            call_args.push((p, a, nf, RC::F));
                            nf += 1;
                        } else {
                            call_args.push((p, a, ni, RC::I));
                            ni += 1;
                        }
                    }
                }
                Inst::Param(d, ArgLoc::IReg(k)) => {
                    let r = [RDI, RSI, RDX, RCX, R8, R9][*k as usize];
                    if INT_CALLER.contains(&r) {
                        rhint[*d as usize] = Some(r);
                    }
                }
                Inst::Param(d, ArgLoc::FReg(k)) => {
                    if FLT_REGS.contains(k) {
                        rhint[*d as usize] = Some(*k);
                    }
                }
                _ => {}
            }
            p += 1;
        }
        let w = 10f64.powi(depth[p as usize].min(6) as i32);
        bl.term.uses(|v| {
            touch(v as usize, p, &mut start, &mut end);
            weight[v as usize] += w;
        });
        for v in 0..nv {
            if live_in[b as usize].get(v) {
                touch(v, bstart[b as usize], &mut start, &mut end);
            }
            if live_out[b as usize].get(v) {
                touch(v, bend[b as usize], &mut start, &mut end);
            }
        }
    }

    for &(p, v, k, rc) in &call_args {
        if end[v as usize] == p && rhint[v as usize].is_none() {
            if rc == RC::I && k < 6 {
                let r = [RDI, RSI, RDX, RCX, R8, R9][k as usize];
                if INT_CALLER.contains(&r) {
                    rhint[v as usize] = Some(r);
                }
            } else if rc == RC::F && k < 8 && FLT_REGS.contains(&k) {
                rhint[v as usize] = Some(k);
            }
        }
    }
    let mut intervals: Vec<Interval> = Vec::new();
    for v in 0..nv {
        if start[v] == u32::MAX {
            continue;
        }
        let (s, e) = (start[v], end[v]);
        // s < c < e болатын шақыру бар ма
        let idx = calls.partition_point(|&c| c <= s);
        let crosses = idx < calls.len() && calls[idx] < e;
        intervals.push(Interval { v: v as u32, start: s, end: e, weight: weight[v], crosses_call: crosses, class: f.classes[v], hint: hint[v], rhint: rhint[v] });
    }
    intervals.sort_by_key(|i| (i.start, i.v));

    let mut locs = vec![Loc::None; nv];
    let mut spilled: Vec<u32> = Vec::new();
    let mut is_spilled = vec![false; nv];

    if spill_all {
        for iv in &intervals {
            spilled.push(iv.v);
            is_spilled[iv.v as usize] = true;
        }
    } else {
        let mut active: Vec<Interval> = Vec::new();
        let mut free_i: Vec<u8> = Vec::new();
        for r in CALLEE_SAVED.iter().rev().chain(INT_CALLER.iter().rev()) {
            free_i.push(*r);
        }
        let mut free_f: Vec<u8> = FLT_REGS.iter().rev().cloned().collect();
        for iv in intervals {
            // мерзімі біткендерді босату
            let mut k = 0;
            while k < active.len() {
                if active[k].end <= iv.start {
                    let a = active.swap_remove(k);
                    if let Loc::Reg(r) = locs[a.v as usize] {
                        if a.class == RC::I {
                            free_i.push(r);
                        } else {
                            free_f.push(r);
                        }
                    }
                } else {
                    k += 1;
                }
            }
            let is_callee = |r: u8| CALLEE_SAVED.contains(&r);
            let ok_reg = |r: u8, iv: &Interval| -> bool { iv.class == RC::F && !iv.crosses_call || iv.class == RC::I && (!iv.crosses_call || is_callee(r)) };
            // тауып аламыз
            let pool = if iv.class == RC::I { &mut free_i } else { &mut free_f };
            let mut chosen: Option<usize> = None;
            if let Some(hr) = iv.rhint {
                if let Some(p) = pool.iter().position(|&r| r == hr) {
                    if ok_reg(hr, &iv) {
                        chosen = Some(p);
                    }
                }
            }
            if let (None, Some(h)) = (chosen, iv.hint) {
                if let Loc::Reg(hr) = locs[h as usize] {
                    if let Some(p) = pool.iter().position(|&r| r == hr) {
                        if ok_reg(hr, &iv) {
                            chosen = Some(p);
                        }
                    }
                }
            }
            if chosen.is_none() {
                // соңынан іздейміз: алдымен caller-saved (олар тізімнің соңында)
                for p in (0..pool.len()).rev() {
                    if ok_reg(pool[p], &iv) {
                        chosen = Some(p);
                        break;
                    }
                }
            }
            if let Some(p) = chosen {
                let r = pool.remove(p);
                locs[iv.v as usize] = Loc::Reg(r);
                active.push(iv);
                continue;
            }
            // регистр жоқ: ең арзан кандидатты шығарамыз
            let density = |i: &Interval| i.weight / ((i.end - i.start + 1) as f64);
            let mut victim: Option<usize> = None;
            for (k, a) in active.iter().enumerate() {
                if a.class != iv.class {
                    continue;
                }
                let r = match locs[a.v as usize] {
                    Loc::Reg(r) => r,
                    _ => continue,
                };
                if !ok_reg(r, &iv) {
                    continue;
                }
                if density(a) < density(&iv) && victim.map_or(true, |vk| density(a) < density(&active[vk])) {
                    victim = Some(k);
                }
            }
            match victim {
                Some(k) => {
                    let a = active.swap_remove(k);
                    let r = match locs[a.v as usize] {
                        Loc::Reg(r) => r,
                        _ => unreachable!(),
                    };
                    spilled.push(a.v);
                    is_spilled[a.v as usize] = true;
                    locs[a.v as usize] = Loc::Reg(r); // уақытша; төменде қайта жазылады
                    locs[iv.v as usize] = Loc::Reg(r);
                    active.push(iv);
                }
                None => {
                    spilled.push(iv.v);
                    is_spilled[iv.v as usize] = true;
                }
            }
        }
    }

    // қолданылған callee-saved
    let mut saved = Vec::new();
    for &r in &CALLEE_SAVED {
        if locs.iter().enumerate().any(|(v, l)| *l == Loc::Reg(r) && !is_spilled[v]) {
            saved.push(r);
        }
    }
    // қашқан интервалдарға слот
    let mut slot = saved.len() as i32;
    for v in spilled {
        slot += 1;
        locs[v as usize] = Loc::Stack(-8 * slot);
    }
    let bytes = (slot as u32) * 8;
    let frame = (bytes + 15) & !15;
    Alloc { locs, frame, saved, order }
}
