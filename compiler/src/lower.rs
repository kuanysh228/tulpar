//! HIR -> IR түсіру.

use std::collections::HashMap;

use crate::ast::{BinOp, CmpOp, UnOp};
use crate::hir::*;
use crate::ir::*;

#[derive(Default)]
pub struct ModuleCtx {
    pub strings: Vec<String>,
    string_ids: HashMap<String, usize>,
    pub descs: Vec<Vec<u8>>,
    desc_ids: HashMap<Vec<u8>, usize>,
}

impl ModuleCtx {
    pub fn str_label(&mut self, s: &str) -> String {
        if let Some(&i) = self.string_ids.get(s) {
            return format!(".Lstr{}", i);
        }
        let i = self.strings.len();
        self.strings.push(s.to_string());
        self.string_ids.insert(s.to_string(), i);
        format!(".Lstr{}", i)
    }
    pub fn desc_label(&mut self, d: Vec<u8>) -> String {
        if let Some(&i) = self.desc_ids.get(&d) {
            return format!(".Ldesc{}", i);
        }
        let i = self.descs.len();
        self.descs.push(d.clone());
        self.desc_ids.insert(d, i);
        format!(".Ldesc{}", i)
    }
}

pub fn rc_of(t: &Ty) -> RC {
    if t.is_float() {
        RC::F
    } else {
        RC::I
    }
}

/// Тізім элементінің жад өлшемі мен енін анықтайды (`логика`/`байт` — 1 байт).
pub fn list_elem(t: &Ty) -> (u8, Width) {
    if matches!(t, Ty::Bool | Ty::Byte) {
        (1, Width::B1)
    } else {
        (8, Width::B8)
    }
}

/// Тізім құруға арналған жалаулар: бит0 — noscan, бит1 — байттық элементтер.
pub fn list_flags(t: &Ty) -> i64 {
    (!t.is_gc_ref()) as i64 | if matches!(t, Ty::Bool | Ty::Byte) { 2 } else { 0 }
}

pub fn elem_size(t: &Ty) -> i64 {
    if *t == Ty::Byte {
        1
    } else {
        8
    }
}

/// Тип дескрипторы (рантаймда басып шығару үшін).
pub fn type_desc(t: &Ty, out: &mut Vec<u8>) {
    match t {
        Ty::Int => out.push(b'i'),
        Ty::Float => out.push(b'f'),
        Ty::Bool => out.push(b'b'),
        Ty::Byte => out.push(b'c'),
        Ty::Str => out.push(b's'),
        Ty::None | Ty::Null => out.push(b'n'),
        Ty::Ptr(_) | Ty::Func(..) => out.push(b'p'),
        Ty::List(e) => {
            out.push(b'L');
            type_desc(e, out);
        }
        Ty::Dict(k, v) => {
            out.push(b'D');
            type_desc(k, out);
            type_desc(v, out);
        }
        Ty::Set(e) => {
            out.push(b'S');
            type_desc(e, out);
        }
        Ty::Class(c) => {
            out.push(b'C');
            out.extend_from_slice(&(*c as u16).to_le_bytes());
        }
        Ty::Tuple(ts) => {
            out.push(b'T');
            out.push(ts.len() as u8);
            for t in ts {
                type_desc(t, out);
            }
        }
    }
}

/// Класс кестесі үшін дескриптор: [атау ұзындығы][атау][өріс саны][ (өріс аты ұзындығы)(атау)(тип дескрипторы) ]*
pub fn class_desc(c: &ClassInfo) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(c.has_header as u8);
    let parent = c.base.map(|b| b as u16).unwrap_or(0xFFFF);
    out.extend_from_slice(&parent.to_le_bytes());
    out.push(c.name.len() as u8);
    out.extend_from_slice(c.name.as_bytes());
    out.push(c.fields.len() as u8);
    for f in &c.fields {
        out.push(f.name.len() as u8);
        out.extend_from_slice(f.name.as_bytes());
        type_desc(&f.ty, &mut out);
    }
    out
}

pub fn lower_module(m: &Module) -> (Vec<IrFunc>, ModuleCtx) {
    let mut cx = ModuleCtx::default();
    let mut out = Vec::new();
    for fid in 0..m.funcs.len() {
        out.push(lower_func(m, &mut cx, fid));
    }
    (out, cx)
}

struct FB<'a> {
    m: &'a Module,
    cx: &'a mut ModuleCtx,
    f: IrFunc,
    cur: BlockId,
    locals: Vec<VReg>,
    /// (continue мақсаты, break мақсаты)
    loops: Vec<(BlockId, BlockId)>,
    cur_line: (u16, u32),
}

fn lower_func(m: &Module, cx: &mut ModuleCtx, fid: FuncId) -> IrFunc {
    let tf = &m.funcs[fid];
    let sym = if fid == m.main { "tlp_main".to_string() } else { format!("f{}", fid) };
    let mut fb = FB {
        m,
        cx,
        f: IrFunc { name: tf.name.clone(), sym, blocks: vec![], classes: vec![] },
        cur: 0,
        locals: vec![],
        loops: vec![],
        cur_line: (0, 0),
    };
    let entry = fb.new_block(false);
    fb.cur = entry;
    let mut ni = 0u8;
    let mut nf = 0u8;
    let mut ns = 0u16;
    for (i, l) in tf.locals.iter().enumerate() {
        let rc = rc_of(&l.ty);
        let v = fb.f.new_vreg(rc);
        fb.locals.push(v);
        if i < tf.nparams {
            let loc = match rc {
                RC::I => {
                    if ni < 6 {
                        ni += 1;
                        ArgLoc::IReg(ni - 1)
                    } else {
                        ns += 1;
                        ArgLoc::Stack(ns - 1)
                    }
                }
                RC::F => {
                    if nf < 8 {
                        nf += 1;
                        ArgLoc::FReg(nf - 1)
                    } else {
                        ns += 1;
                        ArgLoc::Stack(ns - 1)
                    }
                }
            };
            fb.emit(Inst::Param(v, loc));
        }
    }
    for (i, l) in tf.locals.iter().enumerate() {
        if i >= tf.nparams {
            let v = fb.locals[i];
            match rc_of(&l.ty) {
                RC::I => fb.emit(Inst::IConst(v, 0)),
                RC::F => fb.emit(Inst::FConst(v, 0.0)),
            }
        }
    }
    for s in &tf.body {
        fb.stmt(s);
    }
    fb.set_term(Term::Ret(None));
    fb.f.remove_unreachable();
    fb.f
}

impl<'a> FB<'a> {
    fn new_block(&mut self, cold: bool) -> BlockId {
        self.f.blocks.push(Block { insts: vec![], term: Term::Unreachable, cold });
        (self.f.blocks.len() - 1) as BlockId
    }
    fn emit(&mut self, i: Inst) {
        let c = self.cur as usize;
        self.f.blocks[c].insts.push(i);
    }
    fn set_term(&mut self, t: Term) {
        let c = self.cur as usize;
        self.f.blocks[c].term = t;
    }
    /// Терминатор қояды да, әрі қарай шығару үшін жаңа (қол жеткізілмейтін) блок ашады.
    fn terminate(&mut self, t: Term) {
        self.set_term(t);
        let nb = self.new_block(false);
        self.cur = nb;
    }
    fn vi(&mut self) -> VReg {
        self.f.new_vreg(RC::I)
    }
    fn vf(&mut self) -> VReg {
        self.f.new_vreg(RC::F)
    }
    fn vr(&mut self, rc: RC) -> VReg {
        self.f.new_vreg(rc)
    }
    fn iconst(&mut self, v: i64) -> VReg {
        let d = self.vi();
        self.emit(Inst::IConst(d, v));
        d
    }
    fn ibin(&mut self, op: IOp, a: VReg, b: VReg) -> VReg {
        let d = self.vi();
        self.emit(Inst::IBin(op, d, a, b));
        d
    }
    fn ibini(&mut self, op: IOp, a: VReg, imm: i32) -> VReg {
        let d = self.vi();
        self.emit(Inst::IBinI(op, d, a, imm));
        d
    }
    fn symaddr(&mut self, s: &str) -> VReg {
        let d = self.vi();
        self.emit(Inst::SymAddr(d, s.to_string()));
        d
    }
    fn load(&mut self, rc: RC, base: VReg, off: i32) -> VReg {
        let d = self.vr(rc);
        self.emit(Inst::Load { d, base, off, w: Width::B8 });
        d
    }
    fn call(&mut self, sym: &str, args: &[VReg], ret: Option<RC>) -> Option<VReg> {
        let d = ret.map(|rc| self.vr(rc));
        self.emit(Inst::Call { d, sym: sym.to_string(), args: args.to_vec() });
        d
    }
    fn calli(&mut self, sym: &str, args: &[VReg]) -> VReg {
        self.call(sym, args, Some(RC::I)).unwrap()
    }
    fn callf(&mut self, sym: &str, args: &[VReg]) -> VReg {
        self.call(sym, args, Some(RC::F)).unwrap()
    }
    fn callv(&mut self, sym: &str, args: &[VReg]) {
        self.call(sym, args, None);
    }
    fn str_lit(&mut self, s: &str) -> VReg {
        let l = self.cx.str_label(s);
        self.symaddr(&l)
    }
    fn desc_addr(&mut self, t: &Ty) -> VReg {
        let mut d = Vec::new();
        type_desc(t, &mut d);
        let l = self.cx.desc_label(d);
        self.symaddr(&l)
    }
    fn to_bits(&mut self, v: VReg, t: &Ty) -> VReg {
        if t.is_float() {
            let d = self.vi();
            self.emit(Inst::BitsFI(d, v));
            d
        } else {
            v
        }
    }
    fn from_bits(&mut self, v: VReg, t: &Ty) -> VReg {
        if t.is_float() {
            let d = self.vf();
            self.emit(Inst::BitsIF(d, v));
            d
        } else {
            v
        }
    }
    /// Өрістің жад ығысуы (иерархиядағы кластарда 0-ші сөз — класс идентификаторы).
    fn foff(&self, obj_ty: &Ty, i: usize) -> i32 {
        let hdr = match obj_ty {
            Ty::Class(c) => self.m.classes[*c].has_header as usize,
            _ => 0,
        };
        8 * (i + hdr) as i32
    }

    fn val(&mut self, e: &TExpr) -> VReg {
        self.expr(e).expect("мән күтілген")
    }
    fn width(t: &Ty) -> Width {
        if *t == Ty::Byte {
            Width::B1
        } else {
            Width::B8
        }
    }

    // ------------------------------------------------------------ өрнектер

    fn fits32(v: i64) -> bool {
        v >= i32::MIN as i64 && v <= i32::MAX as i64
    }

    fn int_bin(&mut self, op: BinOp, a: &TExpr, b: &TExpr) -> VReg {
        let iop = match op {
            BinOp::Add => IOp::Add,
            BinOp::Sub => IOp::Sub,
            BinOp::Mul => IOp::Mul,
            BinOp::BitAnd => IOp::And,
            BinOp::BitOr => IOp::Or,
            BinOp::BitXor => IOp::Xor,
            BinOp::Shl => IOp::Shl,
            BinOp::Shr => IOp::Sar,
            BinOp::FloorDiv => IOp::FloorDiv,
            BinOp::Mod => IOp::FloorMod,
            _ => unreachable!(),
        };
        // сол жақтағы тұрақты (коммутативті амалдар)
        if iop.commutative() {
            if let (TK::Int(c), false) = (&a.kind, matches!(b.kind, TK::Int(_))) {
                if Self::fits32(*c) {
                    let vb = self.val(b);
                    return self.ibini(iop, vb, *c as i32);
                }
            }
        }
        let va = self.val(a);
        if let TK::Int(c) = b.kind {
            match iop {
                IOp::FloorDiv if c > 0 && (c & (c - 1)) == 0 => {
                    return self.ibini(IOp::Sar, va, c.trailing_zeros() as i32);
                }
                IOp::FloorMod if c > 0 && (c & (c - 1)) == 0 && Self::fits32(c - 1) => {
                    return self.ibini(IOp::And, va, (c - 1) as i32);
                }
                IOp::FloorDiv | IOp::FloorMod => {}
                IOp::Shl | IOp::Sar => {
                    return self.ibini(iop, va, (c & 63) as i32);
                }
                _ => {
                    if Self::fits32(c) {
                        return self.ibini(iop, va, c as i32);
                    }
                }
            }
        }
        let vb = self.val(b);
        self.ibin(iop, va, vb)
    }

    fn cc_of(op: CmpOp) -> Cc {
        match op {
            CmpOp::Eq => Cc::E,
            CmpOp::Ne => Cc::Ne,
            CmpOp::Lt => Cc::L,
            CmpOp::Le => Cc::Le,
            CmpOp::Gt => Cc::G,
            CmpOp::Ge => Cc::Ge,
        }
    }
    fn fcc_of(op: CmpOp) -> FCc {
        match op {
            CmpOp::Eq => FCc::Eq,
            CmpOp::Ne => FCc::Ne,
            CmpOp::Lt => FCc::Lt,
            CmpOp::Le => FCc::Le,
            CmpOp::Gt => FCc::Gt,
            CmpOp::Ge => FCc::Ge,
        }
    }

    fn is_str_cmp(a: &TExpr, b: &TExpr) -> bool {
        a.ty == Ty::Str && !matches!(a.kind, TK::Null) && !matches!(b.kind, TK::Null)
    }

    fn cmp_value(&mut self, op: CmpOp, a: &TExpr, b: &TExpr) -> VReg {
        if a.ty.is_float() {
            let va = self.val(a);
            let vb = self.val(b);
            let d = self.vi();
            self.emit(Inst::FCmp(Self::fcc_of(op), d, va, vb));
            return d;
        }
        if Self::is_str_cmp(a, b) {
            let va = self.val(a);
            let vb = self.val(b);
            return match op {
                CmpOp::Eq => self.calli("tlp_str_eq", &[va, vb]),
                CmpOp::Ne => {
                    let r = self.calli("tlp_str_eq", &[va, vb]);
                    self.ibini(IOp::Xor, r, 1)
                }
                _ => {
                    let r = self.calli("tlp_str_cmp", &[va, vb]);
                    let d = self.vi();
                    self.emit(Inst::ICmpI(Self::cc_of(op), d, r, 0));
                    d
                }
            };
        }
        let va = self.val(a);
        if let TK::Int(c) = b.kind {
            if Self::fits32(c) {
                let d = self.vi();
                self.emit(Inst::ICmpI(Self::cc_of(op), d, va, c as i32));
                return d;
            }
        }
        let vb = self.val(b);
        let d = self.vi();
        self.emit(Inst::ICmp(Self::cc_of(op), d, va, vb));
        d
    }

    /// Шартты тікелей тармақталуға түсіреді.
    fn cond(&mut self, e: &TExpr, t: BlockId, f: BlockId) {
        match &e.kind {
            TK::Bool(true) => self.set_term(Term::Jmp(t)),
            TK::Bool(false) => self.set_term(Term::Jmp(f)),
            TK::Unary(UnOp::Not, x) => self.cond(x, f, t),
            TK::And(a, b) => {
                let mid = self.new_block(false);
                self.cond(a, mid, f);
                self.cur = mid;
                self.cond(b, t, f);
            }
            TK::Or(a, b) => {
                let mid = self.new_block(false);
                self.cond(a, t, mid);
                self.cur = mid;
                self.cond(b, t, f);
            }
            TK::Cmp(op, a, b) if a.ty.is_float() => {
                let va = self.val(a);
                let vb = self.val(b);
                self.set_term(Term::BrF { cc: Self::fcc_of(*op), a: va, b: vb, t, f });
            }
            TK::Cmp(op, a, b) if !Self::is_str_cmp(a, b) => {
                let va = self.val(a);
                if let TK::Int(c) = b.kind {
                    if Self::fits32(c) {
                        self.set_term(Term::BrI { cc: Self::cc_of(*op), a: va, imm: c as i32, t, f });
                        return;
                    }
                }
                let vb = self.val(b);
                self.set_term(Term::Br { cc: Self::cc_of(*op), a: va, b: vb, t, f });
            }
            _ => {
                let v = self.val(e);
                self.set_term(Term::BrI { cc: Cc::Ne, a: v, imm: 0, t, f });
            }
        }
    }

    fn bounds_check(&mut self, i: VReg, len: VReg) {
        let cold = self.new_block(true);
        let ok = self.new_block(false);
        self.set_term(Term::Br { cc: Cc::Ae, a: i, b: len, t: cold, f: ok });
        self.cur = cold;
        self.emit(Inst::Line(self.cur_line.0, self.cur_line.1));
        self.callv("tlp_panic_index", &[i, len]);
        self.set_term(Term::Unreachable);
        self.cur = ok;
    }

    /// Индекс мәнін есептейді; теріс тұрақты индекс ұзындыққа қатысты.
    fn index_val(&mut self, idx: &TExpr, len: VReg) -> VReg {
        if let TK::Int(c) = idx.kind {
            if c < 0 && Self::fits32(c) {
                return self.ibini(IOp::Add, len, c as i32);
            }
        }
        self.val(idx)
    }

    fn expr(&mut self, e: &TExpr) -> Option<VReg> {
        let ty = &e.ty;
        Some(match &e.kind {
            TK::Int(v) => self.iconst(*v),
            TK::Float(v) => {
                let d = self.vf();
                self.emit(Inst::FConst(d, *v));
                d
            }
            TK::Bool(b) => self.iconst(*b as i64),
            TK::Null => self.iconst(0),
            TK::Str(s) => self.str_lit(s),
            TK::Local(l) => self.locals[*l],
            TK::Global(g) => {
                let d = self.vr(rc_of(ty));
                self.emit(Inst::LoadGlobal(d, 8 * *g as i32));
                d
            }
            TK::Unary(op, a) => {
                let v = self.val(a);
                match op {
                    UnOp::Neg => {
                        let d = self.vr(rc_of(ty));
                        if ty.is_float() {
                            self.emit(Inst::FNeg(d, v));
                        } else {
                            self.emit(Inst::INeg(d, v));
                        }
                        d
                    }
                    UnOp::Not => self.ibini(IOp::Xor, v, 1),
                    UnOp::BitNot => {
                        let d = self.vi();
                        self.emit(Inst::INot(d, v));
                        d
                    }
                    _ => unreachable!(),
                }
            }
            TK::Binary(op, a, b) => match &a.ty {
                Ty::Int | Ty::Byte | Ty::Bool => self.int_bin(*op, a, b),
                Ty::Float => {
                    let va = self.val(a);
                    let vb = self.val(b);
                    let d = self.vf();
                    let fop = match op {
                        BinOp::Add => FOp::Add,
                        BinOp::Sub => FOp::Sub,
                        BinOp::Mul => FOp::Mul,
                        BinOp::Div => FOp::Div,
                        _ => unreachable!(),
                    };
                    self.emit(Inst::FBin(fop, d, va, vb));
                    d
                }
                Ty::Str => {
                    let va = self.val(a);
                    let vb = self.val(b);
                    if *op == BinOp::Add {
                        self.calli("tlp_str_concat", &[va, vb])
                    } else {
                        self.calli("tlp_str_repeat", &[va, vb])
                    }
                }
                Ty::List(_) => {
                    let va = self.val(a);
                    let vb = self.val(b);
                    if *op == BinOp::Add {
                        self.calli("tlp_list_concat", &[va, vb])
                    } else {
                        self.calli("tlp_list_repeat", &[va, vb])
                    }
                }
                Ty::Ptr(t) => {
                    let va = self.val(a);
                    let vb = self.val(b);
                    let sz = elem_size(t);
                    let off = if sz == 1 { vb } else { self.ibini(IOp::Shl, vb, 3) };
                    self.ibin(if *op == BinOp::Add { IOp::Add } else { IOp::Sub }, va, off)
                }
                other => unreachable!("binary on {:?}", other),
            },
            TK::Cmp(op, a, b) => self.cmp_value(*op, a, b),
            TK::And(..) | TK::Or(..) => {
                let d = self.vi();
                let t = self.new_block(false);
                let f = self.new_block(false);
                let end = self.new_block(false);
                self.cond(e, t, f);
                self.cur = t;
                self.emit(Inst::IConst(d, 1));
                self.set_term(Term::Jmp(end));
                self.cur = f;
                self.emit(Inst::IConst(d, 0));
                self.set_term(Term::Jmp(end));
                self.cur = end;
                d
            }
            TK::Cast(k, a) => {
                let v = self.val(a);
                match k {
                    CastKind::IntToFloat => {
                        let d = self.vf();
                        self.emit(Inst::I2F(d, v));
                        d
                    }
                    CastKind::FloatToInt => {
                        let d = self.vi();
                        self.emit(Inst::F2I(d, v));
                        d
                    }
                    CastKind::IntToByte => self.ibini(IOp::And, v, 255),
                    CastKind::ByteToInt | CastKind::BoolToInt | CastKind::Reinterpret => v,
                }
            }
            TK::Call(fid, args) => {
                let vs: Vec<VReg> = args.iter().map(|a| self.val(a)).collect();
                let ret = &self.m.funcs[*fid].ret;
                let rc = if ret.is_unit() { None } else { Some(rc_of(ret)) };
                let sym = format!("f{}", fid);
                return self.call(&sym, &vs, rc);
            }
            TK::VCall(name, args) => {
                let vs: Vec<VReg> = args.iter().map(|a| self.val(a)).collect();
                let idx = self.m.vtables.iter().position(|(n, _)| n == name).expect("vtable");
                let id = self.load(RC::I, vs[0], 0);
                let tab = self.symaddr(&format!(".Lvt{}", idx));
                let fp = self.vi();
                self.emit(Inst::LoadIdx { d: fp, base: tab, idx: id, scale: 8, off: 0, w: Width::B8 });
                let d = if ty.is_unit() { None } else { Some(self.vr(rc_of(ty))) };
                self.emit(Inst::CallInd { d, target: fp, args: vs });
                return d;
            }
            TK::Builtin(bi, args) => return self.builtin(bi, args, ty),
            TK::Index(base, idx) => return self.index(base, idx, ty),
            TK::Slice(base, lo, hi) => {
                let b = self.val(base);
                let l = match lo {
                    Some(x) => self.val(x),
                    None => self.iconst(0),
                };
                let h = match hi {
                    Some(x) => self.val(x),
                    None => self.iconst(i64::MAX),
                };
                let f = if base.ty == Ty::Str { "tlp_str_slice" } else { "tlp_list_slice" };
                self.calli(f, &[b, l, h])
            }
            TK::Field(obj, i) => {
                let o = self.val(obj);
                let off = self.foff(&obj.ty, *i);
                self.load(rc_of(ty), o, off)
            }
            TK::ListLit(items) => {
                let et = match ty {
                    Ty::List(t) => (**t).clone(),
                    _ => unreachable!(),
                };
                let n = self.iconst(items.len() as i64);
                let fl = self.iconst(list_flags(&et));
                let l = self.calli("tlp_list_new_n", &[n, fl]);
                if !items.is_empty() {
                    let data = self.load(RC::I, l, 16);
                    let (sc, w) = list_elem(&et);
                    for (i, it) in items.iter().enumerate() {
                        let v = self.val(it);
                        self.emit(Inst::Store { base: data, off: sc as i32 * i as i32, s: v, w });
                    }
                }
                l
            }
            TK::DictLit(items) => {
                let (kt, vt) = match ty {
                    Ty::Dict(k, v) => ((**k).clone(), (**v).clone()),
                    _ => unreachable!(),
                };
                let d = self.new_dict(&kt, &vt);
                for (k, v) in items {
                    let kv = self.val(k);
                    let vv = self.val(v);
                    let vb = self.to_bits(vv, &vt);
                    self.callv("tlp_dict_set", &[d, kv, vb]);
                }
                d
            }
            TK::SetLit(items) => {
                let kt = match ty {
                    Ty::Set(k) => (**k).clone(),
                    _ => unreachable!(),
                };
                let d = self.new_dict(&kt, &Ty::Bool);
                for it in items {
                    let k = self.val(it);
                    let z = self.iconst(0);
                    self.callv("tlp_dict_set", &[d, k, z]);
                }
                d
            }
            TK::TupleLit(items) => {
                let ts = match ty {
                    Ty::Tuple(ts) => ts.clone(),
                    _ => unreachable!(),
                };
                let size = self.iconst(8 * items.len() as i64);
                let fl = self.iconst(ts.iter().all(|t| !t.is_gc_ref()) as i64);
                let obj = self.calli("tlp_alloc", &[size, fl]);
                for (i, it) in items.iter().enumerate() {
                    let v = self.val(it);
                    self.emit(Inst::Store { base: obj, off: 8 * i as i32, s: v, w: Width::B8 });
                }
                obj
            }
            TK::TupleGet(t, i) => {
                let tv = self.val(t);
                self.load(rc_of(ty), tv, 8 * *i as i32)
            }
            TK::Block(stmts, res) => {
                for s in stmts {
                    self.stmt(s);
                }
                return self.expr(res);
            }
            TK::FuncRef(fid) => self.symaddr(&format!("f{}", fid)),
            TK::CallValue(callee, args) => {
                let target = self.val(callee);
                let vs: Vec<VReg> = args.iter().map(|a| self.val(a)).collect();
                let d = if ty.is_unit() { None } else { Some(self.vr(rc_of(ty))) };
                self.emit(Inst::CallInd { d, target, args: vs });
                return d;
            }
            TK::DeepEq(a, b, neg) => {
                let va = self.val(a);
                let vb = self.val(b);
                let d = self.desc_addr(&a.ty);
                let r = self.calli("tlp_deep_eq", &[va, vb, d]);
                if *neg {
                    self.ibini(IOp::Xor, r, 1)
                } else {
                    r
                }
            }
            TK::New { class, fields, ctor } => {
                let ci = &self.m.classes[*class];
                let hdr = ci.has_header as usize;
                let size = (8 * (ci.fields.len() + hdr)).max(8) as i64;
                let noscan = ci.fields.iter().all(|f| !f.ty.is_gc_ref()) as i64;
                let sz = self.iconst(size);
                let fl = self.iconst(noscan);
                let obj = self.calli("tlp_alloc", &[sz, fl]);
                if hdr == 1 {
                    let idc = self.iconst(*class as i64);
                    self.emit(Inst::Store { base: obj, off: 0, s: idc, w: Width::B8 });
                }
                let cty = Ty::Class(*class);
                for (i, f) in fields.iter().enumerate() {
                    if let Some(fe) = f {
                        let v = self.val(fe);
                        let off = self.foff(&cty, i);
                        self.emit(Inst::Store { base: obj, off, s: v, w: Width::B8 });
                    }
                }
                if let Some((fid, args)) = ctor {
                    let mut vs = vec![obj];
                    for a in args {
                        vs.push(self.val(a));
                    }
                    let sym = format!("f{}", fid);
                    self.callv(&sym, &vs);
                }
                obj
            }
            TK::Deref(p) => {
                let pv = self.val(p);
                let d = self.vr(rc_of(ty));
                self.emit(Inst::Load { d, base: pv, off: 0, w: Self::width(ty) });
                d
            }
            TK::Ternary(c, a, b) => {
                let d = self.vr(rc_of(ty));
                let t = self.new_block(false);
                let f = self.new_block(false);
                let end = self.new_block(false);
                self.cond(c, t, f);
                self.cur = t;
                let va = self.val(a);
                self.emit(Inst::Mov(d, va));
                self.set_term(Term::Jmp(end));
                self.cur = f;
                let vb = self.val(b);
                self.emit(Inst::Mov(d, vb));
                self.set_term(Term::Jmp(end));
                self.cur = end;
                d
            }
            TK::FStr(parts) => {
                let mut acc: Option<VReg> = None;
                for p in parts {
                    let v = match p {
                        FPartT::Lit(s) => {
                            if s.is_empty() {
                                continue;
                            }
                            self.str_lit(s)
                        }
                        FPartT::Val(e) => self.val(e),
                    };
                    acc = Some(match acc {
                        None => v,
                        Some(a) => self.calli("tlp_str_concat", &[a, v]),
                    });
                }
                match acc {
                    Some(a) => a,
                    None => self.str_lit(""),
                }
            }
            TK::Contains { item, container, negated } => {
                let c = self.val(container);
                let i = self.val(item);
                let r = match &container.ty {
                    Ty::Str => self.calli("tlp_str_contains", &[c, i]),
                    Ty::List(t) => {
                        let kind = match **t {
                            Ty::Str => 1,
                            Ty::Float => 2,
                            _ => 0,
                        };
                        let k = self.iconst(kind);
                        let ib = self.to_bits(i, t);
                        self.calli("tlp_list_contains", &[c, ib, k])
                    }
                    Ty::Dict(..) | Ty::Set(_) => self.calli("tlp_dict_has", &[c, i]),
                    _ => unreachable!(),
                };
                if *negated {
                    self.ibini(IOp::Xor, r, 1)
                } else {
                    r
                }
            }
        })
    }

    fn new_dict(&mut self, kt: &Ty, vt: &Ty) -> VReg {
        let kind = self.iconst((*kt == Ty::Str) as i64);
        let noscan = !(*kt == Ty::Str || vt.is_gc_ref());
        let fl = self.iconst(noscan as i64);
        self.calli("tlp_dict_new", &[kind, fl])
    }

    fn index(&mut self, base: &TExpr, idx: &TExpr, ty: &Ty) -> Option<VReg> {
        Some(match &base.ty {
            Ty::List(et) => {
                let l = self.val(base);
                let len = self.load(RC::I, l, 0);
                let i = self.index_val(idx, len);
                self.bounds_check(i, len);
                let data = self.load(RC::I, l, 16);
                let d = self.vr(rc_of(ty));
                let (scale, w) = list_elem(et);
                self.emit(Inst::LoadIdx { d, base: data, idx: i, scale, off: 0, w });
                d
            }
            Ty::Str => {
                let s = self.val(base);
                let i = self.val(idx);
                self.calli("tlp_str_index", &[s, i])
            }
            Ty::Ptr(t) => {
                let p = self.val(base);
                let i = self.val(idx);
                let d = self.vr(rc_of(ty));
                self.emit(Inst::LoadIdx { d, base: p, idx: i, scale: elem_size(t) as u8, off: 0, w: Self::width(t) });
                d
            }
            Ty::Dict(..) => {
                let d = self.val(base);
                let k = self.val(idx);
                let r = self.calli("tlp_dict_get", &[d, k]);
                self.from_bits(r, ty)
            }
            _ => unreachable!(),
        })
    }

    // ------------------------------------------------------------ ішкі функциялар

    fn builtin(&mut self, bi: &Bi, args: &[TExpr], ty: &Ty) -> Option<VReg> {
        Some(match bi {
            Bi::Print { nl } => {
                // Python сияқты: алдымен барлық аргумент есептеледі, содан кейін басылады
                let vals: Vec<VReg> = args.iter().map(|a| self.val(a)).collect();
                for (i, (a, v)) in args.iter().zip(vals).enumerate() {
                    if i > 0 {
                        self.callv("tlp_print_sep", &[]);
                    }
                    match &a.ty {
                        Ty::Int | Ty::Byte => self.callv("tlp_print_int", &[v]),
                        Ty::Float => self.callv("tlp_print_float", &[v]),
                        Ty::Bool => self.callv("tlp_print_bool", &[v]),
                        Ty::Str => self.callv("tlp_print_str", &[v]),
                        t => {
                            let bits = self.to_bits(v, t);
                            let d = self.desc_addr(t);
                            self.callv("tlp_print_any", &[bits, d]);
                        }
                    }
                }
                if *nl {
                    self.callv("tlp_print_nl", &[]);
                }
                return None;
            }
            Bi::Input => {
                let p = match args.first() {
                    Some(a) => self.val(a),
                    None => self.iconst(0),
                };
                self.calli("tlp_input", &[p])
            }
            Bi::ToStr => {
                let a = &args[0];
                let v = self.val(a);
                match &a.ty {
                    Ty::Int | Ty::Byte => self.calli("tlp_str_from_int", &[v]),
                    Ty::Float => self.calli("tlp_str_from_float", &[v]),
                    Ty::Bool => self.calli("tlp_str_from_bool", &[v]),
                    t => {
                        let bits = self.to_bits(v, t);
                        let d = self.desc_addr(t);
                        self.calli("tlp_to_str", &[bits, d])
                    }
                }
            }
            Bi::StrToInt => {
                let v = self.val(&args[0]);
                self.calli("tlp_str_to_int", &[v])
            }
            Bi::StrToFloat => {
                let v = self.val(&args[0]);
                self.callf("tlp_str_to_float", &[v])
            }
            Bi::FloatFixed(p) => {
                let v = self.val(&args[0]);
                let pv = self.iconst(*p as i64);
                self.calli("tlp_str_fixed", &[v, pv])
            }
            Bi::StrPad(al) => {
                let s = self.val(&args[0]);
                let w = self.val(&args[1]);
                let a = self.iconst(*al as i64);
                self.calli("tlp_str_pad", &[s, w, a])
            }
            Bi::AbsI => {
                let v = self.val(&args[0]);
                let d = self.vi();
                self.emit(Inst::IAbs(d, v));
                d
            }
            Bi::AbsF => {
                let v = self.val(&args[0]);
                let d = self.vf();
                self.emit(Inst::FAbs(d, v));
                d
            }
            Bi::MinI | Bi::MaxI => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                let d = self.vi();
                self.emit(if *bi == Bi::MinI { Inst::IMin(d, a, b) } else { Inst::IMax(d, a, b) });
                d
            }
            Bi::MinF | Bi::MaxF => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                let d = self.vf();
                self.emit(Inst::Mov(d, a));
                let set = self.new_block(false);
                let end = self.new_block(false);
                let cc = if *bi == Bi::MinF { FCc::Lt } else { FCc::Gt };
                self.set_term(Term::BrF { cc, a: b, b: a, t: set, f: end });
                self.cur = set;
                self.emit(Inst::Mov(d, b));
                self.set_term(Term::Jmp(end));
                self.cur = end;
                d
            }
            Bi::Sqrt => {
                let v = self.val(&args[0]);
                let d = self.vf();
                self.emit(Inst::FSqrt(d, v));
                d
            }
            Bi::Floor | Bi::Ceil => {
                let v = self.val(&args[0]);
                let d = self.vf();
                self.emit(Inst::FRound(d, v, (*bi == Bi::Ceil) as u8));
                d
            }
            Bi::PowI => {
                if let TK::Int(c) = args[1].kind {
                    if (0..=4).contains(&c) {
                        let x = self.val(&args[0]);
                        return Some(match c {
                            0 => self.iconst(1),
                            1 => x,
                            2 => self.ibin(IOp::Mul, x, x),
                            3 => {
                                let s = self.ibin(IOp::Mul, x, x);
                                self.ibin(IOp::Mul, s, x)
                            }
                            _ => {
                                let s = self.ibin(IOp::Mul, x, x);
                                self.ibin(IOp::Mul, s, s)
                            }
                        });
                    }
                }
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_pow_i", &[a, b])
            }
            Bi::PowF => {
                if let TK::Float(c) = args[1].kind {
                    if c == 2.0 {
                        let x = self.val(&args[0]);
                        let d = self.vf();
                        self.emit(Inst::FBin(FOp::Mul, d, x, x));
                        return Some(d);
                    }
                    if c == 0.5 {
                        let x = self.val(&args[0]);
                        let d = self.vf();
                        self.emit(Inst::FSqrt(d, x));
                        return Some(d);
                    }
                }
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.callf("tlp_pow_f", &[a, b])
            }
            Bi::FMod => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.callf("tlp_fmod", &[a, b])
            }
            Bi::Len => {
                let v = self.val(&args[0]);
                match &args[0].ty {
                    Ty::Dict(..) | Ty::Set(_) => self.calli("tlp_dict_len", &[v]),
                    Ty::Str => self.load(RC::I, v, 8),
                    _ => self.load(RC::I, v, 0),
                }
            }
            Bi::StrUpper => {
                let v = self.val(&args[0]);
                self.calli("tlp_str_upper", &[v])
            }
            Bi::StrLower => {
                let v = self.val(&args[0]);
                self.calli("tlp_str_lower", &[v])
            }
            Bi::StrStrip => {
                let v = self.val(&args[0]);
                self.calli("tlp_str_strip", &[v])
            }
            Bi::StrSplit => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_str_split", &[a, b])
            }
            Bi::StrJoin => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_str_join", &[a, b])
            }
            Bi::StrFind => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_str_find", &[a, b])
            }
            Bi::StrReplace => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                let c = self.val(&args[2]);
                self.calli("tlp_str_replace", &[a, b, c])
            }
            Bi::StrByte => {
                let sv = self.val(&args[0]);
                let i = self.val(&args[1]);
                let len = self.load(RC::I, sv, 0);
                self.bounds_check(i, len);
                let d = self.vi();
                self.emit(Inst::LoadIdx { d, base: sv, idx: i, scale: 1, off: 16, w: Width::B1 });
                d
            }
            Bi::StrStarts => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_str_starts", &[a, b])
            }
            Bi::StrEnds => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_str_ends", &[a, b])
            }
            Bi::ListPush => {
                let l = self.val(&args[0]);
                let v = self.val(&args[1]);
                let b = self.to_bits(v, &args[1].ty);
                self.callv("tlp_list_push", &[l, b]);
                return None;
            }
            Bi::ListPop => {
                let l = self.val(&args[0]);
                let r = self.calli("tlp_list_pop", &[l]);
                self.from_bits(r, ty)
            }
            Bi::ListClear => {
                let l = self.val(&args[0]);
                self.callv("tlp_list_clear", &[l]);
                return None;
            }
            Bi::ListReverse => {
                let l = self.val(&args[0]);
                self.callv("tlp_list_reverse", &[l]);
                return None;
            }
            Bi::ListSort => {
                let l = self.val(&args[0]);
                let kind = match &args[0].ty {
                    Ty::List(t) => match **t {
                        Ty::Float => 1,
                        Ty::Str => 2,
                        _ => 0,
                    },
                    _ => 0,
                };
                let k = self.iconst(kind);
                self.callv("tlp_list_sort", &[l, k]);
                return None;
            }
            Bi::DictDel => {
                let d = self.val(&args[0]);
                let k = self.val(&args[1]);
                self.callv("tlp_dict_del", &[d, k]);
                return None;
            }
            Bi::DictGetOr => {
                let d = self.val(&args[0]);
                let k = self.val(&args[1]);
                let df = self.val(&args[2]);
                let db = self.to_bits(df, &args[2].ty);
                let r = self.calli("tlp_dict_get_or", &[d, k, db]);
                self.from_bits(r, ty)
            }
            Bi::DictKeys | Bi::DictValues => {
                let d = self.val(&args[0]);
                let which = (*bi == Bi::DictValues) as i64;
                let w = self.iconst(which);
                let fl = match ty {
                    Ty::List(t) => list_flags(t),
                    _ => 0,
                };
                let fl = self.iconst(fl);
                self.calli("tlp_dict_list", &[d, w, fl])
            }
            Bi::StrBytes => {
                let s = self.val(&args[0]);
                self.load(RC::I, s, 0)
            }
            Bi::NetListen | Bi::NetListenLocal | Bi::NetAccept | Bi::NetRead => {
                let a = self.val(&args[0]);
                let f = match bi {
                    Bi::NetListen => "tlp_net_listen",
                    Bi::NetListenLocal => "tlp_net_listen_local",
                    Bi::NetAccept => "tlp_net_accept",
                    _ => "tlp_net_read",
                };
                self.calli(f, &[a])
            }
            Bi::NetClose => {
                let a = self.val(&args[0]);
                self.callv("tlp_net_close", &[a]);
                return None;
            }
            Bi::NetConnect => {
                let h = self.val(&args[0]);
                let p = self.val(&args[1]);
                self.calli("tlp_net_connect", &[h, p])
            }
            Bi::NetWrite => {
                let fd = self.val(&args[0]);
                let t = self.val(&args[1]);
                self.callv("tlp_net_write", &[fd, t]);
                return None;
            }
            Bi::ListSortBy(kind) => {
                let l = self.val(&args[0]);
                let k = self.val(&args[1]);
                let kk = self.iconst(*kind as i64);
                self.callv("tlp_list_sort_by", &[l, k, kk]);
                return None;
            }
            Bi::Ord => {
                let s = self.val(&args[0]);
                self.calli("tlp_ord", &[s])
            }
            Bi::Chr => {
                let n = self.val(&args[0]);
                self.calli("tlp_chr", &[n])
            }
            Bi::StrIsDigit => {
                let s = self.val(&args[0]);
                self.calli("tlp_str_isdigit", &[s])
            }
            Bi::StrIsAlpha => {
                let s = self.val(&args[0]);
                self.calli("tlp_str_isalpha", &[s])
            }
            Bi::StrSplitWs => {
                let s = self.val(&args[0]);
                self.calli("tlp_str_split_ws", &[s])
            }
            Bi::StrCount => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_str_count", &[a, b])
            }
            Bi::SetAdd => {
                let d = self.val(&args[0]);
                let k = self.val(&args[1]);
                let z = self.iconst(0);
                self.callv("tlp_dict_set", &[d, k, z]);
                return None;
            }
            Bi::SetOp(k) => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                let kk = self.iconst(*k as i64);
                self.calli("tlp_set_op", &[a, b, kk])
            }
            Bi::SetFromList => {
                let l = self.val(&args[0]);
                let kt = match &args[0].ty {
                    Ty::List(t) => (**t).clone(),
                    _ => unreachable!(),
                };
                let kind = self.iconst((kt == Ty::Str) as i64);
                let fl = self.iconst((kt != Ty::Str) as i64);
                self.calli("tlp_set_from_list", &[l, kind, fl])
            }
            Bi::Alloc(t) => {
                let n = self.val(&args[0]);
                let sz = if elem_size(t) == 1 { n } else { self.ibini(IOp::Shl, n, 3) };
                let fl = self.iconst(t.is_gc_ref() as i64);
                self.calli("tlp_mem_alloc", &[sz, fl])
            }
            Bi::Free => {
                let p = self.val(&args[0]);
                self.callv("tlp_mem_free", &[p]);
                return None;
            }
            Bi::Args => self.calli("tlp_args", &[]),
            Bi::FileRead => {
                let p = self.val(&args[0]);
                self.calli("tlp_file_read", &[p])
            }
            Bi::FileWrite(app) => {
                let p = self.val(&args[0]);
                let t = self.val(&args[1]);
                let a = self.iconst(*app as i64);
                self.callv("tlp_file_write", &[p, t, a]);
                return None;
            }
            Bi::FileExists => {
                let p = self.val(&args[0]);
                self.calli("tlp_file_exists", &[p])
            }
            Bi::FileDelete | Bi::MakeDir => {
                let p = self.val(&args[0]);
                let f = if matches!(bi, Bi::FileDelete) { "tlp_file_delete" } else { "tlp_make_dir" };
                self.callv(f, &[p]);
                return None;
            }
            Bi::FileRename => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.callv("tlp_file_rename", &[a, b]);
                return None;
            }
            Bi::NetTimeout => {
                let fd = self.val(&args[0]);
                let ms = self.val(&args[1]);
                self.callv("tlp_net_timeout", &[fd, ms]);
                return None;
            }
            Bi::Time => self.callf("tlp_time", &[]),
            Bi::Rand => self.callf("tlp_rand", &[]),
            Bi::RandRange => {
                let a = self.val(&args[0]);
                let b = self.val(&args[1]);
                self.calli("tlp_rand_range", &[a, b])
            }
            Bi::Seed => {
                let a = self.val(&args[0]);
                self.callv("tlp_seed", &[a]);
                return None;
            }
            Bi::Exit => {
                let a = self.val(&args[0]);
                self.callv("tlp_exit", &[a]);
                return None;
            }
            Bi::Math(k) => {
                let names = ["tlp_sin", "tlp_cos", "tlp_tan", "tlp_atan", "tlp_exp", "tlp_ln", "tlp_log10", "tlp_atan2"];
                let vs: Vec<VReg> = args.iter().map(|a| self.val(a)).collect();
                self.callf(names[*k as usize], &vs)
            }
            Bi::ListIndex => {
                let l = self.val(&args[0]);
                let x = self.val(&args[1]);
                let kind = match &args[1].ty {
                    Ty::Str => 1,
                    Ty::Float => 2,
                    _ => 0,
                };
                let k = self.iconst(kind);
                let xb = self.to_bits(x, &args[1].ty);
                self.calli("tlp_list_index", &[l, xb, k])
            }
            Bi::ListRemove => {
                let l = self.val(&args[0]);
                let i = self.val(&args[1]);
                let r = self.calli("tlp_list_remove", &[l, i]);
                self.from_bits(r, ty)
            }
            Bi::ListInsert => {
                let l = self.val(&args[0]);
                let i = self.val(&args[1]);
                let x = self.val(&args[2]);
                let xb = self.to_bits(x, &args[2].ty);
                self.callv("tlp_list_insert", &[l, i, xb]);
                return None;
            }
            Bi::ListCopy => {
                let l = self.val(&args[0]);
                let lo = self.iconst(0);
                let hi = self.iconst(i64::MAX);
                self.calli("tlp_list_slice", &[l, lo, hi])
            }
            Bi::ListSum | Bi::ListMin | Bi::ListMax => {
                let l = self.val(&args[0]);
                let (op, is_f) = match bi {
                    Bi::ListSum => (0, ty.is_float()),
                    Bi::ListMin => (1, ty.is_float()),
                    _ => (2, ty.is_float()),
                };
                let o = self.iconst(op);
                if is_f {
                    self.callf("tlp_list_fold_f", &[l, o])
                } else {
                    self.calli("tlp_list_fold_i", &[l, o])
                }
            }
            Bi::IsInstance(cid) => {
                let o = self.val(&args[0]);
                if !self.m.classes[*cid].has_header {
                    let d = self.vi();
                    self.emit(Inst::ICmpI(Cc::Ne, d, o, 0));
                    return Some(d);
                }
                let c = self.iconst(*cid as i64);
                self.calli("tlp_isa", &[o, c])
            }
            Bi::Downcast(cid) => {
                let o = self.val(&args[0]);
                if !self.m.classes[*cid].has_header {
                    return Some(o);
                }
                let c = self.iconst(*cid as i64);
                self.calli("tlp_downcast", &[o, c])
            }
            Bi::Gc => {
                self.callv("tlp_gc_collect", &[]);
                return None;
            }
            Bi::GcHeap => self.calli("tlp_heap_bytes", &[]),
            Bi::Panic => {
                let m = self.val(&args[0]);
                self.callv("tlp_panic_str", &[m]);
                return None;
            }
        })
    }

    // ------------------------------------------------------------ операторлар

    fn stmt(&mut self, s: &TStmt) {
        match s {
            TStmt::Expr(e) => {
                self.expr(e);
            }
            TStmt::SetLocal(l, e) => {
                let dst = self.locals[*l];
                match &e.kind {
                    TK::Int(v) => self.emit(Inst::IConst(dst, *v)),
                    TK::Bool(b) => self.emit(Inst::IConst(dst, *b as i64)),
                    TK::Null => self.emit(Inst::IConst(dst, 0)),
                    TK::Float(v) => self.emit(Inst::FConst(dst, *v)),
                    _ => {
                        let v = self.val(e);
                        if v != dst {
                            self.emit(Inst::Mov(dst, v));
                        }
                    }
                }
            }
            TStmt::SetGlobal(g, e) => {
                let v = self.val(e);
                self.emit(Inst::StoreGlobal(8 * *g as i32, v));
            }
            TStmt::SetIndex(b, i, v) => match &b.ty {
                Ty::List(et) => {
                    let l = self.val(b);
                    let len = self.load(RC::I, l, 0);
                    let iv = self.index_val(i, len);
                    let vv = self.val(v);
                    self.bounds_check(iv, len);
                    let data = self.load(RC::I, l, 16);
                    let (scale, w) = list_elem(et);
                    self.emit(Inst::StoreIdx { base: data, idx: iv, scale, off: 0, s: vv, w });
                }
                Ty::Ptr(t) => {
                    let p = self.val(b);
                    let iv = self.val(i);
                    let vv = self.val(v);
                    self.emit(Inst::StoreIdx { base: p, idx: iv, scale: elem_size(t) as u8, off: 0, s: vv, w: Self::width(t) });
                }
                Ty::Dict(_, vt) => {
                    let d = self.val(b);
                    let k = self.val(i);
                    let vv = self.val(v);
                    let bits = self.to_bits(vv, vt);
                    self.callv("tlp_dict_set", &[d, k, bits]);
                }
                _ => unreachable!(),
            },
            TStmt::SetField(o, i, v) => {
                let ov = self.val(o);
                let vv = self.val(v);
                let off = self.foff(&o.ty, *i);
                self.emit(Inst::Store { base: ov, off, s: vv, w: Width::B8 });
            }
            TStmt::SetDeref(p, v) => {
                let pv = self.val(p);
                let vv = self.val(v);
                let w = Self::width(&v.ty);
                self.emit(Inst::Store { base: pv, off: 0, s: vv, w });
            }
            TStmt::If { cond, then, els } => {
                let tb = self.new_block(false);
                let eb = if els.is_empty() { None } else { Some(self.new_block(false)) };
                let end = self.new_block(false);
                self.cond(cond, tb, eb.unwrap_or(end));
                self.cur = tb;
                for s in then {
                    self.stmt(s);
                }
                self.set_term(Term::Jmp(end));
                if let Some(eb) = eb {
                    self.cur = eb;
                    for s in els {
                        self.stmt(s);
                    }
                    self.set_term(Term::Jmp(end));
                }
                self.cur = end;
            }
            TStmt::While { cond, body } => {
                // цикл инверсиясы: шарт басында бір рет және соңында тексеріледі
                let bb = self.new_block(false);
                let cont = self.new_block(false);
                let end = self.new_block(false);
                self.cond(cond, bb, end);
                self.cur = bb;
                self.loops.push((cont, end));
                for s in body {
                    self.stmt(s);
                }
                self.loops.pop();
                self.set_term(Term::Jmp(cont));
                self.cur = cont;
                self.cond(cond, bb, end);
                self.cur = end;
            }
            TStmt::ForRange { var, start, stop, step, body } => {
                let c = self.vi();
                let sv = self.val(start);
                self.emit(Inst::Mov(c, sv));
                let stopv = {
                    let v = self.val(stop);
                    // шекті бекітеміз (цикл ішінде өзгермесін)
                    match stop.kind {
                        TK::Int(_) => v,
                        _ => {
                            let t = self.vi();
                            self.emit(Inst::Mov(t, v));
                            t
                        }
                    }
                };
                let bb = self.new_block(false);
                let cont = self.new_block(false);
                let end = self.new_block(false);
                if let TK::Int(stepc) = step.kind {
                    let cc = if stepc > 0 { Cc::L } else { Cc::G };
                    self.set_term(Term::Br { cc, a: c, b: stopv, t: bb, f: end });
                    self.cur = bb;
                    self.assign_target(*var, c);
                    self.loops.push((cont, end));
                    for s in body {
                        self.stmt(s);
                    }
                    self.loops.pop();
                    self.set_term(Term::Jmp(cont));
                    self.cur = cont;
                    if Self::fits32(stepc) {
                        self.emit(Inst::IBinI(IOp::Add, c, c, stepc as i32));
                    } else {
                        let sv = self.iconst(stepc);
                        self.emit(Inst::IBin(IOp::Add, c, c, sv));
                    }
                    self.set_term(Term::Br { cc, a: c, b: stopv, t: bb, f: end });
                    self.cur = end;
                } else {
                    // қадам жұмыс уақытында белгілі
                    let stv = self.val(step);
                    let st = self.vi();
                    self.emit(Inst::Mov(st, stv));
                    let head = self.new_block(false);
                    let up = self.new_block(false);
                    let down = self.new_block(false);
                    let cold = self.new_block(true);
                    let ok = self.new_block(false);
                    self.set_term(Term::BrI { cc: Cc::E, a: st, imm: 0, t: cold, f: ok });
                    self.cur = cold;
                    self.emit(Inst::Line(self.cur_line.0, self.cur_line.1));
                    let msg = self.str_lit("`аралық`: қадам нөл болмауы керек");
                    self.callv("tlp_panic_str", &[msg]);
                    self.set_term(Term::Unreachable);
                    self.cur = ok;
                    self.set_term(Term::Jmp(head));
                    self.cur = head;
                    self.set_term(Term::BrI { cc: Cc::G, a: st, imm: 0, t: up, f: down });
                    self.cur = up;
                    self.set_term(Term::Br { cc: Cc::L, a: c, b: stopv, t: bb, f: end });
                    self.cur = down;
                    self.set_term(Term::Br { cc: Cc::G, a: c, b: stopv, t: bb, f: end });
                    self.cur = bb;
                    self.assign_target(*var, c);
                    self.loops.push((cont, end));
                    for s in body {
                        self.stmt(s);
                    }
                    self.loops.pop();
                    self.set_term(Term::Jmp(cont));
                    self.cur = cont;
                    self.emit(Inst::IBin(IOp::Add, c, c, st));
                    self.set_term(Term::Jmp(head));
                    self.cur = end;
                }
            }
            TStmt::ForList { item, index, list, body } => {
                let l = self.val(list);
                let lt = self.vi();
                self.emit(Inst::Mov(lt, l));
                let i = self.iconst(0);
                let head = self.new_block(false);
                let bb = self.new_block(false);
                let cont = self.new_block(false);
                let end = self.new_block(false);
                self.set_term(Term::Jmp(head));
                self.cur = head;
                let len = self.load(RC::I, lt, 0);
                self.set_term(Term::Br { cc: Cc::L, a: i, b: len, t: bb, f: end });
                self.cur = bb;
                let et = match &list.ty {
                    Ty::List(t) => (**t).clone(),
                    _ => unreachable!(),
                };
                let data = self.load(RC::I, lt, 16);
                let x = self.vr(rc_of(&et));
                let (scale, w) = list_elem(&et);
                self.emit(Inst::LoadIdx { d: x, base: data, idx: i, scale, off: 0, w });
                if let Some(ix) = index {
                    self.assign_target(*ix, i);
                }
                self.assign_target(*item, x);
                self.loops.push((cont, end));
                for s in body {
                    self.stmt(s);
                }
                self.loops.pop();
                self.set_term(Term::Jmp(cont));
                self.cur = cont;
                self.emit(Inst::IBinI(IOp::Add, i, i, 1));
                self.set_term(Term::Jmp(head));
                self.cur = end;
            }
            TStmt::ForStr { item, s, body } => {
                let sv = self.val(s);
                let st = self.vi();
                self.emit(Inst::Mov(st, sv));
                let pos = self.iconst(0);
                let head = self.new_block(false);
                let bb = self.new_block(false);
                let cont = self.new_block(false);
                let end = self.new_block(false);
                self.set_term(Term::Jmp(head));
                self.cur = head;
                let len = self.load(RC::I, st, 0);
                self.set_term(Term::Br { cc: Cc::L, a: pos, b: len, t: bb, f: end });
                self.cur = bb;
                let ch = self.calli("tlp_str_char", &[st, pos]);
                let clen = self.calli("tlp_str_char_len", &[st, pos]);
                let clen_t = self.vi();
                self.emit(Inst::Mov(clen_t, clen));
                self.assign_target(*item, ch);
                self.loops.push((cont, end));
                for s in body {
                    self.stmt(s);
                }
                self.loops.pop();
                self.set_term(Term::Jmp(cont));
                self.cur = cont;
                self.emit(Inst::IBin(IOp::Add, pos, pos, clen_t));
                self.set_term(Term::Jmp(head));
                self.cur = end;
            }
            TStmt::ForDict { key, val, dict, body } => {
                let dv = self.val(dict);
                let dt = self.vi();
                self.emit(Inst::Mov(dt, dv));
                let (kt, vt) = match &dict.ty {
                    Ty::Dict(k, v) => ((**k).clone(), (**v).clone()),
                    Ty::Set(k) => ((**k).clone(), Ty::Bool),
                    _ => unreachable!(),
                };
                let pos = self.iconst(0);
                let head = self.new_block(false);
                let bb = self.new_block(false);
                let cont = self.new_block(false);
                let end = self.new_block(false);
                self.set_term(Term::Jmp(head));
                self.cur = head;
                let np = self.calli("tlp_dict_next", &[dt, pos]);
                self.emit(Inst::Mov(pos, np));
                self.set_term(Term::BrI { cc: Cc::L, a: pos, imm: 0, t: end, f: bb });
                self.cur = bb;
                let k = self.calli("tlp_dict_key_at", &[dt, pos]);
                let _ = kt;
                self.assign_target(*key, k);
                if let Some(vtgt) = val {
                    let vb = self.calli("tlp_dict_val_at", &[dt, pos]);
                    let vv = self.from_bits(vb, &vt);
                    self.assign_target(*vtgt, vv);
                }
                self.loops.push((cont, end));
                for s in body {
                    self.stmt(s);
                }
                self.loops.pop();
                self.set_term(Term::Jmp(cont));
                self.cur = cont;
                self.emit(Inst::IBinI(IOp::Add, pos, pos, 1));
                self.set_term(Term::Jmp(head));
                self.cur = end;
            }
            TStmt::Line(f, l) => {
                self.cur_line = (*f, *l);
                self.emit(Inst::Line(*f, *l));
            }
            TStmt::Break => {
                let (_, b) = *self.loops.last().unwrap();
                self.terminate(Term::Jmp(b));
            }
            TStmt::Continue => {
                let (c, _) = *self.loops.last().unwrap();
                self.terminate(Term::Jmp(c));
            }
            TStmt::Return(e) => {
                let v = e.as_ref().map(|e| self.val(e));
                self.terminate(Term::Ret(v));
            }
        }
    }

    fn assign_target(&mut self, t: Target, v: VReg) {
        match t {
            Target::Local(l) => {
                let dst = self.locals[l];
                if dst != v {
                    self.emit(Inst::Mov(dst, v));
                }
            }
            Target::Global(g) => self.emit(Inst::StoreGlobal(8 * g as i32, v)),
        }
    }
}
