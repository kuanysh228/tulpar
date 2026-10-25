//! Типтелген аралық ағаш (HIR): тексергіштің нәтижесі, IR генераторының кірісі.

use crate::ast::{BinOp, CmpOp, UnOp};

pub type LocalId = usize;
pub type GlobalId = usize;
pub type FuncId = usize;
pub type ClassId = usize;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Byte,
    Str,
    /// мән қайтармайтын (`жоқ`)
    None,
    List(Box<Ty>),
    Dict(Box<Ty>, Box<Ty>),
    Set(Box<Ty>),
    Class(ClassId),
    Ptr(Box<Ty>),
    Tuple(Vec<Ty>),
    /// функция мәні: (параметрлер) -> қайтару
    Func(Vec<Ty>, Box<Ty>),
    /// `жоқ` литералының типі (көрсеткіштерге ғана сәйкес келеді)
    Null,
}

impl Ty {
    /// Мәні бүтін регистрде тұра ма (иә), әлде float регистрде ме.
    pub fn is_float(&self) -> bool {
        matches!(self, Ty::Float)
    }
    /// GC үйіндісіндегі объектке сілтеме ме.
    pub fn is_gc_ref(&self) -> bool {
        matches!(self, Ty::Str | Ty::List(_) | Ty::Dict(..) | Ty::Set(_) | Ty::Class(_) | Ty::Tuple(_) | Ty::Null)
    }
    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float | Ty::Byte)
    }
    pub fn is_nullable(&self) -> bool {
        matches!(self, Ty::Str | Ty::List(_) | Ty::Dict(..) | Ty::Set(_) | Ty::Class(_) | Ty::Ptr(_) | Ty::Null)
    }
    pub fn is_unit(&self) -> bool {
        matches!(self, Ty::None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastKind {
    IntToFloat,
    FloatToInt,
    ByteToInt,
    IntToByte,
    BoolToInt,
    /// көрсеткіш <-> бүтін (биттер өзгермейді)
    Reinterpret,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Bi {
    Print { nl: bool },
    Input,
    ToStr,
    StrToInt,
    StrToFloat,
    FloatFixed(u32),
    /// (мәтін, ені); 0 — солға, 1 — оңға, 2 — ортаға
    StrPad(u8),
    AbsI,
    AbsF,
    MinI,
    MaxI,
    MinF,
    MaxF,
    Sqrt,
    Floor,
    Ceil,
    PowI,
    PowF,
    FMod,
    Len,
    StrUpper,
    StrLower,
    StrSplit,
    StrJoin,
    StrFind,
    StrReplace,
    StrStrip,
    StrByte,
    StrStarts,
    StrEnds,
    ListPush,
    ListPop,
    ListClear,
    ListSort,
    ListReverse,
    DictDel,
    DictGetOr,
    DictKeys,
    DictValues,
    Alloc(Ty),
    Free,
    Gc,
    GcHeap,
    Panic,
    Args,
    /// (жол) -> мәтін
    FileRead,
    /// (жол, мәтін), қосу режимі
    FileWrite(bool),
    Time,
    Rand,
    RandRange,
    Seed,
    Exit,
    /// 0 sin, 1 cos, 2 tan, 3 atan, 4 exp, 5 ln, 6 log10, 7 atan2
    Math(u8),
    ListIndex,
    ListRemove,
    ListInsert,
    ListCopy,
    ListSum,
    ListMin,
    ListMax,
    IsInstance(ClassId),
    Downcast(ClassId),
    Ord,
    Chr,
    StrIsDigit,
    StrIsAlpha,
    StrSplitWs,
    StrCount,
    SetAdd,
    /// 0 бірігу (|), 1 қиылысу (&), 2 айырма (-)
    SetOp(u8),
    SetFromList,
    /// (тізім, кілттер тізімі): түрі 0 бүтін, 1 нақты, 2 мәтін
    ListSortBy(u8),
    NetListen,
    NetListenLocal,
    StrBytes,
    NetAccept,
    NetConnect,
    NetRead,
    NetWrite,
    NetClose,
    /// файл_бар(жол) -> логика
    FileExists,
    /// файл_жой(жол)
    FileDelete,
    /// файл_ауыстыр(ескі, жаңа)
    FileRename,
    /// қалта_жаса(жол)
    MakeDir,
    /// желі_күту(fd, миллисекунд)
    NetTimeout,
}

#[derive(Clone, Debug)]
pub enum FPartT {
    Lit(String),
    Val(TExpr),
}

#[derive(Clone, Debug)]
pub struct TExpr {
    pub kind: TK,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub enum TK {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Null,
    Local(LocalId),
    Global(GlobalId),
    Unary(UnOp, Box<TExpr>),
    /// екі операнд та бірдей типті (Int/Float), не Str `+`, Str/List `*` бүтінге
    Binary(BinOp, Box<TExpr>, Box<TExpr>),
    Cmp(CmpOp, Box<TExpr>, Box<TExpr>),
    And(Box<TExpr>, Box<TExpr>),
    Or(Box<TExpr>, Box<TExpr>),
    Cast(CastKind, Box<TExpr>),
    Call(FuncId, Vec<TExpr>),
    Builtin(Bi, Vec<TExpr>),
    Index(Box<TExpr>, Box<TExpr>),
    Slice(Box<TExpr>, Option<Box<TExpr>>, Option<Box<TExpr>>),
    Field(Box<TExpr>, usize),
    ListLit(Vec<TExpr>),
    DictLit(Vec<(TExpr, TExpr)>),
    SetLit(Vec<TExpr>),
    TupleLit(Vec<TExpr>),
    TupleGet(Box<TExpr>, usize),
    /// Алдымен операторлар орындалады, содан кейін өрнек (генераторлар үшін)
    Block(Vec<TStmt>, Box<TExpr>),
    /// Құрылымдық теңдік (тізім, сөздік, кортеж)
    DeepEq(Box<TExpr>, Box<TExpr>, bool),
    /// Виртуалды шақыру: әдіс аты (vtable кілті), аргументтер (бірінші — объект)
    VCall(String, Vec<TExpr>),
    /// Функцияға сілтеме (атау немесе лямбда)
    FuncRef(FuncId),
    /// Мән ретіндегі функцияны шақыру
    CallValue(Box<TExpr>, Vec<TExpr>),
    /// класс экземпляры: өрістердің бастапқы мәндері (defaults толтырылған) және құру үшін аргументтер
    New { class: ClassId, fields: Vec<Option<TExpr>>, ctor: Option<(FuncId, Vec<TExpr>)> },
    Deref(Box<TExpr>),
    Ternary(Box<TExpr>, Box<TExpr>, Box<TExpr>),
    FStr(Vec<FPartT>),
    Contains { item: Box<TExpr>, container: Box<TExpr>, negated: bool },
}

#[derive(Clone, Debug)]
pub enum TStmt {
    Expr(TExpr),
    SetLocal(LocalId, TExpr),
    SetGlobal(GlobalId, TExpr),
    SetIndex(TExpr, TExpr, TExpr),
    SetField(TExpr, usize, TExpr),
    SetDeref(TExpr, TExpr),
    If { cond: TExpr, then: Vec<TStmt>, els: Vec<TStmt> },
    While { cond: TExpr, body: Vec<TStmt> },
    /// `әр var ішінде аралық(start, stop, step)`; step — тұрақты
    ForRange { var: Target, start: TExpr, stop: TExpr, step: TExpr, body: Vec<TStmt> },
    /// `idx` — жасырын есептегіш; `index` — қолданушының индекс айнымалысы (enumerate)
    ForList { item: Target, index: Option<Target>, list: TExpr, body: Vec<TStmt> },
    ForStr { item: Target, s: TExpr, body: Vec<TStmt> },
    ForDict { key: Target, val: Option<Target>, dict: TExpr, body: Vec<TStmt> },
    Break,
    Continue,
    Return(Option<TExpr>),
    /// оператордың бастапқы коддағы орны (қате трассасы үшін): (файл, жол)
    Line(u16, u32),
}

#[derive(Clone, Copy, Debug)]
pub enum Target {
    Local(LocalId),
    Global(GlobalId),
}

#[derive(Clone, Debug)]
pub struct Local {
    #[allow(dead_code)]
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub struct TFunc {
    pub name: String,
    pub nparams: usize,
    pub locals: Vec<Local>,
    pub ret: Ty,
    pub body: Vec<TStmt>,
    /// денесі жоқ: хост (платформа) іске асырады
    pub external: bool,
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub struct ClassInfo {
    pub name: String,
    /// мұраланған өрістер алдымен
    pub fields: Vec<FieldInfo>,
    pub base: Option<ClassId>,
    /// иерархияға қатысатын кластарда объектінің 0-ші сөзі — класс идентификаторы
    pub has_header: bool,
}

#[derive(Clone, Debug)]
pub struct GlobalInfo {
    #[allow(dead_code)]
    pub name: String,
    pub ty: Ty,
}

pub struct Module {
    pub funcs: Vec<TFunc>,
    pub classes: Vec<ClassInfo>,
    pub globals: Vec<GlobalInfo>,
    pub main: FuncId,
    /// виртуалды әдістер: аты және әр класс идентификаторы үшін функция
    pub vtables: Vec<(String, Vec<Option<FuncId>>)>,
}

pub fn ty_name(classes: &[ClassInfo], t: &Ty) -> String {
    match t {
        Ty::Int => "бүтін".into(),
        Ty::Float => "нақты".into(),
        Ty::Bool => "логика".into(),
        Ty::Byte => "байт".into(),
        Ty::Str => "мәтін".into(),
        Ty::None => "жоқ".into(),
        Ty::List(e) => format!("тізім[{}]", ty_name(classes, e)),
        Ty::Dict(k, v) => format!("сөздік[{}, {}]", ty_name(classes, k), ty_name(classes, v)),
        Ty::Set(e) => format!("жиын[{}]", ty_name(classes, e)),
        Ty::Class(c) => classes[*c].name.clone(),
        Ty::Ptr(e) => format!("*{}", ty_name(classes, e)),
        Ty::Tuple(v) => format!("({})", v.iter().map(|t| ty_name(classes, t)).collect::<Vec<_>>().join(", ")),
        Ty::Func(ps, r) => format!("({}) -> {}", ps.iter().map(|t| ty_name(classes, t)).collect::<Vec<_>>().join(", "), ty_name(classes, r)),
        Ty::Null => "жоқ".into(),
    }
}
