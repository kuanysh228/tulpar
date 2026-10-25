//! IR оңтайландыру: CFG жеңілдету, тұрақтыларды енгізу, көшірмелерді тарату,
//! `Mov` біріктіру, өлі кодты жою, `test` біріктіру.

use std::collections::HashMap;

use crate::ir::*;

pub fn optimize(f: &mut IrFunc) {
    for _ in 0..4 {
        // Mov біріктіру көшірмелерді таратудан бұрын: `t = i + 1; i = t` -> `i = i + 1`
        let mut changed = coalesce_movs(f);
        changed |= if_convert(f);
        changed |= simplify_cfg(f);
        changed |= local_opts(f);
        changed |= coalesce_movs(f);
        changed |= dce(f);
        if !changed {
            break;
        }
    }
    fuse_test(f);
    simplify_cfg(f);
}

// ---------------------------------------------------------------- CFG

fn simplify_cfg(f: &mut IrFunc) -> bool {
    let mut changed = false;
    let n = f.blocks.len();
    // бос блоктар арқылы секірулерді өткізу
    let mut target: Vec<BlockId> = (0..n as u32).collect();
    for b in 1..n {
        if f.blocks[b].insts.is_empty() {
            if let Term::Jmp(x) = f.blocks[b].term {
                if x as usize != b {
                    target[b] = x;
                }
            }
        }
    }
    let resolve = |mut b: BlockId| -> BlockId {
        let mut steps = 0;
        while target[b as usize] != b && steps < 64 {
            b = target[b as usize];
            steps += 1;
        }
        b
    };
    for b in 0..n {
        let mut t = f.blocks[b].term.clone();
        t.map_blocks(|x| resolve(x));
        if let Term::Br { t: a, f: c, .. } | Term::BrI { t: a, f: c, .. } | Term::BrF { t: a, f: c, .. } | Term::BrTest { t: a, f: c, .. } = &t {
            if a == c {
                t = Term::Jmp(*a);
            }
        }
        if !same_term(&t, &f.blocks[b].term) {
            changed = true;
        }
        f.blocks[b].term = t;
    }
    // жалғыз предшественникті блоктарды біріктіру
    loop {
        let n = f.blocks.len();
        let mut preds = vec![0u32; n];
        preds[0] += 1;
        for b in 0..n {
            for s in f.blocks[b].term.succs() {
                preds[s as usize] += 1;
            }
        }
        let mut merged = false;
        for a in 0..n {
            if let Term::Jmp(b) = f.blocks[a].term {
                let b = b as usize;
                if b != a && b != 0 && preds[b] == 1 && f.blocks[a].cold == f.blocks[b].cold {
                    let insts = std::mem::take(&mut f.blocks[b].insts);
                    let term = std::mem::replace(&mut f.blocks[b].term, Term::Unreachable);
                    f.blocks[a].insts.extend(insts);
                    f.blocks[a].term = term;
                    merged = true;
                    changed = true;
                    break;
                }
            }
        }
        if !merged {
            break;
        }
    }
    let before = f.blocks.len();
    f.remove_unreachable();
    changed || f.blocks.len() != before
}

fn same_term(a: &Term, b: &Term) -> bool {
    format!("{:?}", a) == format!("{:?}", b)
}

// ---------------------------------------------------------------- локалды оңтайландыру

fn fits32(v: i64) -> bool {
    v >= i32::MIN as i64 && v <= i32::MAX as i64
}

fn fold(op: IOp, a: i64, b: i64) -> Option<i64> {
    Some(match op {
        IOp::Add => a.wrapping_add(b),
        IOp::Sub => a.wrapping_sub(b),
        IOp::Mul => a.wrapping_mul(b),
        IOp::And => a & b,
        IOp::Or => a | b,
        IOp::Xor => a ^ b,
        IOp::Shl => a.wrapping_shl((b & 63) as u32),
        IOp::Sar => a.wrapping_shr((b & 63) as u32),
        _ => return None,
    })
}

fn local_opts(f: &mut IrFunc) -> bool {
    let mut changed = false;
    for b in 0..f.blocks.len() {
        let mut consts: HashMap<VReg, i64> = HashMap::new();
        let mut copies: HashMap<VReg, VReg> = HashMap::new();
        let insts = std::mem::take(&mut f.blocks[b].insts);
        let mut out: Vec<Inst> = Vec::with_capacity(insts.len());
        for mut ins in insts {
            // көшірмелерді алмастыру
            let mut sub = false;
            ins.map_uses(|v| match copies.get(&v) {
                Some(&s) => {
                    sub = true;
                    s
                }
                None => v,
            });
            if sub {
                changed = true;
            }
            // тұрақтыларды бүктеу
            let new = match &ins {
                Inst::IBin(op, d, a, bb) => match (consts.get(a).copied(), consts.get(bb).copied()) {
                    (Some(x), Some(y)) => fold(*op, x, y).map(|v| Inst::IConst(*d, v)),
                    (_, Some(y)) if fits32(y) && !matches!(op, IOp::FloorDiv | IOp::FloorMod) => Some(Inst::IBinI(*op, *d, *a, y as i32)),
                    (Some(x), _) if fits32(x) && op.commutative() => Some(Inst::IBinI(*op, *d, *bb, x as i32)),
                    _ => None,
                },
                Inst::IBinI(op, d, a, imm) => consts.get(a).and_then(|x| fold(*op, *x, *imm as i64)).map(|v| Inst::IConst(*d, v)),
                Inst::ICmp(cc, d, a, bb) => match (consts.get(a).copied(), consts.get(bb).copied()) {
                    (Some(x), Some(y)) => Some(Inst::IConst(*d, cc.eval(x, y) as i64)),
                    (_, Some(y)) if fits32(y) => Some(Inst::ICmpI(*cc, *d, *a, y as i32)),
                    (Some(x), _) if fits32(x) => Some(Inst::ICmpI(cc.swap(), *d, *bb, x as i32)),
                    _ => None,
                },
                Inst::ICmpI(cc, d, a, imm) => consts.get(a).map(|x| Inst::IConst(*d, cc.eval(*x, *imm as i64) as i64)),
                Inst::INeg(d, s) => consts.get(s).map(|x| Inst::IConst(*d, x.wrapping_neg())),
                Inst::Mov(d, s) if f.classes[*d as usize] == RC::I => consts.get(s).map(|x| Inst::IConst(*d, *x)),
                _ => None,
            };
            if let Some(n) = new {
                ins = n;
                changed = true;
            }
            // карталарды жаңарту
            if let Some(d) = ins.def() {
                consts.remove(&d);
                copies.remove(&d);
                copies.retain(|_, s| *s != d);
                match &ins {
                    Inst::IConst(_, c) => {
                        consts.insert(d, *c);
                    }
                    Inst::Mov(_, s) if *s != d => {
                        copies.insert(d, *s);
                    }
                    _ => {}
                }
            }
            out.push(ins);
        }
        // терминатор
        let mut term = f.blocks[b].term.clone();
        term.map_uses(|v| *copies.get(&v).unwrap_or(&v));
        term = match term {
            Term::Br { cc, a, b: bb, t, f: ff } => match (consts.get(&a).copied(), consts.get(&bb).copied()) {
                (Some(x), Some(y)) => Term::Jmp(if cc.eval(x, y) { t } else { ff }),
                (_, Some(y)) if fits32(y) => Term::BrI { cc, a, imm: y as i32, t, f: ff },
                (Some(x), _) if fits32(x) => Term::BrI { cc: cc.swap(), a: bb, imm: x as i32, t, f: ff },
                _ => Term::Br { cc, a, b: bb, t, f: ff },
            },
            Term::BrI { cc, a, imm, t, f: ff } => match consts.get(&a) {
                Some(x) => Term::Jmp(if cc.eval(*x, imm as i64) { t } else { ff }),
                None => Term::BrI { cc, a, imm, t, f: ff },
            },
            other => other,
        };
        if !same_term(&term, &f.blocks[b].term) {
            changed = true;
        }
        f.blocks[b].insts = out;
        f.blocks[b].term = term;
    }
    changed
}

// ---------------------------------------------------------------- Mov біріктіру

fn counts(f: &IrFunc) -> (Vec<u32>, Vec<u32>) {
    let n = f.classes.len();
    let mut uses = vec![0u32; n];
    let mut defs = vec![0u32; n];
    for b in &f.blocks {
        for i in &b.insts {
            i.uses(|v| uses[v as usize] += 1);
            if let Some(d) = i.def() {
                defs[d as usize] += 1;
            }
        }
        b.term.uses(|v| uses[v as usize] += 1);
    }
    (uses, defs)
}

/// `t = op ...; d = t` -> `d = op ...` (t тек осы жерде қолданылса).
fn coalesce_movs(f: &mut IrFunc) -> bool {
    let (uses, defs) = counts(f);
    let mut changed = false;
    for b in 0..f.blocks.len() {
        let mut i = 0;
        while i < f.blocks[b].insts.len() {
            if let Inst::Mov(d, s) = f.blocks[b].insts[i] {
                if d != s && uses[s as usize] == 1 && defs[s as usize] == 1 && f.classes[d as usize] == f.classes[s as usize] {
                    // s-ті анықтайтын нұсқауды артқа қарай іздейміз
                    let mut j = i;
                    let mut found = None;
                    while j > 0 {
                        j -= 1;
                        let ins = &f.blocks[b].insts[j];
                        if ins.def() == Some(s) {
                            found = Some(j);
                            break;
                        }
                        // d аралықта қолданылса/анықталса — болмайды
                        let mut touches = ins.def() == Some(d);
                        ins.uses(|v| touches |= v == d);
                        if touches {
                            break;
                        }
                    }
                    if let Some(j) = found {
                        if !matches!(f.blocks[b].insts[j], Inst::Param(..)) {
                            f.blocks[b].insts[j].set_def(d);
                            f.blocks[b].insts.remove(i);
                            changed = true;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
    }
    changed
}

// ---------------------------------------------------------------- өлі кодты жою

fn live_out_sets(f: &IrFunc) -> Vec<Vec<u64>> {
    let nv = f.classes.len();
    let nb = f.blocks.len();
    let words = (nv + 63) / 64;
    let mut usep = vec![vec![0u64; words]; nb];
    let mut defp = vec![vec![0u64; words]; nb];
    for (bi, b) in f.blocks.iter().enumerate() {
        for ins in &b.insts {
            ins.uses(|v| {
                if defp[bi][v as usize / 64] >> (v % 64) & 1 == 0 {
                    usep[bi][v as usize / 64] |= 1 << (v % 64);
                }
            });
            if let Some(d) = ins.def() {
                defp[bi][d as usize / 64] |= 1 << (d % 64);
            }
        }
        b.term.uses(|v| {
            if defp[bi][v as usize / 64] >> (v % 64) & 1 == 0 {
                usep[bi][v as usize / 64] |= 1 << (v % 64);
            }
        });
    }
    let mut live_in = vec![vec![0u64; words]; nb];
    let mut live_out = vec![vec![0u64; words]; nb];
    loop {
        let mut changed = false;
        for bi in (0..nb).rev() {
            let mut out = vec![0u64; words];
            for s in f.blocks[bi].term.succs() {
                for w in 0..words {
                    out[w] |= live_in[s as usize][w];
                }
            }
            let mut inn = vec![0u64; words];
            for w in 0..words {
                inn[w] = usep[bi][w] | (out[w] & !defp[bi][w]);
            }
            if out != live_out[bi] || inn != live_in[bi] {
                live_out[bi] = out;
                live_in[bi] = inn;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    live_out
}

fn dce(f: &mut IrFunc) -> bool {
    let mut changed = false;
    loop {
        let live_out = live_out_sets(f);
        let mut any = false;
        for bi in 0..f.blocks.len() {
            let mut live = live_out[bi].clone();
            let set = |l: &mut Vec<u64>, v: VReg| l[v as usize / 64] |= 1 << (v % 64);
            let clear = |l: &mut Vec<u64>, v: VReg| l[v as usize / 64] &= !(1 << (v % 64));
            let get = |l: &Vec<u64>, v: VReg| l[v as usize / 64] >> (v % 64) & 1 == 1;
            f.blocks[bi].term.uses(|v| set(&mut live, v));
            let mut keep: Vec<bool> = vec![true; f.blocks[bi].insts.len()];
            for k in (0..f.blocks[bi].insts.len()).rev() {
                let ins = &mut f.blocks[bi].insts[k];
                if let Some(d) = ins.def() {
                    if !get(&live, d) {
                        if let Inst::Call { d: dd, .. } | Inst::CallInd { d: dd, .. } = ins {
                            if dd.is_some() {
                                *dd = None;
                                any = true;
                            }
                        } else if !ins.has_effect() {
                            keep[k] = false;
                            any = true;
                            continue;
                        }
                    } else {
                        clear(&mut live, d);
                    }
                }
                let ins = &f.blocks[bi].insts[k];
                ins.uses(|v| set(&mut live, v));
            }
            if keep.iter().any(|k| !*k) {
                let mut it = keep.iter();
                f.blocks[bi].insts.retain(|_| *it.next().unwrap());
            }
        }
        if any {
            changed = true;
        } else {
            break;
        }
    }
    changed
}

// ---------------------------------------------------------------- test біріктіру

fn fuse_test(f: &mut IrFunc) {
    let (uses, _) = counts(f);
    for b in &mut f.blocks {
        if let Term::BrI { cc: cc @ (Cc::E | Cc::Ne), a, imm: 0, t, f: ff } = b.term {
            if let Some(Inst::IBinI(IOp::And, d, x, m)) = b.insts.last().cloned() {
                if d == a && uses[d as usize] == 1 {
                    b.insts.pop();
                    b.term = Term::BrTest { cc, a: x, imm: m, t, f: ff };
                }
            }
        }
    }
}

// ---------------------------------------------------------------- if-түрлендіру (cmov)

fn pure_int(i: &Inst, f: &IrFunc) -> bool {
    match i {
        Inst::IConst(..) | Inst::INeg(..) | Inst::INot(..) | Inst::ICmp(..) | Inst::ICmpI(..) | Inst::IAbs(..) | Inst::IMin(..) | Inst::IMax(..) => true,
        Inst::Mov(d, _) => f.classes[*d as usize] == RC::I,
        Inst::IBin(op, ..) => !matches!(op, IOp::FloorDiv | IOp::FloorMod),
        Inst::IBinI(..) => true,
        Inst::Line(..) => true,
        _ => false,
    }
}

/// Диамант/үшбұрыш `егер c: x = A әйтпесе: x = B` пішінін `cmov`-ға айналдырады.
fn if_convert(f: &mut IrFunc) -> bool {
    let mut any = false;
    for _ in 0..64 {
        if !if_convert_once(f) {
            break;
        }
        any = true;
    }
    any
}

fn if_convert_once(f: &mut IrFunc) -> bool {
    let nb = f.blocks.len();
    let mut preds = vec![0u32; nb];
    preds[0] += 1;
    for b in &f.blocks {
        for s in b.term.succs() {
            preds[s as usize] += 1;
        }
    }
    let (_, defs) = counts(f);
    let live_out = live_out_sets(f);
    for h in 0..nb {
        let (cc, a, bop, t, ff) = match f.blocks[h].term {
            Term::Br { cc, a, b, t, f } => (cc, a, Some(b), t, f),
            Term::BrI { cc, a, imm, t, f } => (cc, a, None, t, f),
            _ => continue,
        };
        let _ = imm_of(&f.blocks[h].term);
        let (t, ff) = (t as usize, ff as usize);
        let arm_ok = |x: usize, f: &IrFunc| -> Option<usize> {
            // x — жеке жеке қолы: бір предшественник, `Jmp(m)`, аз әрі таза нұсқаулар
            let bl = &f.blocks[x];
            if x == h || bl.cold || preds[x] != 1 {
                return None;
            }
            let m = match bl.term {
                Term::Jmp(m) => m as usize,
                _ => return None,
            };
            let n = bl.insts.iter().filter(|i| !matches!(i, Inst::Line(..))).count();
            if n > 5 || !bl.insts.iter().all(|i| pure_int(i, f)) {
                return None;
            }
            Some(m)
        };
        let mt = arm_ok(t, f);
        let mf = arm_ok(ff, f);
        // шектеу: біріктіру нүктесі
        let (arm_t, arm_f, m) = match (mt, mf) {
            (Some(a1), Some(a2)) if a1 == a2 => (Some(t), Some(ff), a1),
            (Some(a1), _) if a1 == ff => (Some(t), None, a1),
            (_, Some(a2)) if a2 == t => (None, Some(ff), a2),
            _ => continue,
        };
        if m == h {
            continue;
        }
        // шығыстарды анықтау
        let live_m = |v: VReg, arm: usize| -> bool { live_out[arm][v as usize / 64] >> (v % 64) & 1 == 1 };
        let mut outs: Vec<VReg> = Vec::new();
        let mut ok = true;
        for arm in [arm_t, arm_f].into_iter().flatten() {
            for ins in &f.blocks[arm].insts {
                if let Some(d) = ins.def() {
                    if f.classes[d as usize] != RC::I {
                        ok = false;
                    }
                    if live_m(d, arm) {
                        if !outs.contains(&d) {
                            outs.push(d);
                        }
                    } else if defs[d as usize] != 1 {
                        ok = false;
                    }
                }
            }
        }
        if !ok || outs.is_empty() || outs.len() > 3 {
            continue;
        }
        // әр шығыс әр қолда ең көбі бір рет анықталады
        let mut single = true;
        for arm in [arm_t, arm_f].into_iter().flatten() {
            for &o in &outs {
                if f.blocks[arm].insts.iter().filter(|i| i.def() == Some(o)).count() > 1 {
                    single = false;
                }
            }
        }
        if !single {
            continue;
        }
        // түрлендіру
        let mut hoisted: Vec<Inst> = Vec::new();
        let mut renamed: HashMap<VReg, (Option<VReg>, Option<VReg>)> = HashMap::new(); // шығыс -> (T мәні, F мәні)
        for (which, arm) in [(0, arm_t), (1, arm_f)] {
            let arm = match arm {
                Some(a) => a,
                None => continue,
            };
            let mut map: HashMap<VReg, VReg> = HashMap::new();
            for ins in f.blocks[arm].insts.clone() {
                if matches!(ins, Inst::Line(..)) {
                    continue;
                }
                let mut ins = ins;
                ins.map_uses(|v| *map.get(&v).unwrap_or(&v));
                if let Some(d) = ins.def() {
                    if outs.contains(&d) {
                        let nv = f.new_vreg(RC::I);
                        ins.set_def(nv);
                        map.insert(d, nv);
                        let e = renamed.entry(d).or_insert((None, None));
                        if which == 0 {
                            e.0 = Some(nv);
                        } else {
                            e.1 = Some(nv);
                        }
                    }
                }
                hoisted.push(ins);
            }
        }
        let mut cond_b = bop;
        let mut cond_imm = None;
        if let Term::BrI { imm, .. } = f.blocks[h].term {
            cond_imm = Some(imm);
        }
        let mut cc2 = cc;
        let mut a2 = a;
        if outs.len() > 1 {
            // шартты бір рет есептеп алу
            let c = f.new_vreg(RC::I);
            match (cond_b, cond_imm) {
                (Some(bv), _) => hoisted.push(Inst::ICmp(cc, c, a, bv)),
                (None, Some(imm)) => hoisted.push(Inst::ICmpI(cc, c, a, imm)),
                _ => unreachable!(),
            }
            cc2 = Cc::Ne;
            a2 = c;
            cond_b = None;
            cond_imm = Some(0);
        }
        for &o in &outs {
            let (tv, fv) = renamed.get(&o).copied().unwrap_or((None, None));
            let tv = tv.unwrap_or(o);
            let fv = fv.unwrap_or(o);
            match (cond_b, cond_imm) {
                (Some(bv), _) => hoisted.push(Inst::Select { cc: cc2, d: o, a: a2, b: bv, t: tv, f: fv }),
                (None, Some(imm)) => hoisted.push(Inst::SelectI { cc: cc2, d: o, a: a2, imm, t: tv, f: fv }),
                _ => unreachable!(),
            }
        }
        f.blocks[h].insts.extend(hoisted);
        f.blocks[h].term = Term::Jmp(m as BlockId);
        for arm in [arm_t, arm_f].into_iter().flatten() {
            f.blocks[arm].insts.clear();
            f.blocks[arm].term = Term::Unreachable;
        }
        f.remove_unreachable();
        return true;
    }
    false
}

fn imm_of(_t: &Term) {}
