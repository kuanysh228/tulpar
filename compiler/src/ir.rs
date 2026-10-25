//! Төмен деңгейлі IR: виртуал регистрлер, негізгі блоктар, үш адресті нұсқаулар.

pub type VReg = u32;
pub type BlockId = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RC {
    I,
    F,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IOp {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Shl,
    Sar,
    /// қысқартып бөлу емес — еденге қарай (Python `//`)
    FloorDiv,
    FloorMod,
}

impl IOp {
    pub fn commutative(self) -> bool {
        matches!(self, IOp::Add | IOp::Mul | IOp::And | IOp::Or | IOp::Xor)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cc {
    E,
    Ne,
    L,
    Le,
    G,
    Ge,
    B,
    Be,
    A,
    Ae,
}

impl Cc {
    pub fn invert(self) -> Cc {
        match self {
            Cc::E => Cc::Ne,
            Cc::Ne => Cc::E,
            Cc::L => Cc::Ge,
            Cc::Le => Cc::G,
            Cc::G => Cc::Le,
            Cc::Ge => Cc::L,
            Cc::B => Cc::Ae,
            Cc::Be => Cc::A,
            Cc::A => Cc::Be,
            Cc::Ae => Cc::B,
        }
    }
    /// a cc b  <=>  b swap(cc) a
    pub fn swap(self) -> Cc {
        match self {
            Cc::E => Cc::E,
            Cc::Ne => Cc::Ne,
            Cc::L => Cc::G,
            Cc::Le => Cc::Ge,
            Cc::G => Cc::L,
            Cc::Ge => Cc::Le,
            Cc::B => Cc::A,
            Cc::Be => Cc::Ae,
            Cc::A => Cc::B,
            Cc::Ae => Cc::Be,
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            Cc::E => "e",
            Cc::Ne => "ne",
            Cc::L => "l",
            Cc::Le => "le",
            Cc::G => "g",
            Cc::Ge => "ge",
            Cc::B => "b",
            Cc::Be => "be",
            Cc::A => "a",
            Cc::Ae => "ae",
        }
    }
    pub fn eval(self, a: i64, b: i64) -> bool {
        match self {
            Cc::E => a == b,
            Cc::Ne => a != b,
            Cc::L => a < b,
            Cc::Le => a <= b,
            Cc::G => a > b,
            Cc::Ge => a >= b,
            Cc::B => (a as u64) < (b as u64),
            Cc::Be => (a as u64) <= (b as u64),
            Cc::A => (a as u64) > (b as u64),
            Cc::Ae => (a as u64) >= (b as u64),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FCc {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    B1,
    B8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ArgLoc {
    IReg(u8),
    FReg(u8),
    Stack(u16),
}

#[derive(Clone, Debug)]
pub enum Inst {
    IConst(VReg, i64),
    FConst(VReg, f64),
    Mov(VReg, VReg),
    IBin(IOp, VReg, VReg, VReg),
    IBinI(IOp, VReg, VReg, i32),
    FBin(FOp, VReg, VReg, VReg),
    INeg(VReg, VReg),
    INot(VReg, VReg),
    IAbs(VReg, VReg),
    IMin(VReg, VReg, VReg),
    IMax(VReg, VReg, VReg),
    FNeg(VReg, VReg),
    FAbs(VReg, VReg),
    FSqrt(VReg, VReg),
    /// 0 — еден (floor), 1 — төбе (ceil)
    FRound(VReg, VReg, u8),
    ICmp(Cc, VReg, VReg, VReg),
    ICmpI(Cc, VReg, VReg, i32),
    FCmp(FCc, VReg, VReg, VReg),
    I2F(VReg, VReg),
    F2I(VReg, VReg),
    /// float биттерін бүтін регистрге
    BitsFI(VReg, VReg),
    /// бүтін биттерді float регистрге
    BitsIF(VReg, VReg),
    Load { d: VReg, base: VReg, off: i32, w: Width },
    Store { base: VReg, off: i32, s: VReg, w: Width },
    /// d = [base + idx*scale + off]
    LoadIdx { d: VReg, base: VReg, idx: VReg, scale: u8, off: i32, w: Width },
    StoreIdx { base: VReg, idx: VReg, scale: u8, off: i32, s: VReg, w: Width },
    SymAddr(VReg, String),
    LoadGlobal(VReg, i32),
    StoreGlobal(i32, VReg),
    Param(VReg, ArgLoc),
    Call { d: Option<VReg>, sym: String, args: Vec<VReg> },
    /// d = (a cc b) ? t : f  (cmov)
    Select { cc: Cc, d: VReg, a: VReg, b: VReg, t: VReg, f: VReg },
    SelectI { cc: Cc, d: VReg, a: VReg, imm: i32, t: VReg, f: VReg },
    /// Қате трассасы үшін орын белгісі (нұсқау жасалмайды)
    Line(u16, u32),
    /// Жанама шақыру (виртуалды әдістер)
    CallInd { d: Option<VReg>, target: VReg, args: Vec<VReg> },
}

#[derive(Clone, Debug)]
pub enum Term {
    Jmp(BlockId),
    Br { cc: Cc, a: VReg, b: VReg, t: BlockId, f: BlockId },
    BrI { cc: Cc, a: VReg, imm: i32, t: BlockId, f: BlockId },
    BrF { cc: FCc, a: VReg, b: VReg, t: BlockId, f: BlockId },
    /// (a & imm) cc 0, cc ∈ {E, Ne}
    BrTest { cc: Cc, a: VReg, imm: i32, t: BlockId, f: BlockId },
    Ret(Option<VReg>),
    Unreachable,
}

impl Term {
    pub fn succs(&self) -> Vec<BlockId> {
        match self {
            Term::Jmp(b) => vec![*b],
            Term::Br { t, f, .. } | Term::BrI { t, f, .. } | Term::BrF { t, f, .. } | Term::BrTest { t, f, .. } => vec![*t, *f],
            _ => vec![],
        }
    }
    pub fn uses(&self, mut f: impl FnMut(VReg)) {
        match self {
            Term::Br { a, b, .. } | Term::BrF { a, b, .. } => {
                f(*a);
                f(*b);
            }
            Term::BrI { a, .. } | Term::BrTest { a, .. } => f(*a),
            Term::Ret(Some(v)) => f(*v),
            _ => {}
        }
    }
    pub fn map_uses(&mut self, mut f: impl FnMut(VReg) -> VReg) {
        match self {
            Term::Br { a, b, .. } | Term::BrF { a, b, .. } => {
                *a = f(*a);
                *b = f(*b);
            }
            Term::BrI { a, .. } | Term::BrTest { a, .. } => *a = f(*a),
            Term::Ret(Some(v)) => *v = f(*v),
            _ => {}
        }
    }
    pub fn map_blocks(&mut self, mut f: impl FnMut(BlockId) -> BlockId) {
        match self {
            Term::Jmp(b) => *b = f(*b),
            Term::Br { t, f: ff, .. } | Term::BrI { t, f: ff, .. } | Term::BrF { t, f: ff, .. } | Term::BrTest { t, f: ff, .. } => {
                *t = f(*t);
                *ff = f(*ff);
            }
            _ => {}
        }
    }
}

impl Inst {
    pub fn def(&self) -> Option<VReg> {
        use Inst::*;
        match self {
            IConst(d, _) | FConst(d, _) | Mov(d, _) | IBin(_, d, _, _) | IBinI(_, d, _, _) | FBin(_, d, _, _) | INeg(d, _) | INot(d, _)
            | IAbs(d, _) | IMin(d, _, _) | IMax(d, _, _) | FNeg(d, _) | FAbs(d, _) | FSqrt(d, _) | FRound(d, _, _) | ICmp(_, d, _, _)
            | ICmpI(_, d, _, _) | FCmp(_, d, _, _) | I2F(d, _) | F2I(d, _) | BitsFI(d, _) | BitsIF(d, _) | SymAddr(d, _) | LoadGlobal(d, _)
            | Param(d, _) => Some(*d),
            Select { d, .. } | SelectI { d, .. } => Some(*d),
            Load { d, .. } | LoadIdx { d, .. } => Some(*d),
            Call { d, .. } | CallInd { d, .. } => *d,
            Store { .. } | StoreIdx { .. } | StoreGlobal(..) | Line(..) => None,
        }
    }

    pub fn uses(&self, mut f: impl FnMut(VReg)) {
        use Inst::*;
        match self {
            IConst(..) | FConst(..) | SymAddr(..) | LoadGlobal(..) | Param(..) | Line(..) => {}
            Mov(_, s) | INeg(_, s) | INot(_, s) | IAbs(_, s) | FNeg(_, s) | FAbs(_, s) | FSqrt(_, s) | FRound(_, s, _) | I2F(_, s)
            | F2I(_, s) | BitsFI(_, s) | BitsIF(_, s) | IBinI(_, _, s, _) | ICmpI(_, _, s, _) => f(*s),
            IBin(_, _, a, b) | FBin(_, _, a, b) | IMin(_, a, b) | IMax(_, a, b) | ICmp(_, _, a, b) | FCmp(_, _, a, b) => {
                f(*a);
                f(*b);
            }
            Select { a, b, t, f: ff, .. } => {
                f(*a);
                f(*b);
                f(*t);
                f(*ff);
            }
            SelectI { a, t, f: ff, .. } => {
                f(*a);
                f(*t);
                f(*ff);
            }
            Load { base, .. } => f(*base),
            Store { base, s, .. } => {
                f(*base);
                f(*s);
            }
            LoadIdx { base, idx, .. } => {
                f(*base);
                f(*idx);
            }
            StoreIdx { base, idx, s, .. } => {
                f(*base);
                f(*idx);
                f(*s);
            }
            StoreGlobal(_, s) => f(*s),
            Call { args, .. } => args.iter().for_each(|a| f(*a)),
            CallInd { target, args, .. } => {
                f(*target);
                args.iter().for_each(|a| f(*a));
            }
        }
    }

    pub fn map_uses(&mut self, mut f: impl FnMut(VReg) -> VReg) {
        use Inst::*;
        match self {
            IConst(..) | FConst(..) | SymAddr(..) | LoadGlobal(..) | Param(..) | Line(..) => {}
            Mov(_, s) | INeg(_, s) | INot(_, s) | IAbs(_, s) | FNeg(_, s) | FAbs(_, s) | FSqrt(_, s) | FRound(_, s, _) | I2F(_, s)
            | F2I(_, s) | BitsFI(_, s) | BitsIF(_, s) | IBinI(_, _, s, _) | ICmpI(_, _, s, _) => *s = f(*s),
            IBin(_, _, a, b) | FBin(_, _, a, b) | IMin(_, a, b) | IMax(_, a, b) | ICmp(_, _, a, b) | FCmp(_, _, a, b) => {
                *a = f(*a);
                *b = f(*b);
            }
            Select { a, b, t, f: ff, .. } => {
                *a = f(*a);
                *b = f(*b);
                *t = f(*t);
                *ff = f(*ff);
            }
            SelectI { a, t, f: ff, .. } => {
                *a = f(*a);
                *t = f(*t);
                *ff = f(*ff);
            }
            Load { base, .. } => *base = f(*base),
            Store { base, s, .. } => {
                *base = f(*base);
                *s = f(*s);
            }
            LoadIdx { base, idx, .. } => {
                *base = f(*base);
                *idx = f(*idx);
            }
            StoreIdx { base, idx, s, .. } => {
                *base = f(*base);
                *idx = f(*idx);
                *s = f(*s);
            }
            StoreGlobal(_, s) => *s = f(*s),
            Call { args, .. } => args.iter_mut().for_each(|a| *a = f(*a)),
            CallInd { target, args, .. } => {
                *target = f(*target);
                args.iter_mut().for_each(|a| *a = f(*a));
            }
        }
    }

    pub fn set_def(&mut self, nd: VReg) {
        use Inst::*;
        match self {
            IConst(d, _) | FConst(d, _) | Mov(d, _) | IBin(_, d, _, _) | IBinI(_, d, _, _) | FBin(_, d, _, _) | INeg(d, _) | INot(d, _)
            | IAbs(d, _) | IMin(d, _, _) | IMax(d, _, _) | FNeg(d, _) | FAbs(d, _) | FSqrt(d, _) | FRound(d, _, _) | ICmp(_, d, _, _)
            | ICmpI(_, d, _, _) | FCmp(_, d, _, _) | I2F(d, _) | F2I(d, _) | BitsFI(d, _) | BitsIF(d, _) | SymAddr(d, _) | LoadGlobal(d, _)
            | Param(d, _) => *d = nd,
            Select { d, .. } | SelectI { d, .. } => *d = nd,
            Load { d, .. } | LoadIdx { d, .. } => *d = nd,
            Call { d, .. } | CallInd { d, .. } => *d = Some(nd),
            _ => {}
        }
    }

    /// Жанама әсері бар (жойып жіберуге болмайтын) нұсқау ма.
    pub fn has_effect(&self) -> bool {
        // бөлу нұсқаулары нөлге бөлуде тоқтатады — оларды жоюға болмайды
        matches!(self, Inst::Store { .. } | Inst::StoreIdx { .. } | Inst::StoreGlobal(..) | Inst::Call { .. } | Inst::CallInd { .. } | Inst::Line(..))
            || matches!(self, Inst::IBin(IOp::FloorDiv | IOp::FloorMod, ..))
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub insts: Vec<Inst>,
    pub term: Term,
    pub cold: bool,
}

#[derive(Clone, Debug)]
pub struct IrFunc {
    pub name: String,
    pub sym: String,
    pub blocks: Vec<Block>,
    pub classes: Vec<RC>,
}

impl IrFunc {
    pub fn new_vreg(&mut self, rc: RC) -> VReg {
        self.classes.push(rc);
        (self.classes.len() - 1) as VReg
    }

    /// Блоктардың шығару реті: "iz" (trace) орналастыру, суық блоктар соңында.
    pub fn layout(&self) -> Vec<BlockId> {
        let n = self.blocks.len();
        let mut placed = vec![false; n];
        let mut order: Vec<BlockId> = Vec::with_capacity(n);
        let mut cold: Vec<BlockId> = Vec::new();
        let mut pending: Vec<BlockId> = vec![0];
        while let Some(start) = pending.pop() {
            let mut b = start;
            loop {
                if placed[b as usize] {
                    break;
                }
                placed[b as usize] = true;
                if self.blocks[b as usize].cold {
                    cold.push(b);
                    break;
                }
                order.push(b);
                let succs = self.blocks[b as usize].term.succs();
                let mut next = None;
                for &s in &succs {
                    if !placed[s as usize] && !self.blocks[s as usize].cold && next.is_none() {
                        next = Some(s);
                    }
                }
                for &s in succs.iter().rev() {
                    if Some(s) != next && !placed[s as usize] {
                        pending.push(s);
                    }
                }
                match next {
                    Some(nb) => b = nb,
                    None => break,
                }
            }
        }
        for b in 0..n as u32 {
            if !placed[b as usize] {
                if self.blocks[b as usize].cold {
                    cold.push(b);
                } else {
                    order.push(b);
                }
            }
        }
        order.extend(cold);
        order
    }

    /// Қол жеткізілмейтін блоктарды жояды (индекстерді қайта нөмірлейді).
    pub fn remove_unreachable(&mut self) {
        let n = self.blocks.len();
        let mut seen = vec![false; n];
        let mut stack = vec![0u32];
        while let Some(b) = stack.pop() {
            if seen[b as usize] {
                continue;
            }
            seen[b as usize] = true;
            for s in self.blocks[b as usize].term.succs() {
                stack.push(s);
            }
        }
        let mut map = vec![u32::MAX; n];
        let mut k = 0;
        for i in 0..n {
            if seen[i] {
                map[i] = k;
                k += 1;
            }
        }
        let old = std::mem::take(&mut self.blocks);
        for (i, mut b) in old.into_iter().enumerate() {
            if seen[i] {
                b.term.map_blocks(|x| map[x as usize]);
                self.blocks.push(b);
            }
        }
    }
}
