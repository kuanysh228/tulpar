//! IR деңгейіндегі инлайндау: кішкентай, циклсіз, рекурсивті емес функцияларды шақыру орнына қою.

use std::collections::HashMap;

use crate::ir::*;

const MAX_INSTS: usize = 40;
const MAX_BLOCKS: usize = 10;
const MAX_CALLER_INSTS: usize = 3000;

fn inst_count(f: &IrFunc) -> usize {
    f.blocks.iter().map(|b| b.insts.len() + 1).sum()
}

fn is_candidate(f: &IrFunc) -> bool {
    if f.sym == "tlp_main" || f.blocks.len() > MAX_BLOCKS || inst_count(f) > MAX_INSTS {
        return false;
    }
    if has_cycle(f) {
        return false;
    }
    // өзін шақырмайды
    for b in &f.blocks {
        for ins in &b.insts {
            if let Inst::Call { sym, .. } = ins {
                if *sym == f.sym {
                    return false;
                }
            }
        }
    }
    true
}

/// CFG-де цикл бар ма (DFS, артқа жебе).
fn has_cycle(f: &IrFunc) -> bool {
    let n = f.blocks.len();
    let mut color = vec![0u8; n];
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    color[0] = 1;
    while let Some(&(b, k)) = stack.last() {
        let succs = f.blocks[b].term.succs();
        if k < succs.len() {
            stack.last_mut().unwrap().1 += 1;
            let s = succs[k] as usize;
            match color[s] {
                1 => return true,
                0 => {
                    color[s] = 1;
                    stack.push((s, 0));
                }
                _ => {}
            }
        } else {
            color[b] = 2;
            stack.pop();
        }
    }
    false
}

pub fn inline_all(funcs: &mut Vec<IrFunc>) {
    for _round in 0..3 {
        let snapshot: HashMap<String, IrFunc> = funcs.iter().filter(|f| is_candidate(f)).map(|f| (f.sym.clone(), f.clone())).collect();
        if snapshot.is_empty() {
            return;
        }
        let mut any = false;
        for f in funcs.iter_mut() {
            any |= inline_into(f, &snapshot);
        }
        if !any {
            return;
        }
    }
}

fn arg_slots(args: &[VReg], classes: &[RC]) -> HashMap<ArgLoc, VReg> {
    let (mut ni, mut nf, mut ns) = (0u8, 0u8, 0u16);
    let mut m = HashMap::new();
    for &a in args {
        match classes[a as usize] {
            RC::I => {
                if ni < 6 {
                    m.insert(ArgLoc::IReg(ni), a);
                    ni += 1;
                } else {
                    m.insert(ArgLoc::Stack(ns), a);
                    ns += 1;
                }
            }
            RC::F => {
                if nf < 8 {
                    m.insert(ArgLoc::FReg(nf), a);
                    nf += 1;
                } else {
                    m.insert(ArgLoc::Stack(ns), a);
                    ns += 1;
                }
            }
        }
    }
    m
}

fn inline_into(f: &mut IrFunc, snap: &HashMap<String, IrFunc>) -> bool {
    let mut changed = false;
    let mut budget = 40;
    let mut b = 0;
    while b < f.blocks.len() {
        if inst_count(f) > MAX_CALLER_INSTS || budget == 0 {
            break;
        }
        let mut found = None;
        for (i, ins) in f.blocks[b].insts.iter().enumerate() {
            if let Inst::Call { sym, .. } = ins {
                if *sym != f.sym && snap.contains_key(sym) {
                    found = Some(i);
                    break;
                }
            }
        }
        let i = match found {
            Some(i) => i,
            None => {
                b += 1;
                continue;
            }
        };
        budget -= 1;
        changed = true;
        let (dest, sym, args) = match &f.blocks[b].insts[i] {
            Inst::Call { d, sym, args } => (*d, sym.clone(), args.clone()),
            _ => unreachable!(),
        };
        let callee = &snap[&sym];
        let slots = arg_slots(&args, &f.classes);
        let voff = f.classes.len() as u32;
        let boff = f.blocks.len() as u32;
        f.classes.extend(callee.classes.iter().cloned());
        let cont_id = boff + callee.blocks.len() as u32;
        // жалғасатын блок
        let tail: Vec<Inst> = f.blocks[b].insts.split_off(i + 1);
        f.blocks[b].insts.pop(); // шақыруды алып тастаймыз
        let orig_term = std::mem::replace(&mut f.blocks[b].term, Term::Jmp(boff));
        let cold = f.blocks[b].cold;
        let mut new_blocks: Vec<Block> = Vec::new();
        for cb in &callee.blocks {
            let mut insts = Vec::new();
            for ins in &cb.insts {
                let mut ni = ins.clone();
                // vreg-терді жылжыту
                ni.map_uses(|v| v + voff);
                if let Some(d) = ni.def() {
                    ni.set_def(d + voff);
                }
                if let Inst::Param(d, loc) = ni {
                    let src = slots[&loc];
                    insts.push(Inst::Mov(d, src));
                } else {
                    insts.push(ni);
                }
            }
            let mut term = cb.term.clone();
            term.map_uses(|v| v + voff);
            term.map_blocks(|x| x + boff);
            if let Term::Ret(v) = term {
                if let (Some(d), Some(v)) = (dest, v) {
                    insts.push(Inst::Mov(d, v));
                }
                term = Term::Jmp(cont_id);
            }
            new_blocks.push(Block { insts, term, cold: cb.cold });
        }
        new_blocks.push(Block { insts: tail, term: orig_term, cold });
        f.blocks.extend(new_blocks);
        // жалғасатын блокты (және ішіндегі шақыруларды) кейінірек қараймыз; b блогы енді шақырусыз қайта тексеріледі
        b += 1;
    }
    changed
}
