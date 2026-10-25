//! Абстрактты синтаксис ағашы (AST).

use crate::diag::Pos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
    BitNot,
    Deref,
}

#[derive(Clone, Debug)]
pub enum FStrPart {
    Lit(String),
    Expr { expr: Expr, spec: Option<String> },
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    FStr(Vec<FStrPart>),
    Bool(bool),
    None,
    Name(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Cmp(CmpOp, Box<Expr>, Box<Expr>),
    /// `a ішінде b` / `a емес ішінде b`
    Contains { item: Box<Expr>, container: Box<Expr>, negated: bool },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Arg>),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    Field(Box<Expr>, String),
    List(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    /// `{a, b, c}` — жиын
    Set(Vec<Expr>),
    /// Кортеж `(a, b)`; индекс позициясында — типтер тізімі: `сөздік[мәтін, бүтін]`
    Tuple(Vec<Expr>),
    /// `[өрнек әр x ішінде тізбек егер шарт]`
    ListComp { elem: Box<Expr>, targets: Vec<(String, Pos)>, iter: Box<Expr>, cond: Option<Box<Expr>> },
    /// `лямбда x, y: өрнек`
    Lambda { params: Vec<(String, Pos)>, body: Box<Expr> },
    /// `a егер шарт әйтпесе b`
    Ternary { cond: Box<Expr>, then: Box<Expr>, els: Box<Expr> },
}

#[derive(Clone, Debug)]
pub enum TypeExpr {
    Name(String, Pos),
    Generic(String, Vec<TypeExpr>, Pos),
    Ptr(Box<TypeExpr>, Pos),
    Tuple(Vec<TypeExpr>, Pos),
    /// `(T1, T2) -> R`
    Func(Vec<TypeExpr>, Box<TypeExpr>, Pos),
}

impl TypeExpr {
    pub fn pos(&self) -> Pos {
        match self {
            TypeExpr::Name(_, p) | TypeExpr::Generic(_, _, p) | TypeExpr::Ptr(_, p) | TypeExpr::Tuple(_, p) | TypeExpr::Func(_, _, p) => *p,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub default: Option<Expr>,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Vec<Stmt>,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct ClassDef {
    pub name: String,
    pub base: Option<(String, Pos)>,
    pub fields: Vec<FieldDef>,
    pub methods: Vec<FuncDef>,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Expr(Expr),
    Assign { target: Expr, value: Expr },
    AnnAssign { target: Expr, ty: TypeExpr, value: Option<Expr> },
    TupleAssign { targets: Vec<Expr>, values: Vec<Expr> },
    AugAssign { target: Expr, op: BinOp, value: Expr },
    If { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    For { targets: Vec<(String, Pos)>, iter: Expr, body: Vec<Stmt> },
    Break,
    Continue,
    Pass,
    Return(Option<Expr>),
    Global(Vec<String>),
    Manual(Vec<Stmt>),
    Assert(Expr, Option<Expr>),
    FuncDef(FuncDef),
    ClassDef(ClassDef),
    /// `a(); b()` — бір жолдағы бірнеше оператор
    Seq(Vec<Stmt>),
    /// `импорт "файл.tlp"`
    Import(String),
}

pub struct Program {
    pub stmts: Vec<Stmt>,
}
