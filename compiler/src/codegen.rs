//! x86-64 (Intel синтаксисі, GNU as) код генераторы. System V ABI.

use std::collections::HashMap;
use std::fmt::Write;

use crate::hir::Module;
use crate::ir::*;
use crate::lower::{class_desc, ModuleCtx};
use crate::regalloc::*;

const R64: [&str; 16] = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
const R32: [&str; 16] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d", "r13d", "r14d", "r15d"];
const R8N: [&str; 16] = ["al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b", "r13b", "r14b", "r15b"];
const ARG_REGS: [u8; 6] = [RDI, RSI, RDX, RCX, R8, R9];

fn r64(r: u8) -> &'static str {
    R64[r as usize]
}
fn x(r: u8) -> String {
    format!("xmm{}", r)
}
fn mem(off: i32) -> String {
    format!("qword ptr [rbp{:+}]", off)
}

#[derive(Default)]
pub struct Consts {
    floats: HashMap<u64, usize>,
    order: Vec<u64>,
}

impl Consts {
    fn float(&mut self, bits: u64) -> String {
        let n = match self.floats.get(&bits) {
            Some(&n) => n,
            None => {
                let n = self.order.len();
                self.order.push(bits);
                self.floats.insert(bits, n);
                n
            }
        };
        format!(".LC{}", n)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Place {
    R(u8),
    X(u8),
    /// rbp-ға қатысты жад
    M(i32),
}

struct Gen<'a> {
    out: String,
    f: &'a IrFunc,
    a: Alloc,
    consts: &'a mut Consts,
    next: Option<BlockId>,
    fidx: usize,
    label_n: usize,
    lines: Vec<(String, u16, u32)>,
}

impl<'a> Gen<'a> {
    fn e(&mut self, s: impl AsRef<str>) {
        self.out.push_str("    ");
        self.out.push_str(s.as_ref());
        self.out.push('\n');
    }
    fn lbl(&self, b: BlockId) -> String {
        format!(".L{}_{}", self.fidx, b)
    }
    fn new_label(&mut self) -> String {
        self.label_n += 1;
        format!(".Lq{}_{}", self.fidx, self.label_n)
    }
    fn loc(&self, v: VReg) -> Loc {
        self.a.locs[v as usize]
    }
    fn is_f(&self, v: VReg) -> bool {
        self.f.classes[v as usize] == RC::F
    }

    // ---- бүтін операндтар
    /// Бүтін мән регистрде тұрса — сол, әйтпесе scratch-қа жүктейді.
    fn ireg(&mut self, v: VReg, scratch: u8) -> u8 {
        match self.loc(v) {
            Loc::Reg(r) => r,
            Loc::Stack(o) => {
                self.e(format!("mov {}, {}", r64(scratch), mem(o)));
                scratch
            }
            Loc::None => {
                self.e(format!("xor {}, {}", R32[scratch as usize], R32[scratch as usize]));
                scratch
            }
        }
    }
    /// Дереккөз операнд: регистр немесе жад.
    fn isrc(&self, v: VReg) -> String {
        match self.loc(v) {
            Loc::Reg(r) => r64(r).to_string(),
            Loc::Stack(o) => mem(o),
            Loc::None => "0".to_string(),
        }
    }
    fn is_mem(&self, v: VReg) -> bool {
        matches!(self.loc(v), Loc::Stack(_))
    }
    fn istore(&mut self, d: VReg, r: u8) {
        match self.loc(d) {
            Loc::Reg(dr) => {
                if dr != r {
                    self.e(format!("mov {}, {}", r64(dr), r64(r)));
                }
            }
            Loc::Stack(o) => self.e(format!("mov {}, {}", mem(o), r64(r))),
            Loc::None => {}
        }
    }
    fn itarget(&self, d: VReg) -> u8 {
        match self.loc(d) {
            Loc::Reg(r) => r,
            _ => RAX,
        }
    }

    // ---- float операндтар
    fn freg(&mut self, v: VReg, scratch: u8) -> u8 {
        match self.loc(v) {
            Loc::Reg(r) => r,
            Loc::Stack(o) => {
                self.e(format!("movsd {}, {}", x(scratch), mem(o)));
                scratch
            }
            Loc::None => {
                self.e(format!("xorpd {}, {}", x(scratch), x(scratch)));
                scratch
            }
        }
    }
    fn fsrc(&self, v: VReg) -> String {
        match self.loc(v) {
            Loc::Reg(r) => x(r),
            Loc::Stack(o) => mem(o),
            Loc::None => "xmm15".to_string(),
        }
    }
    fn fstore(&mut self, d: VReg, r: u8) {
        match self.loc(d) {
            Loc::Reg(dr) => {
                if dr != r {
                    self.e(format!("movaps {}, {}", x(dr), x(r)));
                }
            }
            Loc::Stack(o) => self.e(format!("movsd {}, {}", mem(o), x(r))),
            Loc::None => {}
        }
    }
    fn ftarget(&self, d: VReg) -> u8 {
        match self.loc(d) {
            Loc::Reg(r) => r,
            _ => 0,
        }
    }

    // ---- параллель жылжыту
    fn parallel_move(&mut self, moves: Vec<(Place, Place)>) {
        // (dst, src); dst тек R/X/M
        let mut pend: Vec<(Place, Place)> = moves.into_iter().filter(|(d, s)| d != s).collect();
        // жадқа жазулар алдымен (регистрлерді бұзбайды)
        let mut k = 0;
        while k < pend.len() {
            if let (Place::M(o), s) = pend[k] {
                match s {
                    Place::R(r) => self.e(format!("mov {}, {}", mem(o), r64(r))),
                    Place::X(r) => self.e(format!("movsd {}, {}", mem(o), x(r))),
                    Place::M(so) => {
                        self.e(format!("mov rax, {}", mem(so)));
                        self.e(format!("mov {}, rax", mem(o)));
                    }
                }
                pend.remove(k);
            } else {
                k += 1;
            }
        }
        while !pend.is_empty() {
            let mut progressed = false;
            let mut i = 0;
            while i < pend.len() {
                let (d, s) = pend[i];
                let blocked = pend.iter().enumerate().any(|(j, (_, s2))| j != i && *s2 == d);
                if !blocked {
                    match (d, s) {
                        (Place::R(dr), Place::R(sr)) => self.e(format!("mov {}, {}", r64(dr), r64(sr))),
                        (Place::R(dr), Place::M(so)) => self.e(format!("mov {}, {}", r64(dr), mem(so))),
                        (Place::X(dr), Place::X(sr)) => self.e(format!("movaps {}, {}", x(dr), x(sr))),
                        (Place::X(dr), Place::M(so)) => self.e(format!("movsd {}, {}", x(dr), mem(so))),
                        _ => unreachable!("parallel_move {:?}", (d, s)),
                    }
                    pend.remove(i);
                    progressed = true;
                } else {
                    i += 1;
                }
            }
            if !progressed {
                // цикл: бір тағайындалу мәнін scratch-қа сақтаймыз
                let (d, _) = pend[0];
                match d {
                    Place::R(dr) => {
                        self.e(format!("mov r11, {}", r64(dr)));
                        for m in pend.iter_mut() {
                            if m.1 == d {
                                m.1 = Place::R(R11);
                            }
                        }
                    }
                    Place::X(dr) => {
                        self.e(format!("movaps xmm15, {}", x(dr)));
                        for m in pend.iter_mut() {
                            if m.1 == d {
                                m.1 = Place::X(15);
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn place_of(&self, v: VReg) -> Place {
        match (self.loc(v), self.is_f(v)) {
            (Loc::Reg(r), false) => Place::R(r),
            (Loc::Reg(r), true) => Place::X(r),
            (Loc::Stack(o), _) => Place::M(o),
            (Loc::None, _) => Place::M(0),
        }
    }

    // ---- функция
    fn gen_func(&mut self) {
        let sym = self.f.sym.clone();
        let name = self.f.name.clone();
        let _ = writeln!(self.out, "# {}", name);
        let _ = writeln!(self.out, ".globl {}\n{}:", sym, sym);
        self.e("push rbp");
        self.e("mov rbp, rsp");
        if self.a.frame > 0 {
            self.e(format!("sub rsp, {}", self.a.frame));
        }
        for (i, r) in self.a.saved.clone().iter().enumerate() {
            self.e(format!("mov qword ptr [rbp-{}], {}", 8 * (i + 1), r64(*r)));
        }
        let order = self.a.order.clone();
        for (bi, &b) in order.iter().enumerate() {
            self.next = order.get(bi + 1).copied();
            let lbl = self.lbl(b);
            let _ = writeln!(self.out, "{}:", lbl);
            let insts = self.f.blocks[b as usize].insts.clone();
            let mut start = 0;
            if bi == 0 {
                // бастапқы Param-дар: параллель жылжыту
                let mut moves = Vec::new();
                while start < insts.len() {
                    if let Inst::Param(d, loc) = &insts[start] {
                        let src = match loc {
                            ArgLoc::IReg(k) => Place::R(ARG_REGS[*k as usize]),
                            ArgLoc::FReg(k) => Place::X(*k),
                            ArgLoc::Stack(i) => Place::M(16 + 8 * *i as i32),
                        };
                        let dst = self.place_of(*d);
                        if matches!(self.loc(*d), Loc::None) {
                            start += 1;
                            continue;
                        }
                        // стектен жадқа: Place::M(+) бастапқысы rbp-ға қатысты
                        moves.push((dst, src));
                        start += 1;
                    } else {
                        break;
                    }
                }
                self.parallel_move(moves);
            }
            for ins in &insts[start..] {
                self.gen_inst(ins);
            }
            let term = self.f.blocks[b as usize].term.clone();
            self.gen_term(&term);
        }
    }

    fn epilogue(&mut self) {
        for (i, r) in self.a.saved.clone().iter().enumerate() {
            self.e(format!("mov {}, qword ptr [rbp-{}]", r64(*r), 8 * (i + 1)));
        }
        self.e("leave");
        self.e("ret");
    }

    fn jmp(&mut self, b: BlockId) {
        if self.next != Some(b) {
            let l = self.lbl(b);
            self.e(format!("jmp {}", l));
        }
    }

    fn branch(&mut self, cc: &str, inv: &str, t: BlockId, f: BlockId) {
        let (lt, lf) = (self.lbl(t), self.lbl(f));
        if self.next == Some(f) {
            self.e(format!("j{} {}", cc, lt));
        } else if self.next == Some(t) {
            self.e(format!("j{} {}", inv, lf));
        } else {
            self.e(format!("j{} {}", cc, lt));
            self.e(format!("jmp {}", lf));
        }
    }

    fn gen_term(&mut self, t: &Term) {
        match t {
            Term::Jmp(b) => self.jmp(*b),
            Term::Br { cc, a, b, t, f } => {
                if self.is_mem(*a) && self.is_mem(*b) {
                    let ra = self.ireg(*a, RAX);
                    let bs = self.isrc(*b);
                    self.e(format!("cmp {}, {}", r64(ra), bs));
                } else {
                    let (asr, bs) = (self.isrc(*a), self.isrc(*b));
                    self.e(format!("cmp {}, {}", asr, bs));
                }
                self.branch(cc.suffix(), cc.invert().suffix(), *t, *f);
            }
            Term::BrI { cc, a, imm, t, f } => {
                if *imm == 0 && !self.is_mem(*a) {
                    let s = self.isrc(*a);
                    self.e(format!("test {}, {}", s, s));
                } else {
                    let s = self.isrc(*a);
                    self.e(format!("cmp {}, {}", s, imm));
                }
                self.branch(cc.suffix(), cc.invert().suffix(), *t, *f);
            }
            Term::BrTest { cc, a, imm, t, f } => {
                let s = self.isrc(*a);
                self.e(format!("test {}, {}", s, imm));
                self.branch(cc.suffix(), cc.invert().suffix(), *t, *f);
            }
            Term::BrF { cc, a, b, t, f } => {
                let (lt, lf) = (self.lbl(*t), self.lbl(*f));
                match cc {
                    FCc::Lt | FCc::Le => {
                        // a < b  <=>  ucomisd b, a ; ja
                        let rb = self.freg(*b, 0);
                        let asr = self.fsrc(*a);
                        self.e(format!("ucomisd {}, {}", x(rb), asr));
                        let c = if *cc == FCc::Lt { "a" } else { "ae" };
                        let inv = if *cc == FCc::Lt { "be" } else { "b" };
                        self.branch(c, inv, *t, *f);
                    }
                    FCc::Gt | FCc::Ge => {
                        let ra = self.freg(*a, 0);
                        let bs = self.fsrc(*b);
                        self.e(format!("ucomisd {}, {}", x(ra), bs));
                        let c = if *cc == FCc::Gt { "a" } else { "ae" };
                        let inv = if *cc == FCc::Gt { "be" } else { "b" };
                        self.branch(c, inv, *t, *f);
                    }
                    FCc::Eq => {
                        let ra = self.freg(*a, 0);
                        let bs = self.fsrc(*b);
                        self.e(format!("ucomisd {}, {}", x(ra), bs));
                        self.e(format!("jp {}", lf));
                        self.e(format!("je {}", lt));
                        if self.next != Some(*f) {
                            self.e(format!("jmp {}", lf));
                        }
                    }
                    FCc::Ne => {
                        let ra = self.freg(*a, 0);
                        let bs = self.fsrc(*b);
                        self.e(format!("ucomisd {}, {}", x(ra), bs));
                        self.e(format!("jp {}", lt));
                        self.e(format!("jne {}", lt));
                        if self.next != Some(*f) {
                            self.e(format!("jmp {}", lf));
                        }
                    }
                }
            }
            Term::Ret(v) => {
                if let Some(v) = v {
                    if self.is_f(*v) {
                        match self.loc(*v) {
                            Loc::Reg(r) => {
                                if r != 0 {
                                    self.e(format!("movaps xmm0, {}", x(r)));
                                }
                            }
                            Loc::Stack(o) => self.e(format!("movsd xmm0, {}", mem(o))),
                            Loc::None => {}
                        }
                    } else {
                        match self.loc(*v) {
                            Loc::Reg(r) => self.e(format!("mov rax, {}", r64(r))),
                            Loc::Stack(o) => self.e(format!("mov rax, {}", mem(o))),
                            Loc::None => {}
                        }
                    }
                }
                self.epilogue();
            }
            Term::Unreachable => self.e("ud2"),
        }
    }

    fn gen_ibin(&mut self, op: IOp, d: VReg, a: VReg, b: VReg) {
        match op {
            IOp::FloorDiv | IOp::FloorMod => {
                let ra = self.isrc(a);
                self.e(format!("mov rax, {}", ra));
                let bs = match self.loc(b) {
                    Loc::Reg(r) => r64(r).to_string(),
                    _ => {
                        let s = self.isrc(b);
                        self.e(format!("mov r11, {}", s));
                        "r11".to_string()
                    }
                };
                let done = self.new_label();
                self.e("cqo");
                self.e(format!("idiv {}", bs));
                self.e("test rdx, rdx");
                self.e(format!("jz {}", done));
                if op == IOp::FloorDiv {
                    self.e("mov rcx, rdx");
                    self.e(format!("xor rcx, {}", bs));
                    self.e(format!("jns {}", done));
                    self.e("dec rax");
                    let _ = writeln!(self.out, "{}:", done);
                    self.istore(d, RAX);
                } else {
                    self.e("mov rcx, rdx");
                    self.e(format!("xor rcx, {}", bs));
                    self.e(format!("jns {}", done));
                    self.e(format!("add rdx, {}", bs));
                    let _ = writeln!(self.out, "{}:", done);
                    self.istore(d, RDX);
                }
            }
            IOp::Shl | IOp::Sar => {
                let mut t = self.itarget(d);
                let (la, lb) = (self.loc(a), self.loc(b));
                if lb == Loc::Reg(t) && la != Loc::Reg(t) {
                    t = RAX;
                }
                let asr = self.isrc(a);
                if la != Loc::Reg(t) {
                    self.e(format!("mov {}, {}", r64(t), asr));
                }
                let bs = self.isrc(b);
                self.e(format!("mov rcx, {}", bs));
                let m = if op == IOp::Shl { "shl" } else { "sar" };
                self.e(format!("{} {}, cl", m, r64(t)));
                self.istore(d, t);
            }
            IOp::Add if matches!((self.loc(d), self.loc(a), self.loc(b)), (Loc::Reg(rd), Loc::Reg(ra), Loc::Reg(rb)) if rd != ra && rd != rb) => {
                if let (Loc::Reg(rd), Loc::Reg(ra), Loc::Reg(rb)) = (self.loc(d), self.loc(a), self.loc(b)) {
                    self.e(format!("lea {}, [{}+{}]", r64(rd), r64(ra), r64(rb)));
                }
            }
            _ => {
                let mut t = self.itarget(d);
                let (mut a, mut b) = (a, b);
                let (la, lb) = (self.loc(a), self.loc(b));
                if lb == Loc::Reg(t) && la != Loc::Reg(t) {
                    if op.commutative() {
                        std::mem::swap(&mut a, &mut b);
                    } else {
                        t = RAX;
                    }
                }
                let la = self.loc(a);
                let asr = self.isrc(a);
                if la != Loc::Reg(t) {
                    self.e(format!("mov {}, {}", r64(t), asr));
                }
                let bs = self.isrc(b);
                let m = match op {
                    IOp::Add => "add",
                    IOp::Sub => "sub",
                    IOp::Mul => "imul",
                    IOp::And => "and",
                    IOp::Or => "or",
                    IOp::Xor => "xor",
                    _ => unreachable!(),
                };
                self.e(format!("{} {}, {}", m, r64(t), bs));
                self.istore(d, t);
            }
        }
    }

    fn gen_ibini(&mut self, op: IOp, d: VReg, a: VReg, imm: i32) {
        let t = self.itarget(d);
        let la = self.loc(a);
        let asr = self.isrc(a);
        match op {
            IOp::Mul => {
                self.e(format!("imul {}, {}, {}", r64(t), asr, imm));
            }
            IOp::Add if matches!((la, self.loc(d)), (Loc::Reg(ra), Loc::Reg(rd)) if ra != rd) => {
                if let Loc::Reg(ra) = la {
                    self.e(format!("lea {}, [{}{:+}]", r64(t), r64(ra), imm));
                }
            }
            IOp::Sub if imm != i32::MIN && matches!((la, self.loc(d)), (Loc::Reg(ra), Loc::Reg(rd)) if ra != rd) => {
                if let Loc::Reg(ra) = la {
                    self.e(format!("lea {}, [{}{:+}]", r64(t), r64(ra), -imm));
                }
            }
            _ => {
                if la != Loc::Reg(t) {
                    self.e(format!("mov {}, {}", r64(t), asr));
                }
                let m = match op {
                    IOp::Add => "add",
                    IOp::Sub => "sub",
                    IOp::And => "and",
                    IOp::Or => "or",
                    IOp::Xor => "xor",
                    IOp::Shl => "shl",
                    IOp::Sar => "sar",
                    _ => unreachable!("ibini {:?}", op),
                };
                self.e(format!("{} {}, {}", m, r64(t), imm));
            }
        }
        self.istore(d, t);
    }

    fn setcc(&mut self, cc: &str, d: VReg) {
        let t = self.itarget(d);
        self.e(format!("set{} al", cc));
        self.e(format!("movzx {}, al", R32[t as usize]));
        self.istore(d, t);
    }

    fn gen_inst(&mut self, ins: &Inst) {
        match ins {
            Inst::IConst(d, v) => match self.loc(*d) {
                Loc::Reg(r) => {
                    if *v == 0 {
                        self.e(format!("xor {}, {}", R32[r as usize], R32[r as usize]));
                    } else if *v > 0 && *v <= u32::MAX as i64 {
                        self.e(format!("mov {}, {}", R32[r as usize], v));
                    } else if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                        self.e(format!("mov {}, {}", r64(r), v));
                    } else {
                        self.e(format!("movabs {}, {}", r64(r), v));
                    }
                }
                Loc::Stack(o) => {
                    if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                        self.e(format!("mov {}, {}", mem(o), v));
                    } else {
                        self.e(format!("movabs rax, {}", v));
                        self.e(format!("mov {}, rax", mem(o)));
                    }
                }
                Loc::None => {}
            },
            Inst::FConst(d, v) => {
                let t = if let Loc::Reg(r) = self.loc(*d) { r } else { 15 };
                if v.to_bits() == 0 {
                    self.e(format!("xorpd {}, {}", x(t), x(t)));
                } else {
                    let c = self.consts.float(v.to_bits());
                    self.e(format!("movsd {}, qword ptr [rip + {}]", x(t), c));
                }
                self.fstore(*d, t);
            }
            Inst::Mov(d, s) => {
                if self.is_f(*d) {
                    match (self.loc(*d), self.loc(*s)) {
                        (Loc::Reg(a), Loc::Reg(b)) => {
                            if a != b {
                                self.e(format!("movaps {}, {}", x(a), x(b)));
                            }
                        }
                        (Loc::Reg(a), Loc::Stack(o)) => self.e(format!("movsd {}, {}", x(a), mem(o))),
                        (Loc::Stack(o), Loc::Reg(b)) => self.e(format!("movsd {}, {}", mem(o), x(b))),
                        (Loc::Stack(o1), Loc::Stack(o2)) => {
                            if o1 != o2 {
                                self.e(format!("mov rax, {}", mem(o2)));
                                self.e(format!("mov {}, rax", mem(o1)));
                            }
                        }
                        _ => {}
                    }
                } else {
                    match (self.loc(*d), self.loc(*s)) {
                        (Loc::Reg(a), Loc::Reg(b)) => {
                            if a != b {
                                self.e(format!("mov {}, {}", r64(a), r64(b)));
                            }
                        }
                        (Loc::Reg(a), Loc::Stack(o)) => self.e(format!("mov {}, {}", r64(a), mem(o))),
                        (Loc::Stack(o), Loc::Reg(b)) => self.e(format!("mov {}, {}", mem(o), r64(b))),
                        (Loc::Stack(o1), Loc::Stack(o2)) => {
                            if o1 != o2 {
                                self.e(format!("mov rax, {}", mem(o2)));
                                self.e(format!("mov {}, rax", mem(o1)));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Inst::IBin(op, d, a, b) => self.gen_ibin(*op, *d, *a, *b),
            Inst::IBinI(op, d, a, imm) => self.gen_ibini(*op, *d, *a, *imm),
            Inst::FBin(op, d, a, b) => {
                let mut t = self.ftarget(*d);
                let (mut a, mut b) = (*a, *b);
                let (la, lb) = (self.loc(a), self.loc(b));
                if lb == Loc::Reg(t) && la != Loc::Reg(t) && matches!(self.loc(*d), Loc::Reg(_)) {
                    if matches!(op, FOp::Add | FOp::Mul) {
                        std::mem::swap(&mut a, &mut b);
                    } else {
                        t = 0;
                    }
                }
                let la = self.loc(a);
                if la != Loc::Reg(t) {
                    let asr = self.fsrc(a);
                    if matches!(la, Loc::Reg(_)) {
                        self.e(format!("movaps {}, {}", x(t), asr));
                    } else {
                        self.e(format!("movsd {}, {}", x(t), asr));
                    }
                }
                let bs = self.fsrc(b);
                let m = match op {
                    FOp::Add => "addsd",
                    FOp::Sub => "subsd",
                    FOp::Mul => "mulsd",
                    FOp::Div => "divsd",
                };
                self.e(format!("{} {}, {}", m, x(t), bs));
                self.fstore(*d, t);
            }
            Inst::INeg(d, s) | Inst::INot(d, s) => {
                let t = self.itarget(*d);
                let ss = self.isrc(*s);
                if self.loc(*s) != Loc::Reg(t) {
                    self.e(format!("mov {}, {}", r64(t), ss));
                }
                let m = if matches!(ins, Inst::INeg(..)) { "neg" } else { "not" };
                self.e(format!("{} {}", m, r64(t)));
                self.istore(*d, t);
            }
            Inst::IAbs(d, s) => {
                let ss = self.isrc(*s);
                self.e(format!("mov rax, {}", ss));
                self.e("neg rax");
                self.e(format!("cmovl rax, {}", ss));
                self.istore(*d, RAX);
            }
            Inst::IMin(d, a, b) | Inst::IMax(d, a, b) => {
                let asr = self.isrc(*a);
                self.e(format!("mov rax, {}", asr));
                let bs = self.isrc(*b);
                self.e(format!("cmp rax, {}", bs));
                let m = if matches!(ins, Inst::IMin(..)) { "cmovg" } else { "cmovl" };
                self.e(format!("{} rax, {}", m, bs));
                self.istore(*d, RAX);
            }
            Inst::FNeg(d, s) | Inst::FAbs(d, s) => {
                let t = if let Loc::Reg(r) = self.loc(*d) { r } else { 0 };
                let ss = self.fsrc(*s);
                if self.loc(*s) != Loc::Reg(t) {
                    if matches!(self.loc(*s), Loc::Reg(_)) {
                        self.e(format!("movaps {}, {}", x(t), ss));
                    } else {
                        self.e(format!("movsd {}, {}", x(t), ss));
                    }
                }
                if matches!(ins, Inst::FNeg(..)) {
                    self.e(format!("xorpd {}, xmmword ptr [rip + .Lfneg]", x(t)));
                } else {
                    self.e(format!("andpd {}, xmmword ptr [rip + .Lfabs]", x(t)));
                }
                self.fstore(*d, t);
            }
            Inst::FSqrt(d, s) => {
                let t = self.ftarget(*d);
                let ss = self.fsrc(*s);
                self.e(format!("sqrtsd {}, {}", x(t), ss));
                self.fstore(*d, t);
            }
            Inst::FRound(d, s, mode) => {
                let t = self.ftarget(*d);
                let ss = self.fsrc(*s);
                self.e(format!("roundsd {}, {}, {}", x(t), ss, if *mode == 0 { 9 } else { 10 }));
                self.fstore(*d, t);
            }
            Inst::ICmp(cc, d, a, b) => {
                if self.is_mem(*a) && self.is_mem(*b) {
                    let ra = self.ireg(*a, RAX);
                    let bs = self.isrc(*b);
                    self.e(format!("cmp {}, {}", r64(ra), bs));
                } else {
                    let (asr, bs) = (self.isrc(*a), self.isrc(*b));
                    self.e(format!("cmp {}, {}", asr, bs));
                }
                self.setcc(cc.suffix(), *d);
            }
            Inst::ICmpI(cc, d, a, imm) => {
                let asr = self.isrc(*a);
                self.e(format!("cmp {}, {}", asr, imm));
                self.setcc(cc.suffix(), *d);
            }
            Inst::FCmp(cc, d, a, b) => {
                let t = self.itarget(*d);
                match cc {
                    FCc::Lt | FCc::Le => {
                        let rb = self.freg(*b, 0);
                        let asr = self.fsrc(*a);
                        self.e(format!("ucomisd {}, {}", x(rb), asr));
                        self.setcc(if *cc == FCc::Lt { "a" } else { "ae" }, *d);
                    }
                    FCc::Gt | FCc::Ge => {
                        let ra = self.freg(*a, 0);
                        let bs = self.fsrc(*b);
                        self.e(format!("ucomisd {}, {}", x(ra), bs));
                        self.setcc(if *cc == FCc::Gt { "a" } else { "ae" }, *d);
                    }
                    FCc::Eq | FCc::Ne => {
                        let ra = self.freg(*a, 0);
                        let bs = self.fsrc(*b);
                        self.e(format!("ucomisd {}, {}", x(ra), bs));
                        if *cc == FCc::Eq {
                            self.e("sete al");
                            self.e("setnp cl");
                            self.e("and al, cl");
                        } else {
                            self.e("setne al");
                            self.e("setp cl");
                            self.e("or al, cl");
                        }
                        self.e(format!("movzx {}, al", R32[t as usize]));
                        self.istore(*d, t);
                    }
                }
            }
            Inst::I2F(d, s) => {
                let t = self.ftarget(*d);
                let ss = self.isrc(*s);
                self.e(format!("pxor {}, {}", x(t), x(t)));
                self.e(format!("cvtsi2sd {}, {}", x(t), ss));
                self.fstore(*d, t);
            }
            Inst::F2I(d, s) => {
                let t = self.itarget(*d);
                let ss = self.fsrc(*s);
                self.e(format!("cvttsd2si {}, {}", r64(t), ss));
                self.istore(*d, t);
            }
            Inst::BitsFI(d, s) => {
                let t = self.itarget(*d);
                match self.loc(*s) {
                    Loc::Reg(r) => self.e(format!("movq {}, {}", r64(t), x(r))),
                    Loc::Stack(o) => self.e(format!("mov {}, {}", r64(t), mem(o))),
                    Loc::None => {}
                }
                self.istore(*d, t);
            }
            Inst::BitsIF(d, s) => {
                let t = self.ftarget(*d);
                match self.loc(*s) {
                    Loc::Reg(r) => self.e(format!("movq {}, {}", x(t), r64(r))),
                    Loc::Stack(o) => self.e(format!("movsd {}, {}", x(t), mem(o))),
                    Loc::None => {}
                }
                self.fstore(*d, t);
            }
            Inst::Load { d, base, off, w } => {
                let b = self.ireg(*base, R10);
                let addr = format!("[{}{:+}]", r64(b), off);
                if self.is_f(*d) {
                    let t = self.ftarget(*d);
                    self.e(format!("movsd {}, qword ptr {}", x(t), addr));
                    self.fstore(*d, t);
                } else {
                    let t = self.itarget(*d);
                    match w {
                        Width::B8 => self.e(format!("mov {}, qword ptr {}", r64(t), addr)),
                        Width::B1 => self.e(format!("movzx {}, byte ptr {}", R32[t as usize], addr)),
                    }
                    self.istore(*d, t);
                }
            }
            Inst::Store { base, off, s, w } => {
                let b = self.ireg(*base, R10);
                let addr = format!("[{}{:+}]", r64(b), off);
                self.gen_store(&addr, *s, *w);
            }
            Inst::LoadIdx { d, base, idx, scale, off, w } => {
                let b = self.ireg(*base, R10);
                let i = self.ireg(*idx, R11);
                let addr = format!("[{}+{}*{}{:+}]", r64(b), r64(i), scale, off);
                if self.is_f(*d) {
                    let t = self.ftarget(*d);
                    self.e(format!("movsd {}, qword ptr {}", x(t), addr));
                    self.fstore(*d, t);
                } else {
                    let t = self.itarget(*d);
                    match w {
                        Width::B8 => self.e(format!("mov {}, qword ptr {}", r64(t), addr)),
                        Width::B1 => self.e(format!("movzx {}, byte ptr {}", R32[t as usize], addr)),
                    }
                    self.istore(*d, t);
                }
            }
            Inst::StoreIdx { base, idx, scale, off, s, w } => {
                let b = self.ireg(*base, R10);
                let i = self.ireg(*idx, R11);
                let addr = format!("[{}+{}*{}{:+}]", r64(b), r64(i), scale, off);
                self.gen_store(&addr, *s, *w);
            }
            Inst::SymAddr(d, sym) => {
                let t = self.itarget(*d);
                self.e(format!("lea {}, [rip + {}]", r64(t), sym));
                self.istore(*d, t);
            }
            Inst::LoadGlobal(d, off) => {
                if self.is_f(*d) {
                    let t = self.ftarget(*d);
                    self.e(format!("movsd {}, qword ptr [rip + tlp_globals + {}]", x(t), off));
                    self.fstore(*d, t);
                } else {
                    let t = self.itarget(*d);
                    self.e(format!("mov {}, qword ptr [rip + tlp_globals + {}]", r64(t), off));
                    self.istore(*d, t);
                }
            }
            Inst::StoreGlobal(off, s) => {
                let addr = format!("[rip + tlp_globals + {}]", off);
                self.gen_store(&addr, *s, Width::B8);
            }
            Inst::Select { cc, d, a, b, t, f } => {
                if self.is_mem(*a) && self.is_mem(*b) {
                    let ra = self.ireg(*a, RAX);
                    let bs = self.isrc(*b);
                    self.e(format!("cmp {}, {}", r64(ra), bs));
                } else {
                    let (asr, bs) = (self.isrc(*a), self.isrc(*b));
                    self.e(format!("cmp {}, {}", asr, bs));
                }
                let (fs, ts) = (self.isrc(*f), self.isrc(*t));
                self.e(format!("mov rax, {}", fs));
                self.e(format!("cmov{} rax, {}", cc.suffix(), ts));
                self.istore(*d, RAX);
            }
            Inst::SelectI { cc, d, a, imm, t, f } => {
                let asr = self.isrc(*a);
                self.e(format!("cmp {}, {}", asr, imm));
                let (fs, ts) = (self.isrc(*f), self.isrc(*t));
                self.e(format!("mov rax, {}", fs));
                self.e(format!("cmov{} rax, {}", cc.suffix(), ts));
                self.istore(*d, RAX);
            }
            Inst::Line(file, line) => {
                if self.lines.last().map_or(true, |(_, f, l)| *f != *file || *l != *line) {
                    self.label_n += 1;
                    let lbl = format!(".Lln{}_{}", self.fidx, self.label_n);
                    let _ = writeln!(self.out, "{}:", lbl);
                    self.lines.push((lbl, *file, *line));
                }
            }
            Inst::Param(..) => {
                // бастапқы емес Param-дар болмауы тиіс
                panic!("Param кірісті емес жерде");
            }
            Inst::Call { d, sym, args } => self.gen_call(*d, Some(sym), None, args),
            Inst::CallInd { d, target, args } => self.gen_call(*d, None, Some(*target), args),
        }
    }

    fn gen_store(&mut self, addr: &str, s: VReg, w: Width) {
        if self.is_f(s) {
            let r = self.freg(s, 0);
            self.e(format!("movsd qword ptr {}, {}", addr, x(r)));
        } else {
            let r = self.ireg(s, RAX);
            match w {
                Width::B8 => self.e(format!("mov qword ptr {}, {}", addr, r64(r))),
                Width::B1 => self.e(format!("mov byte ptr {}, {}", addr, R8N[r as usize])),
            }
        }
    }

    fn gen_call(&mut self, d: Option<VReg>, sym: Option<&str>, target: Option<VReg>, args: &[VReg]) {
        let mut ni = 0;
        let mut nf = 0;
        let mut stack_args: Vec<VReg> = Vec::new();
        let mut moves: Vec<(Place, Place)> = Vec::new();
        for &a in args {
            if self.is_f(a) {
                if nf < 8 {
                    moves.push((Place::X(nf as u8), self.place_of(a)));
                    nf += 1;
                } else {
                    stack_args.push(a);
                }
            } else if ni < 6 {
                moves.push((Place::R(ARG_REGS[ni]), self.place_of(a)));
                ni += 1;
            } else {
                stack_args.push(a);
            }
        }
        let mut pad = 0;
        if !stack_args.is_empty() {
            pad = ((stack_args.len() + 1) / 2) * 16;
            self.e(format!("sub rsp, {}", pad));
            for (i, &a) in stack_args.iter().enumerate() {
                match self.loc(a) {
                    Loc::Reg(r) => {
                        if self.is_f(a) {
                            self.e(format!("movsd qword ptr [rsp+{}], {}", 8 * i, x(r)));
                        } else {
                            self.e(format!("mov qword ptr [rsp+{}], {}", 8 * i, r64(r)));
                        }
                    }
                    Loc::Stack(o) => {
                        self.e(format!("mov rax, {}", mem(o)));
                        self.e(format!("mov qword ptr [rsp+{}], rax", 8 * i));
                    }
                    Loc::None => {}
                }
            }
        }
        if let Some(t) = target {
            let ts = self.isrc(t);
            self.e(format!("mov r10, {}", ts));
        }
        self.parallel_move(moves);
        match sym {
            Some(s) => self.e(format!("call {}", s)),
            None => self.e("call r10"),
        }
        if pad > 0 {
            self.e(format!("add rsp, {}", pad));
        }
        if let Some(d) = d {
            if self.is_f(d) {
                self.fstore(d, 0);
            } else {
                self.istore(d, RAX);
            }
        }
    }
}

fn bytes_directive(out: &mut String, bytes: &[u8]) {
    for chunk in bytes.chunks(24) {
        let s: Vec<String> = chunk.iter().map(|b| b.to_string()).collect();
        let _ = writeln!(out, "    .byte {}", s.join(","));
    }
}

pub fn gen_module(m: &Module, funcs: &[IrFunc], cx: &ModuleCtx, spill_all: bool) -> String {
    let mut out = String::new();
    out.push_str(".intel_syntax noprefix\n.text\n");
    out.push_str(".globl _start\n_start:\n");
    out.push_str("    mov rdi, rsp\n    lea rsi, [rip + tlp_globals]\n    lea rdx, [rip + tlp_globals_end]\n    lea rcx, [rip + tlp_class_table]\n");
    out.push_str("    lea r8, [rip + tlp_info]\n    and rsp, -16\n    call tlp_rt_start\n    ud2\n\n");
    let mut consts = Consts::default();
    let mut all_lines: Vec<(String, u16, u32)> = Vec::new();
    for (i, f) in funcs.iter().enumerate() {
        let a = allocate(f, spill_all);
        let mut g = Gen { out: String::new(), f, a, consts: &mut consts, next: None, fidx: i, label_n: 0, lines: Vec::new() };
        g.gen_func();
        out.push_str(&g.out);
        out.push('\n');
        all_lines.extend(g.lines);
    }
    out.push_str("tlp_text_end:\n");
    out.push_str(".section .rodata\n.balign 16\n");
    out.push_str(".Lfneg:\n    .quad 0x8000000000000000, 0x8000000000000000\n");
    out.push_str(".Lfabs:\n    .quad 0x7fffffffffffffff, 0x7fffffffffffffff\n");
    out.push_str(".balign 8\n");
    for (i, b) in consts.order.iter().enumerate() {
        let _ = writeln!(out, ".LC{}:\n    .quad {}", i, b);
    }
    for (i, s) in cx.strings.iter().enumerate() {
        let _ = writeln!(out, ".balign 8\n.Lstr{}:\n    .quad {}, {}", i, s.len(), s.chars().count());
        bytes_directive(&mut out, s.as_bytes());
    }
    for (i, d) in cx.descs.iter().enumerate() {
        let _ = writeln!(out, ".Ldesc{}:", i);
        bytes_directive(&mut out, d);
    }
    for (i, c) in m.classes.iter().enumerate() {
        let _ = writeln!(out, ".Lcls{}:", i);
        bytes_directive(&mut out, &class_desc(c));
    }
    for (i, (_, table)) in m.vtables.iter().enumerate() {
        let _ = writeln!(out, ".balign 8\n.Lvt{}:", i);
        for e in table {
            match e {
                Some(f) => {
                    let _ = writeln!(out, "    .quad f{}", f);
                }
                None => out.push_str("    .quad 0\n"),
            }
        }
        if table.is_empty() {
            out.push_str("    .quad 0\n");
        }
    }
    // қате трассасы кестелері
    let files = crate::diag::file_names();
    out.push_str(".balign 8\ntlp_linetab:\n");
    for (lbl, file, line) in &all_lines {
        let _ = writeln!(out, "    .quad {}\n    .long {}, {}", lbl, file, line);
    }
    out.push_str("tlp_linetab_end:\n.balign 8\ntlp_functab:\n");
    for (i, f) in funcs.iter().enumerate() {
        let _ = writeln!(out, "    .quad {}, .Lfn{}", f.sym, i);
    }
    out.push_str("tlp_functab_end:\n.balign 8\ntlp_filetab:\n");
    for i in 0..files.len() {
        let _ = writeln!(out, "    .quad .Lfile{}", i);
    }
    out.push_str("    .quad 0\n");
    for (i, f) in funcs.iter().enumerate() {
        let _ = writeln!(out, ".Lfn{}:", i);
        bytes_directive(&mut out, f.name.as_bytes());
        out.push_str("    .byte 0\n");
    }
    for (i, f) in files.iter().enumerate() {
        let _ = writeln!(out, ".Lfile{}:", i);
        bytes_directive(&mut out, f.as_bytes());
        out.push_str("    .byte 0\n");
    }
    out.push_str(".balign 8\ntlp_info:\n    .quad tlp_linetab, tlp_linetab_end, tlp_functab, tlp_functab_end, tlp_filetab, tlp_text_end\n");
    out.push_str(".balign 8\ntlp_class_table:\n");
    if m.classes.is_empty() {
        out.push_str("    .quad 0\n");
    }
    for i in 0..m.classes.len() {
        let _ = writeln!(out, "    .quad .Lcls{}", i);
    }
    out.push_str(".section .bss\n.balign 16\n.globl tlp_globals\ntlp_globals:\n");
    let _ = writeln!(out, "    .zero {}", (m.globals.len() * 8).max(8));
    out.push_str("tlp_globals_end:\n");
    out.push_str(".section .note.GNU-stack,\"\",@progbits\n");
    out
}
