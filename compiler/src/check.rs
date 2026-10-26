//! Тип тексергіші: AST -> HIR. Статикалық типтер, жергілікті тип шығару.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::*;
use crate::diag::{Diag, Pos, Res};
use crate::hir::*;

const BUILTIN_NAMES: &[&str] = &[
    "жаз", "көрсет", "оқы", "ұзындық", "бүтін", "нақты", "мәтін", "логика", "байт", "абс", "мин", "макс", "түбір", "еден", "төбе", "бөл",
    "босат", "жина", "жад", "қате", "аралық", "тізім", "сөздік", "аргтар", "файл_оқы", "файл_жаз", "файл_қос", "уақыт", "кездейсоқ",
    "кездейсоқ_аралық", "тұқым", "шығу", "синус", "косинус", "тангенс", "арктангенс", "арктангенс2", "экспонента", "логарифм",
    "логарифм10", "қосынды", "нөмірле", "мысал", "түрлендір", "жиын", "код", "таңба", "тыңда", "тыңда_локал", "қабылда", "қосыл", "желі_оқы", "желі_жаз", "желі_жап",
    "желі_күту", "файл_бар", "файл_жой", "файл_ауыстыр", "қалта_жаса", "мекен", "мекенжай",
    "порт_оқы8", "порт_оқы16", "порт_оқы32", "порт_жаз8", "порт_жаз16", "порт_жаз32", "оқы8", "оқы16", "оқы32", "оқы64", "жаз8", "жаз16",
    "жаз32", "жаз64", "тоқтат", "үзіліс_қос", "үзіліс_өшір", "cr_оқы", "cr_жаз", "msr_оқы", "msr_жаз", "тактілер", "cpuid", "invlpg",
    "жад_көшір", "жад_толтыр", "үзіліс_орнату", "жүктеу_ақпараты", "пернетақта_оқы",
];

/// Аппараттық ішкі функциялар (ядро режимі): (аты, рантайм символы, аргумент түрлері, мән қайтарады ма).
/// `i` — бүтін, `a` — мекенжай (бүтін немесе көрсеткіш), `h` — үзіліс өңдеушісі.
const HW_BUILTINS: &[(&str, &str, &str, bool)] = &[
    ("порт_оқы8", "tlp_hw_in8", "i", true),
    ("порт_оқы16", "tlp_hw_in16", "i", true),
    ("порт_оқы32", "tlp_hw_in32", "i", true),
    ("порт_жаз8", "tlp_hw_out8", "ii", false),
    ("порт_жаз16", "tlp_hw_out16", "ii", false),
    ("порт_жаз32", "tlp_hw_out32", "ii", false),
    ("оқы8", "tlp_hw_rd8", "a", true),
    ("оқы16", "tlp_hw_rd16", "a", true),
    ("оқы32", "tlp_hw_rd32", "a", true),
    ("оқы64", "tlp_hw_rd64", "a", true),
    ("жаз8", "tlp_hw_wr8", "ai", false),
    ("жаз16", "tlp_hw_wr16", "ai", false),
    ("жаз32", "tlp_hw_wr32", "ai", false),
    ("жаз64", "tlp_hw_wr64", "ai", false),
    ("тоқтат", "tlp_hw_hlt", "", false),
    ("үзіліс_қос", "tlp_hw_sti", "", false),
    ("үзіліс_өшір", "tlp_hw_cli", "", false),
    ("cr_оқы", "tlp_hw_cr_read", "i", true),
    ("cr_жаз", "tlp_hw_cr_write", "ii", false),
    ("msr_оқы", "tlp_hw_msr_read", "i", true),
    ("msr_жаз", "tlp_hw_msr_write", "ii", false),
    ("тактілер", "tlp_hw_rdtsc", "", true),
    ("cpuid", "tlp_hw_cpuid", "ii", true),
    ("invlpg", "tlp_hw_invlpg", "a", false),
    ("жад_көшір", "memcpy", "aai", false),
    ("жад_толтыр", "memset", "aii", false),
    ("үзіліс_орнату", "tlp_hw_set_handler", "ih", false),
    ("жүктеу_ақпараты", "tlp_hw_mbi", "", true),
    ("пернетақта_оқы", "tlp_hw_key", "", true),
];

static KERNEL_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ядро режимін қосады (`tulpar os`): аппараттық ішкі функциялар рұқсат етіледі.
pub fn set_kernel_mode(on: bool) {
    KERNEL_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[derive(PartialEq, Clone, Copy)]
enum St {
    Todo,
    Busy,
    Done,
}

struct FuncSlot {
    name: String,
    params: Vec<(String, Ty)>,
    defaults: Vec<Option<TExpr>>,
    ret: Option<Ty>,
    ast: Rc<FuncDef>,
    state: St,
    done: Option<TFunc>,
    /// шығарылған (немесе жарияланған) қайтару типі
    final_ret: Option<Ty>,
}

#[derive(Default)]
struct Ctx {
    locals: Vec<Local>,
    names: HashMap<String, LocalId>,
    declared_global: HashSet<String>,
    ret: Option<Ty>,
    ret_declared: bool,
    is_main: bool,
    manual: u32,
    loops: u32,
    tmp: u32,
    comp: u32,
    /// келесі шақыру оператор позициясында (нәтиже қолданылмайды)
    stmt_call: bool,
}

enum ForKind {
    Range { var: Target, start: TExpr, stop: TExpr, step: TExpr },
    List { item: Target, index: Option<Target>, list: TExpr },
    Str { item: Target, s: TExpr },
    Dict { key: Target, val: Option<Target>, dict: TExpr },
}

struct ForParts {
    kind: ForKind,
    /// цикл денесінің алдында орындалатын операторлар (кортежді ашу)
    prelude: Vec<TStmt>,
}

impl ForParts {
    fn into_stmt(self, body: Vec<TStmt>) -> TStmt {
        let mut b = self.prelude;
        b.extend(body);
        match self.kind {
            ForKind::Range { var, start, stop, step } => TStmt::ForRange { var, start, stop, step, body: b },
            ForKind::List { item, index, list } => TStmt::ForList { item, index, list, body: b },
            ForKind::Str { item, s } => TStmt::ForStr { item, s, body: b },
            ForKind::Dict { key, val, dict } => TStmt::ForDict { key, val, dict, body: b },
        }
    }
}

pub struct Checker {
    classes: Vec<ClassInfo>,
    class_names: HashMap<String, ClassId>,
    class_methods: Vec<HashMap<String, FuncId>>,
    class_ctor: Vec<Option<FuncId>>,
    class_defaults: Vec<Vec<Option<TExpr>>>,
    class_children: Vec<Vec<ClassId>>,
    used_virtual: Vec<String>,
    overrides: Vec<(FuncId, FuncId)>,
    funcs: Vec<FuncSlot>,
    func_names: HashMap<String, FuncId>,
    globals: Vec<GlobalInfo>,
    global_ids: HashMap<String, GlobalId>,
    global_names: HashSet<String>,
    cx: Ctx,
}

fn err<T>(pos: Pos, msg: impl Into<String>) -> Res<T> {
    Err(Diag::new(pos, msg))
}

// ---------------------------------------------------------------- атауларды сканерлеу

#[derive(Default)]
struct Scan {
    reads: HashSet<String>,
    assigned: HashSet<String>,
    globals: HashSet<String>,
}

fn scan_expr(e: &Expr, s: &mut Scan) {
    match &e.kind {
        ExprKind::Name(n) => {
            s.reads.insert(n.clone());
        }
        ExprKind::FStr(parts) => {
            for p in parts {
                if let FStrPart::Expr { expr, .. } = p {
                    scan_expr(expr, s);
                }
            }
        }
        ExprKind::Unary(_, a) | ExprKind::Field(a, _) => scan_expr(a, s),
        ExprKind::Binary(_, a, b) | ExprKind::Cmp(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) | ExprKind::Index(a, b) => {
            scan_expr(a, s);
            scan_expr(b, s);
        }
        ExprKind::Contains { item, container, .. } => {
            scan_expr(item, s);
            scan_expr(container, s);
        }
        ExprKind::Call(f, args) => {
            scan_expr(f, s);
            for a in args {
                scan_expr(&a.value, s);
            }
        }
        ExprKind::Slice(a, lo, hi) => {
            scan_expr(a, s);
            if let Some(x) = lo {
                scan_expr(x, s);
            }
            if let Some(x) = hi {
                scan_expr(x, s);
            }
        }
        ExprKind::List(v) | ExprKind::Tuple(v) => v.iter().for_each(|x| scan_expr(x, s)),
        ExprKind::Dict(v) => v.iter().for_each(|(k, x)| {
            scan_expr(k, s);
            scan_expr(x, s);
        }),
        ExprKind::Ternary { cond, then, els } => {
            scan_expr(cond, s);
            scan_expr(then, s);
            scan_expr(els, s);
        }
        _ => {}
    }
}

fn scan_target(e: &Expr, s: &mut Scan) {
    match &e.kind {
        ExprKind::Name(n) => {
            s.assigned.insert(n.clone());
        }
        _ => scan_expr(e, s),
    }
}

fn scan_stmts(v: &[Stmt], s: &mut Scan) {
    for st in v {
        match &st.kind {
            StmtKind::Expr(e) => scan_expr(e, s),
            StmtKind::Assign { target, value } => {
                scan_target(target, s);
                scan_expr(value, s);
            }
            StmtKind::AnnAssign { target, value, .. } => {
                scan_target(target, s);
                if let Some(v) = value {
                    scan_expr(v, s);
                }
            }
            StmtKind::TupleAssign { targets, values } => {
                targets.iter().for_each(|t| scan_target(t, s));
                values.iter().for_each(|t| scan_expr(t, s));
            }
            StmtKind::AugAssign { target, value, .. } => {
                // оқылады да, жазылады да; ат жергілікті болса ғана жергілікті
                scan_expr(target, s);
                scan_target(target, s);
                scan_expr(value, s);
            }
            StmtKind::If { cond, then, els } => {
                scan_expr(cond, s);
                scan_stmts(then, s);
                scan_stmts(els, s);
            }
            StmtKind::While { cond, body } => {
                scan_expr(cond, s);
                scan_stmts(body, s);
            }
            StmtKind::For { targets, iter, body } => {
                for (t, _) in targets {
                    s.assigned.insert(t.clone());
                }
                scan_expr(iter, s);
                scan_stmts(body, s);
            }
            StmtKind::Return(Some(e)) => scan_expr(e, s),
            StmtKind::Global(names) => names.iter().for_each(|n| {
                s.globals.insert(n.clone());
            }),
            StmtKind::Manual(b) | StmtKind::Seq(b) => scan_stmts(b, s),
            StmtKind::Assert(c, m) => {
                scan_expr(c, s);
                if let Some(m) = m {
                    scan_expr(m, s);
                }
            }
            _ => {}
        }
    }
}

fn scan_func(f: &FuncDef, out: &mut HashSet<String>) {
    let mut s = Scan::default();
    for p in &f.params {
        s.assigned.insert(p.name.clone());
        if let Some(d) = &p.default {
            scan_expr(d, &mut s);
        }
    }
    scan_stmts(&f.body, &mut s);
    for r in &s.reads {
        if !s.assigned.contains(r) || s.globals.contains(r) {
            out.insert(r.clone());
        }
    }
    for g in &s.globals {
        out.insert(g.clone());
    }
}

// ---------------------------------------------------------------- көмекші

fn terminates(v: &[TStmt]) -> bool {
    match v.last() {
        Some(TStmt::Return(_)) => true,
        Some(TStmt::Expr(TExpr { kind: TK::Builtin(Bi::Panic, _), .. })) => true,
        Some(TStmt::If { then, els, .. }) => terminates(then) && terminates(els),
        Some(TStmt::While { cond: TExpr { kind: TK::Bool(true), .. }, body }) => !has_break(body),
        _ => false,
    }
}

fn has_break(v: &[TStmt]) -> bool {
    v.iter().any(|s| match s {
        TStmt::Break => true,
        TStmt::If { then, els, .. } => has_break(then) || has_break(els),
        _ => false,
    })
}

fn texpr(kind: TK, ty: Ty) -> TExpr {
    TExpr { kind, ty }
}

impl Checker {
    fn tn(&self, t: &Ty) -> String {
        ty_name(&self.classes, t)
    }

    fn is_subclass(&self, mut d: ClassId, b: ClassId) -> bool {
        loop {
            if d == b {
                return true;
            }
            match self.classes[d].base {
                Some(p) => d = p,
                None => return false,
            }
        }
    }

    /// Екі типтің ортақ типі (класстар үшін — ең жақын ортақ ата-ана).
    fn common_ty(&self, a: &Ty, b: &Ty) -> Option<Ty> {
        if a == b {
            return Some(a.clone());
        }
        if let (Ty::Class(x), Ty::Class(y)) = (a, b) {
            let mut cur = Some(*x);
            while let Some(c) = cur {
                if self.is_subclass(*y, c) {
                    return Some(Ty::Class(c));
                }
                cur = self.classes[c].base;
            }
        }
        None
    }

    fn is_overridden(&self, cid: ClassId, m: &str, fid: FuncId) -> bool {
        for &ch in &self.class_children[cid] {
            if self.class_methods[ch].get(m) != Some(&fid) || self.is_overridden(ch, m, fid) {
                return true;
            }
        }
        false
    }

    fn mismatch<T>(&self, pos: Pos, expected: &Ty, got: &Ty) -> Res<T> {
        err(pos, format!("тип сәйкес емес: `{}` күтілді, бірақ `{}` табылды", self.tn(expected), self.tn(got)))
    }

    fn resolve_type(&self, t: &TypeExpr) -> Res<Ty> {
        match t {
            TypeExpr::Name(n, p) => Ok(match n.as_str() {
                "бүтін" => Ty::Int,
                "нақты" => Ty::Float,
                "логика" => Ty::Bool,
                "байт" => Ty::Byte,
                "мәтін" => Ty::Str,
                "жоқ" => Ty::None,
                "тізім" | "сөздік" | "жиын" => return err(*p, format!("`{}` үшін элемент типін көрсетіңіз: `{}[...]`", n, n)),
                other => match self.class_names.get(other) {
                    Some(c) => Ty::Class(*c),
                    None => return err(*p, format!("белгісіз тип `{}`", other)),
                },
            }),
            TypeExpr::Generic(n, args, p) => match n.as_str() {
                "тізім" => {
                    if args.len() != 1 {
                        return err(*p, "`тізім` бір тип параметрін алады: `тізім[бүтін]`");
                    }
                    let e = self.resolve_type(&args[0])?;
                    if e.is_unit() {
                        return err(*p, "тізім элементі `жоқ` бола алмайды");
                    }
                    Ok(Ty::List(Box::new(e)))
                }
                "жиын" => {
                    if args.len() != 1 {
                        return err(*p, "`жиын` бір тип параметрін алады: `жиын[бүтін]`");
                    }
                    let e = self.resolve_type(&args[0])?;
                    if !matches!(e, Ty::Int | Ty::Str | Ty::Bool | Ty::Byte) {
                        return err(*p, "жиын элементі `бүтін`, `мәтін`, `логика` немесе `байт` болуы керек");
                    }
                    Ok(Ty::Set(Box::new(e)))
                }
                "сөздік" => {
                    if args.len() != 2 {
                        return err(*p, "`сөздік` екі тип параметрін алады: `сөздік[мәтін, бүтін]`");
                    }
                    let k = self.resolve_type(&args[0])?;
                    let v = self.resolve_type(&args[1])?;
                    if !matches!(k, Ty::Int | Ty::Str | Ty::Bool | Ty::Byte) {
                        return err(*p, "сөздік кілті `бүтін`, `мәтін`, `логика` немесе `байт` болуы керек");
                    }
                    if v.is_unit() {
                        return err(*p, "сөздік мәні `жоқ` бола алмайды");
                    }
                    Ok(Ty::Dict(Box::new(k), Box::new(v)))
                }
                _ => err(*p, format!("`{}` тип параметрлерін қабылдамайды", n)),
            },
            TypeExpr::Ptr(inner, _) => Ok(Ty::Ptr(Box::new(self.resolve_type(inner)?))),
            TypeExpr::Func(ps, r, _) => {
                let pts = ps.iter().map(|t| self.resolve_type(t)).collect::<Res<Vec<_>>>()?;
                let rt = self.resolve_type(r)?;
                Ok(Ty::Func(pts, Box::new(rt)))
            }
            TypeExpr::Tuple(items, p) => {
                let ts = items.iter().map(|t| self.resolve_type(t)).collect::<Res<Vec<_>>>()?;
                if ts.iter().any(|t| t.is_unit()) {
                    return err(*p, "кортеж элементі `жоқ` бола алмайды");
                }
                Ok(Ty::Tuple(ts))
            }
        }
    }

    /// Өрнек түрінде жазылған типті (`тізім[бүтін]`, `*байт`, `Нүкте`) типке айналдырады.
    fn expr_as_type(&self, e: &Expr) -> Res<Ty> {
        let te = match &e.kind {
            ExprKind::Name(n) => TypeExpr::Name(n.clone(), e.pos),
            ExprKind::Unary(UnOp::Deref, inner) => TypeExpr::Ptr(Box::new(self.expr_to_typeexpr(inner)?), e.pos),
            ExprKind::Index(..) | ExprKind::Tuple(_) => self.expr_to_typeexpr(e)?,
            _ => return err(e.pos, "тип күтілді"),
        };
        self.resolve_type(&te)
    }

    fn expr_to_typeexpr(&self, e: &Expr) -> Res<TypeExpr> {
        match &e.kind {
            ExprKind::Name(n) => Ok(TypeExpr::Name(n.clone(), e.pos)),
            ExprKind::Unary(UnOp::Deref, inner) => Ok(TypeExpr::Ptr(Box::new(self.expr_to_typeexpr(inner)?), e.pos)),
            ExprKind::Tuple(v) => Ok(TypeExpr::Tuple(v.iter().map(|x| self.expr_to_typeexpr(x)).collect::<Res<Vec<_>>>()?, e.pos)),
            ExprKind::Index(base, idx) => {
                let name = match &base.kind {
                    ExprKind::Name(n) => n.clone(),
                    _ => return err(e.pos, "тип күтілді"),
                };
                let args = match &idx.kind {
                    ExprKind::Tuple(v) => v.iter().map(|x| self.expr_to_typeexpr(x)).collect::<Res<Vec<_>>>()?,
                    _ => vec![self.expr_to_typeexpr(idx)?],
                };
                Ok(TypeExpr::Generic(name, args, e.pos))
            }
            _ => err(e.pos, "тип күтілді"),
        }
    }

    fn is_type_name(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Name(n) => {
                matches!(n.as_str(), "бүтін" | "нақты" | "логика" | "байт" | "мәтін") || self.class_names.contains_key(n)
            }
            ExprKind::Unary(UnOp::Deref, inner) => self.is_type_name(inner),
            ExprKind::Tuple(v) => !v.is_empty() && v.iter().all(|x| self.is_type_name(x)),
            ExprKind::Index(b, _) => matches!(&b.kind, ExprKind::Name(n) if n == "тізім" || n == "сөздік" || n == "жиын"),
            _ => false,
        }
    }

    // ---------------------------------------------------------------- айнымалылар

    fn new_local(&mut self, name: &str, ty: Ty) -> LocalId {
        let id = self.cx.locals.len();
        self.cx.locals.push(Local { name: name.to_string(), ty });
        id
    }

    fn temp(&mut self, ty: Ty) -> LocalId {
        self.cx.tmp += 1;
        let n = format!("$t{}", self.cx.tmp);
        self.new_local(&n, ty)
    }

    fn lookup_var(&self, name: &str) -> Option<(Target, Ty)> {
        if self.cx.declared_global.contains(name) {
            let g = *self.global_ids.get(name)?;
            return Some((Target::Global(g), self.globals[g].ty.clone()));
        }
        if let Some(&l) = self.cx.names.get(name) {
            return Some((Target::Local(l), self.cx.locals[l].ty.clone()));
        }
        if let Some(&g) = self.global_ids.get(name) {
            return Some((Target::Global(g), self.globals[g].ty.clone()));
        }
        None
    }

    /// Жазуға болатын (қолданыстағы) айнымалы: функцияда тек локал немесе `глобал`, main-да глобал да.
    fn find_assignable(&self, name: &str) -> Option<(Target, Ty)> {
        if self.cx.declared_global.contains(name) {
            let g = *self.global_ids.get(name)?;
            return Some((Target::Global(g), self.globals[g].ty.clone()));
        }
        if let Some(&l) = self.cx.names.get(name) {
            return Some((Target::Local(l), self.cx.locals[l].ty.clone()));
        }
        if self.cx.is_main {
            if let Some(&g) = self.global_ids.get(name) {
                return Some((Target::Global(g), self.globals[g].ty.clone()));
            }
        }
        None
    }

    fn check_not_reserved(&self, name: &str, pos: Pos) -> Res<()> {
        if BUILTIN_NAMES.contains(&name) {
            return err(pos, format!("`{}` — ішкі атау, оны қайта анықтауға болмайды", name));
        }
        Ok(())
    }

    fn create_var(&mut self, name: &str, ty: Ty, pos: Pos) -> Res<Target> {
        self.check_not_reserved(name, pos)?;
        if self.cx.is_main && self.global_names.contains(name) {
            let id = self.globals.len();
            self.globals.push(GlobalInfo { name: name.to_string(), ty });
            self.global_ids.insert(name.to_string(), id);
            Ok(Target::Global(id))
        } else {
            let id = self.new_local(name, ty);
            self.cx.names.insert(name.to_string(), id);
            Ok(Target::Local(id))
        }
    }

    fn set_var(&self, t: Target, v: TExpr) -> TStmt {
        match t {
            Target::Local(l) => TStmt::SetLocal(l, v),
            Target::Global(g) => TStmt::SetGlobal(g, v),
        }
    }

    // ---------------------------------------------------------------- түрлендірулер

    fn coerce(&self, e: TExpr, target: &Ty, pos: Pos) -> Res<TExpr> {
        if &e.ty == target {
            return Ok(e);
        }
        match (&e.ty, target) {
            (Ty::Int, Ty::Float) => {
                if let TK::Int(v) = e.kind {
                    return Ok(texpr(TK::Float(v as f64), Ty::Float));
                }
                Ok(texpr(TK::Cast(CastKind::IntToFloat, Box::new(e)), Ty::Float))
            }
            (Ty::Byte, Ty::Int) => Ok(texpr(TK::Cast(CastKind::ByteToInt, Box::new(e)), Ty::Int)),
            (Ty::Byte, Ty::Float) => {
                let i = texpr(TK::Cast(CastKind::ByteToInt, Box::new(e)), Ty::Int);
                Ok(texpr(TK::Cast(CastKind::IntToFloat, Box::new(i)), Ty::Float))
            }
            (Ty::Int, Ty::Byte) => {
                if let TK::Int(v) = e.kind {
                    if (0..=255).contains(&v) {
                        return Ok(texpr(TK::Int(v), Ty::Byte));
                    }
                }
                self.mismatch(pos, target, &e.ty)
            }
            (Ty::Null, t) if t.is_nullable() => Ok(texpr(TK::Null, t.clone())),
            (Ty::Class(d), Ty::Class(b)) if self.is_subclass(*d, *b) => Ok(texpr(e.kind, target.clone())),
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                if let TK::TupleLit(items) = e.kind {
                    let mut out = Vec::new();
                    for (it, t) in items.into_iter().zip(b) {
                        out.push(self.coerce(it, t, pos)?);
                    }
                    return Ok(texpr(TK::TupleLit(out), target.clone()));
                }
                self.mismatch(pos, target, &e.ty)
            }
            _ => self.mismatch(pos, target, &e.ty),
        }
    }

    fn to_int(&self, e: TExpr, pos: Pos) -> Res<TExpr> {
        self.coerce(e, &Ty::Int, pos)
    }

    /// Шартты bool-ға келтіреді (Python сияқты truthiness).
    fn truthy(&self, e: TExpr, pos: Pos) -> Res<TExpr> {
        let z = |k, t| texpr(k, t);
        Ok(match &e.ty {
            Ty::Bool => e,
            Ty::Int => texpr(TK::Cmp(CmpOp::Ne, Box::new(e), Box::new(z(TK::Int(0), Ty::Int))), Ty::Bool),
            Ty::Byte => {
                let i = texpr(TK::Cast(CastKind::ByteToInt, Box::new(e)), Ty::Int);
                texpr(TK::Cmp(CmpOp::Ne, Box::new(i), Box::new(z(TK::Int(0), Ty::Int))), Ty::Bool)
            }
            Ty::Float => texpr(TK::Cmp(CmpOp::Ne, Box::new(e), Box::new(z(TK::Float(0.0), Ty::Float))), Ty::Bool),
            Ty::Str | Ty::List(_) | Ty::Dict(..) | Ty::Set(_) => {
                let l = texpr(TK::Builtin(Bi::Len, vec![e]), Ty::Int);
                texpr(TK::Cmp(CmpOp::Gt, Box::new(l), Box::new(z(TK::Int(0), Ty::Int))), Ty::Bool)
            }
            Ty::Func(..) => texpr(TK::Bool(true), Ty::Bool),
            Ty::Class(_) | Ty::Ptr(_) => {
                let t = e.ty.clone();
                texpr(TK::Cmp(CmpOp::Ne, Box::new(e), Box::new(z(TK::Null, t))), Ty::Bool)
            }
            Ty::Null => texpr(TK::Bool(false), Ty::Bool),
            Ty::Tuple(_) => texpr(TK::Bool(true), Ty::Bool),
            Ty::None => return err(pos, "бұл өрнек мән қайтармайды"),
        })
    }

    fn need_value(&self, e: &TExpr, pos: Pos) -> Res<()> {
        if e.ty.is_unit() {
            return err(pos, "бұл өрнек мән қайтармайды");
        }
        Ok(())
    }

    fn promote(&self, a: TExpr, b: TExpr, pos: Pos) -> Res<(TExpr, TExpr, Ty)> {
        let ok = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Byte);
        if !ok(&a.ty) || !ok(&b.ty) {
            return err(pos, format!("`{}` және `{}` типтерімен бұл амал орындалмайды", self.tn(&a.ty), self.tn(&b.ty)));
        }
        let target = if a.ty == Ty::Float || b.ty == Ty::Float { Ty::Float } else { Ty::Int };
        let a = self.coerce(a, &target, pos)?;
        let b = self.coerce(b, &target, pos)?;
        Ok((a, b, target))
    }

    // ---------------------------------------------------------------- өрнектер

    fn expr(&mut self, e: &Expr, exp: Option<&Ty>) -> Res<TExpr> {
        let pos = e.pos;
        match &e.kind {
            ExprKind::Int(v) => Ok(texpr(TK::Int(*v), Ty::Int)),
            ExprKind::Float(v) => Ok(texpr(TK::Float(*v), Ty::Float)),
            ExprKind::Str(s) => Ok(texpr(TK::Str(s.clone()), Ty::Str)),
            ExprKind::Bool(b) => Ok(texpr(TK::Bool(*b), Ty::Bool)),
            ExprKind::None => Ok(texpr(TK::Null, Ty::Null)),
            ExprKind::Name(n) => match self.lookup_var(n) {
                Some((Target::Local(l), t)) => Ok(texpr(TK::Local(l), t)),
                Some((Target::Global(g), t)) => Ok(texpr(TK::Global(g), t)),
                None => {
                    if n == "ПИ" {
                        return Ok(texpr(TK::Float(core::f64::consts::PI), Ty::Float));
                    }
                    if n == "Е" {
                        return Ok(texpr(TK::Float(core::f64::consts::E), Ty::Float));
                    }
                    if let Some(&fid) = self.func_names.get(n) {
                        let ret = self.func_ret(fid, pos, false)?;
                        let ps: Vec<Ty> = self.funcs[fid].params.iter().map(|p| p.1.clone()).collect();
                        Ok(texpr(TK::FuncRef(fid), Ty::Func(ps, Box::new(ret))))
                    } else if self.global_names.contains(n) && !self.cx.is_main {
                        err(pos, format!("`{}` глобал айнымалысы функция шақырылғанға дейін әлі анықталмаған", n))
                    } else {
                        err(pos, format!("анықталмаған атау `{}`", n))
                    }
                }
            },
            ExprKind::FStr(parts) => self.fstr(parts, pos),
            ExprKind::Unary(op, a) => self.unary(*op, a, pos, exp),
            ExprKind::Binary(op, a, b) => {
                let l = self.expr(a, None)?;
                let r = self.expr(b, None)?;
                self.binary(*op, l, r, pos)
            }
            ExprKind::Cmp(op, a, b) => {
                let l = self.expr(a, None)?;
                let r = self.expr(b, None)?;
                self.compare(*op, l, r, pos)
            }
            ExprKind::Contains { item, container, negated } => {
                let c = self.expr(container, None)?;
                let it = match &c.ty {
                    Ty::List(t) => {
                        let t = (**t).clone();
                        let i = self.expr(item, Some(&t))?;
                        self.coerce(i, &t, item.pos)?
                    }
                    Ty::Dict(k, _) => {
                        let k = (**k).clone();
                        let i = self.expr(item, Some(&k))?;
                        self.coerce(i, &k, item.pos)?
                    }
                    Ty::Set(k) => {
                        let k = (**k).clone();
                        let i = self.expr(item, Some(&k))?;
                        self.coerce(i, &k, item.pos)?
                    }
                    Ty::Str => {
                        let i = self.expr(item, None)?;
                        self.coerce(i, &Ty::Str, item.pos)?
                    }
                    other => return err(pos, format!("`{}` типінде `ішінде` амалы жоқ", self.tn(other))),
                };
                Ok(texpr(TK::Contains { item: Box::new(it), container: Box::new(c), negated: *negated }, Ty::Bool))
            }
            ExprKind::And(a, b) => {
                let l = self.expr(a, None)?;
                let l = self.truthy(l, a.pos)?;
                let r = self.expr(b, None)?;
                let r = self.truthy(r, b.pos)?;
                Ok(texpr(TK::And(Box::new(l), Box::new(r)), Ty::Bool))
            }
            ExprKind::Or(a, b) => {
                let l = self.expr(a, None)?;
                let l = self.truthy(l, a.pos)?;
                let r = self.expr(b, None)?;
                let r = self.truthy(r, b.pos)?;
                Ok(texpr(TK::Or(Box::new(l), Box::new(r)), Ty::Bool))
            }
            ExprKind::Ternary { cond, then, els } => {
                let c = self.expr(cond, None)?;
                let c = self.truthy(c, cond.pos)?;
                let a = self.expr(then, exp)?;
                let b = self.expr(els, exp)?;
                self.need_value(&a, then.pos)?;
                self.need_value(&b, els.pos)?;
                let (a, b, ty) = if a.ty == b.ty {
                    let t = a.ty.clone();
                    (a, b, t)
                } else if a.ty.is_numeric() && b.ty.is_numeric() {
                    self.promote(a, b, pos)?
                } else if let Some(t) = self.common_ty(&a.ty, &b.ty) {
                    let a2 = self.coerce(a, &t, pos)?;
                    let b2 = self.coerce(b, &t, pos)?;
                    (a2, b2, t)
                } else if a.ty == Ty::Null && b.ty.is_nullable() {
                    let t = b.ty.clone();
                    (self.coerce(a, &t, pos)?, b, t)
                } else if b.ty == Ty::Null && a.ty.is_nullable() {
                    let t = a.ty.clone();
                    let b2 = self.coerce(b, &t, pos)?;
                    (a, b2, t)
                } else {
                    return err(pos, format!("шарт өрнегінің тармақтарының типтері әртүрлі: `{}` және `{}`", self.tn(&a.ty), self.tn(&b.ty)));
                };
                Ok(texpr(TK::Ternary(Box::new(c), Box::new(a), Box::new(b)), ty))
            }
            ExprKind::Index(base, idx) => self.index(base, idx, pos),
            ExprKind::Slice(base, lo, hi) => {
                let b = self.expr(base, None)?;
                if !matches!(b.ty, Ty::Str | Ty::List(_)) {
                    return err(pos, format!("`{}` типін кесуге болмайды", self.tn(&b.ty)));
                }
                let lo = match lo {
                    Some(x) => {
                        let v = self.expr(x, None)?;
                        Some(Box::new(self.to_int(v, x.pos)?))
                    }
                    None => None,
                };
                let hi = match hi {
                    Some(x) => {
                        let v = self.expr(x, None)?;
                        Some(Box::new(self.to_int(v, x.pos)?))
                    }
                    None => None,
                };
                let ty = b.ty.clone();
                Ok(texpr(TK::Slice(Box::new(b), lo, hi), ty))
            }
            ExprKind::Field(obj, name) => {
                let o = self.expr(obj, None)?;
                match &o.ty {
                    Ty::Class(c) => {
                        let ci = &self.classes[*c];
                        match ci.fields.iter().position(|f| &f.name == name) {
                            Some(i) => {
                                let ty = ci.fields[i].ty.clone();
                                Ok(texpr(TK::Field(Box::new(o), i), ty))
                            }
                            None => err(pos, format!("`{}` класында `{}` өрісі жоқ", ci.name, name)),
                        }
                    }
                    other => err(pos, format!("`{}` типінде `{}` өрісі жоқ", self.tn(other), name)),
                }
            }
            ExprKind::List(items) => {
                let elem_exp = match exp {
                    Some(Ty::List(t)) => Some((**t).clone()),
                    _ => None,
                };
                if items.is_empty() {
                    return match elem_exp {
                        Some(t) => Ok(texpr(TK::ListLit(vec![]), Ty::List(Box::new(t)))),
                        None => err(pos, "бос тізімнің типі белгісіз: айнымалыға тип көрсетіңіз (`x: тізім[бүтін] = []`)"),
                    };
                }
                let mut vals = Vec::new();
                for it in items {
                    let v = self.expr(it, elem_exp.as_ref())?;
                    self.need_value(&v, it.pos)?;
                    vals.push(v);
                }
                let t = match elem_exp {
                    Some(t) => t,
                    None => {
                        let mut t = vals[0].ty.clone();
                        if vals.iter().any(|v| v.ty == Ty::Float) && vals.iter().all(|v| v.ty.is_numeric()) {
                            t = Ty::Float;
                        }
                        if let Ty::Class(_) = t {
                            for v in &vals {
                                match self.common_ty(&t, &v.ty) {
                                    Some(c) => t = c,
                                    None if v.ty == Ty::Null => {}
                                    None => break,
                                }
                            }
                        }
                        if t == Ty::Null {
                            return err(pos, "тізім элементінің типін анықтау мүмкін емес");
                        }
                        t
                    }
                };
                let mut out = Vec::new();
                for (v, it) in vals.into_iter().zip(items) {
                    out.push(self.coerce(v, &t, it.pos)?);
                }
                Ok(texpr(TK::ListLit(out), Ty::List(Box::new(t))))
            }
            ExprKind::Dict(items) => {
                let (kexp, vexp) = match exp {
                    Some(Ty::Dict(k, v)) => (Some((**k).clone()), Some((**v).clone())),
                    _ => (None, None),
                };
                if items.is_empty() {
                    if let Some(Ty::Set(t)) = exp {
                        return Ok(texpr(TK::SetLit(vec![]), Ty::Set(t.clone())));
                    }
                    return match (kexp, vexp) {
                        (Some(k), Some(v)) => Ok(texpr(TK::DictLit(vec![]), Ty::Dict(Box::new(k), Box::new(v)))),
                        _ => err(pos, "бос сөздіктің типі белгісіз: айнымалыға тип көрсетіңіз (`x: сөздік[мәтін, бүтін] = {}`)"),
                    };
                }
                let mut ks = Vec::new();
                let mut vs = Vec::new();
                for (k, v) in items {
                    ks.push(self.expr(k, kexp.as_ref())?);
                    vs.push(self.expr(v, vexp.as_ref())?);
                }
                let kt = kexp.unwrap_or_else(|| ks[0].ty.clone());
                let vt = match vexp {
                    Some(t) => t,
                    None => {
                        let mut t = vs[0].ty.clone();
                        if vs.iter().any(|v| v.ty == Ty::Float) && vs.iter().all(|v| v.ty.is_numeric()) {
                            t = Ty::Float;
                        }
                        t
                    }
                };
                if !matches!(kt, Ty::Int | Ty::Str | Ty::Bool | Ty::Byte) {
                    return err(pos, "сөздік кілті `бүтін`, `мәтін`, `логика` немесе `байт` болуы керек");
                }
                if vt == Ty::Null || vt.is_unit() {
                    return err(pos, "сөздік мәнінің типін анықтау мүмкін емес");
                }
                let mut out = Vec::new();
                for ((k, v), (ke, ve)) in ks.into_iter().zip(vs).zip(items.iter()) {
                    out.push((self.coerce(k, &kt, ke.pos)?, self.coerce(v, &vt, ve.pos)?));
                }
                Ok(texpr(TK::DictLit(out), Ty::Dict(Box::new(kt), Box::new(vt))))
            }
            ExprKind::Lambda { params, body } => self.lambda(params, body, exp, pos),
            ExprKind::Set(items) => {
                let elem_exp = match exp {
                    Some(Ty::Set(t)) => Some((**t).clone()),
                    _ => None,
                };
                let mut vals = Vec::new();
                for it in items {
                    let v = self.expr(it, elem_exp.as_ref())?;
                    self.need_value(&v, it.pos)?;
                    vals.push(v);
                }
                let t = elem_exp.unwrap_or_else(|| vals[0].ty.clone());
                if !matches!(t, Ty::Int | Ty::Str | Ty::Bool | Ty::Byte) {
                    return err(pos, "жиын элементі `бүтін`, `мәтін`, `логика` немесе `байт` болуы керек");
                }
                let mut out = Vec::new();
                for (v, it) in vals.into_iter().zip(items) {
                    out.push(self.coerce(v, &t, it.pos)?);
                }
                Ok(texpr(TK::SetLit(out), Ty::Set(Box::new(t))))
            }
            ExprKind::Tuple(items) => {
                let exps: Vec<Option<Ty>> = match exp {
                    Some(Ty::Tuple(ts)) if ts.len() == items.len() => ts.iter().cloned().map(Some).collect(),
                    _ => vec![None; items.len()],
                };
                let mut out = Vec::new();
                let mut tys = Vec::new();
                for (it, ex) in items.iter().zip(&exps) {
                    let v = self.expr(it, ex.as_ref())?;
                    self.need_value(&v, it.pos)?;
                    if v.ty == Ty::Null {
                        return err(it.pos, "кортеж элементінің типін анықтау мүмкін емес");
                    }
                    tys.push(v.ty.clone());
                    out.push(v);
                }
                Ok(texpr(TK::TupleLit(out), Ty::Tuple(tys)))
            }
            ExprKind::ListComp { elem, targets, iter, cond } => {
                let saved: Vec<(String, Option<LocalId>)> = targets.iter().map(|(n, _)| (n.clone(), self.cx.names.get(n).copied())).collect();
                for (n, _) in targets {
                    self.cx.names.remove(n);
                }
                self.cx.comp += 1;
                let r = self.list_comp(elem, targets, iter, cond.as_deref(), exp, pos);
                self.cx.comp -= 1;
                for (n, old) in saved {
                    match old {
                        Some(id) => {
                            self.cx.names.insert(n, id);
                        }
                        None => {
                            self.cx.names.remove(&n);
                        }
                    }
                }
                r
            }
            ExprKind::Call(f, args) => self.call(f, args, pos, exp),
        }
    }

    fn lambda(&mut self, params: &[(String, Pos)], body: &Expr, exp: Option<&Ty>, pos: Pos) -> Res<TExpr> {
        let (pts, rt) = match exp {
            Some(Ty::Func(ps, r)) => {
                if ps.len() != params.len() {
                    return err(pos, format!("лямбда {} параметр алуы керек, бірақ {} берілген", ps.len(), params.len()));
                }
                (ps.clone(), (**r).clone())
            }
            _ => {
                return err(pos, "лямбданың типін анықтау мүмкін емес: оны функция типті айнымалыға/параметрге беріңіз, мысалы `f: (бүтін) -> бүтін = лямбда x: x + 1`")
            }
        };
        let fid = self.funcs.len();
        let name = format!("<лямбда {}>", fid);
        let stmt_body = if rt == Ty::None {
            Stmt { kind: StmtKind::Expr(body.clone()), pos: body.pos }
        } else {
            Stmt { kind: StmtKind::Return(Some(body.clone())), pos: body.pos }
        };
        let ast = FuncDef { name: name.clone(), params: vec![], ret: None, body: vec![stmt_body], external: false, pos };
        let declared = if rt == Ty::Null { None } else { Some(rt.clone()) };
        let mut seen = HashSet::new();
        for (n, p) in params {
            if !seen.insert(n.clone()) {
                return err(*p, format!("`{}` параметрі қайталанды", n));
            }
        }
        self.funcs.push(FuncSlot {
            name,
            params: params.iter().map(|p| p.0.clone()).zip(pts.iter().cloned()).collect(),
            defaults: vec![None; params.len()],
            ret: declared,
            ast: Rc::new(ast),
            state: St::Todo,
            done: None,
            final_ret: None,
        });
        let ret = self.func_ret(fid, pos, false)?;
        Ok(texpr(TK::FuncRef(fid), Ty::Func(pts, Box::new(ret))))
    }

    fn call_value(&mut self, callee: TExpr, args: &[Arg], pos: Pos) -> Res<TExpr> {
        let (ps, r) = match &callee.ty {
            Ty::Func(ps, r) => (ps.clone(), (**r).clone()),
            other => return err(pos, format!("`{}` типін шақыруға болмайды", self.tn(other))),
        };
        if args.iter().any(|a| a.name.is_some()) {
            return err(pos, "функция мәнін шақырғанда атаулы аргументтер қолданылмайды");
        }
        if args.len() != ps.len() {
            return err(pos, format!("функция {} аргумент алады, бірақ {} берілді", ps.len(), args.len()));
        }
        let mut out = Vec::new();
        for (a, p) in args.iter().zip(&ps) {
            out.push(self.arg(a, Some(p))?);
        }
        Ok(texpr(TK::CallValue(Box::new(callee), out), r))
    }

    fn fstr(&mut self, parts: &[FStrPart], _pos: Pos) -> Res<TExpr> {
        let mut out = Vec::new();
        for p in parts {
            match p {
                FStrPart::Lit(s) => out.push(FPartT::Lit(s.clone())),
                FStrPart::Expr { expr, spec } => {
                    let v = self.expr(expr, None)?;
                    self.need_value(&v, expr.pos)?;
                    out.push(FPartT::Val(self.format_value(v, spec.as_deref(), expr.pos)?));
                }
            }
        }
        Ok(texpr(TK::FStr(out), Ty::Str))
    }

    fn to_str(&self, v: TExpr) -> TExpr {
        if v.ty == Ty::Str {
            v
        } else {
            texpr(TK::Builtin(Bi::ToStr, vec![v]), Ty::Str)
        }
    }

    fn format_value(&self, v: TExpr, spec: Option<&str>, pos: Pos) -> Res<TExpr> {
        let spec = match spec {
            None => return Ok(self.to_str(v)),
            Some(s) => s,
        };
        let bad = || err(pos, format!("форматтау спецификациясы қате: `{}`", spec));
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;
        let mut align: Option<u8> = None;
        if i < chars.len() {
            match chars[i] {
                '<' => {
                    align = Some(0);
                    i += 1;
                }
                '>' => {
                    align = Some(1);
                    i += 1;
                }
                '^' => {
                    align = Some(2);
                    i += 1;
                }
                _ => {}
            }
        }
        let mut width = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            width.push(chars[i]);
            i += 1;
        }
        let mut prec: Option<u32> = None;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            let mut p = String::new();
            while i < chars.len() && chars[i].is_ascii_digit() {
                p.push(chars[i]);
                i += 1;
            }
            if p.is_empty() {
                return bad();
            }
            prec = Some(p.parse().map_err(|_| Diag::new(pos, "дәлдік тым үлкен"))?);
        }
        let mut kind = None;
        if i < chars.len() {
            kind = Some(chars[i]);
            i += 1;
        }
        if i != chars.len() {
            return bad();
        }
        let numeric = v.ty.is_numeric();
        let mut s = match (prec, kind) {
            (Some(p), Some('f') | None) => {
                if !numeric {
                    return bad();
                }
                let f = self.coerce(v, &Ty::Float, pos)?;
                texpr(TK::Builtin(Bi::FloatFixed(p), vec![f]), Ty::Str)
            }
            (None, Some('f')) => {
                let f = self.coerce(v, &Ty::Float, pos)?;
                texpr(TK::Builtin(Bi::FloatFixed(6), vec![f]), Ty::Str)
            }
            (None, Some('d') | Some('s') | None) => self.to_str(v),
            _ => return bad(),
        };
        if !width.is_empty() {
            let w: i64 = width.parse().map_err(|_| Diag::new(pos, "ені тым үлкен"))?;
            let al = align.unwrap_or(if numeric { 1 } else { 0 });
            s = texpr(TK::Builtin(Bi::StrPad(al), vec![s, texpr(TK::Int(w), Ty::Int)]), Ty::Str);
        }
        Ok(s)
    }

    fn unary(&mut self, op: UnOp, a: &Expr, pos: Pos, _exp: Option<&Ty>) -> Res<TExpr> {
        match op {
            UnOp::Deref => {
                self.need_manual(pos)?;
                let p = self.expr(a, None)?;
                match &p.ty {
                    Ty::Ptr(t) => {
                        let t = (**t).clone();
                        Ok(texpr(TK::Deref(Box::new(p)), t))
                    }
                    other => err(pos, format!("`*` тек көрсеткіш (`*T`) үшін жұмыс істейді, бірақ `{}` табылды", self.tn(other))),
                }
            }
            UnOp::Not => {
                let v = self.expr(a, None)?;
                let v = self.truthy(v, a.pos)?;
                Ok(texpr(TK::Unary(UnOp::Not, Box::new(v)), Ty::Bool))
            }
            UnOp::Neg | UnOp::Pos => {
                let v = self.expr(a, None)?;
                let v = if v.ty == Ty::Byte { self.to_int(v, a.pos)? } else { v };
                if !matches!(v.ty, Ty::Int | Ty::Float) {
                    return err(pos, format!("`{}` типіне таңба қолдануға болмайды", self.tn(&v.ty)));
                }
                if op == UnOp::Pos {
                    return Ok(v);
                }
                let t = v.ty.clone();
                Ok(texpr(TK::Unary(UnOp::Neg, Box::new(v)), t))
            }
            UnOp::BitNot => {
                let v = self.expr(a, None)?;
                let v = self.to_int(v, a.pos)?;
                Ok(texpr(TK::Unary(UnOp::BitNot, Box::new(v)), Ty::Int))
            }
        }
    }

    fn need_manual(&self, pos: Pos) -> Res<()> {
        if self.cx.manual == 0 {
            return err(pos, "көрсеткіштермен және қолмен жадпен жұмыс тек `қолмен:` блогында рұқсат етіледі");
        }
        Ok(())
    }

    fn binary(&mut self, op: BinOp, l: TExpr, r: TExpr, pos: Pos) -> Res<TExpr> {
        self.need_value(&l, pos)?;
        self.need_value(&r, pos)?;
        let bad = |s: &Checker, l: &TExpr, r: &TExpr| -> Res<TExpr> {
            err(pos, format!("`{}` және `{}` типтеріне бұл амал қолданылмайды", s.tn(&l.ty), s.tn(&r.ty)))
        };
        match op {
            BinOp::Add => match (&l.ty, &r.ty) {
                (Ty::Str, Ty::Str) => Ok(texpr(TK::Binary(op, Box::new(l), Box::new(r)), Ty::Str)),
                (Ty::List(a), Ty::List(b)) if a == b => {
                    let t = l.ty.clone();
                    Ok(texpr(TK::Binary(op, Box::new(l), Box::new(r)), t))
                }
                (Ty::Ptr(_), Ty::Int) => {
                    let t = l.ty.clone();
                    Ok(texpr(TK::Binary(op, Box::new(l), Box::new(r)), t))
                }
                _ => {
                    let (a, b, t) = self.promote(l, r, pos)?;
                    Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), t))
                }
            },
            BinOp::Mul => match (&l.ty, &r.ty) {
                (Ty::Str, Ty::Int) | (Ty::List(_), Ty::Int) => {
                    let t = l.ty.clone();
                    Ok(texpr(TK::Binary(op, Box::new(l), Box::new(r)), t))
                }
                (Ty::Int, Ty::Str) | (Ty::Int, Ty::List(_)) => {
                    let t = r.ty.clone();
                    Ok(texpr(TK::Binary(op, Box::new(r), Box::new(l)), t))
                }
                _ => {
                    let (a, b, t) = self.promote(l, r, pos)?;
                    Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), t))
                }
            },
            BinOp::Sub if matches!(l.ty, Ty::Set(_)) && l.ty == r.ty => {
                let t = l.ty.clone();
                Ok(texpr(TK::Builtin(Bi::SetOp(2), vec![l, r]), t))
            }
            BinOp::Sub => {
                if let (Ty::Ptr(_), Ty::Int) = (&l.ty, &r.ty) {
                    let t = l.ty.clone();
                    return Ok(texpr(TK::Binary(op, Box::new(l), Box::new(r)), t));
                }
                let (a, b, t) = self.promote(l, r, pos)?;
                Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), t))
            }
            BinOp::Div => {
                let (a, b, _) = self.promote(l, r, pos)?;
                let a = self.coerce(a, &Ty::Float, pos)?;
                let b = self.coerce(b, &Ty::Float, pos)?;
                Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), Ty::Float))
            }
            BinOp::FloorDiv => {
                let (a, b, t) = self.promote(l, r, pos)?;
                if t == Ty::Float {
                    let d = texpr(TK::Binary(BinOp::Div, Box::new(a), Box::new(b)), Ty::Float);
                    Ok(texpr(TK::Builtin(Bi::Floor, vec![d]), Ty::Float))
                } else {
                    Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), Ty::Int))
                }
            }
            BinOp::Mod => {
                let (a, b, t) = self.promote(l, r, pos)?;
                if t == Ty::Float {
                    Ok(texpr(TK::Builtin(Bi::FMod, vec![a, b]), Ty::Float))
                } else {
                    Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), Ty::Int))
                }
            }
            BinOp::Pow => {
                let (a, b, t) = self.promote(l, r, pos)?;
                if t == Ty::Float {
                    Ok(texpr(TK::Builtin(Bi::PowF, vec![a, b]), Ty::Float))
                } else {
                    Ok(texpr(TK::Builtin(Bi::PowI, vec![a, b]), Ty::Int))
                }
            }
            BinOp::Shl | BinOp::Shr => {
                let a = self.to_int(l, pos)?;
                let b = self.to_int(r, pos)?;
                Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), Ty::Int))
            }
            BinOp::BitAnd | BinOp::BitOr if matches!(l.ty, Ty::Set(_)) && l.ty == r.ty => {
                let t = l.ty.clone();
                let k = if op == BinOp::BitOr { 0 } else { 1 };
                Ok(texpr(TK::Builtin(Bi::SetOp(k), vec![l, r]), t))
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                if l.ty == Ty::Bool && r.ty == Ty::Bool {
                    return Ok(texpr(TK::Binary(op, Box::new(l), Box::new(r)), Ty::Bool));
                }
                if !matches!(l.ty, Ty::Int | Ty::Byte) || !matches!(r.ty, Ty::Int | Ty::Byte) {
                    return bad(self, &l, &r);
                }
                let a = self.to_int(l, pos)?;
                let b = self.to_int(r, pos)?;
                Ok(texpr(TK::Binary(op, Box::new(a), Box::new(b)), Ty::Int))
            }
        }
    }

    fn compare(&mut self, op: CmpOp, l: TExpr, r: TExpr, pos: Pos) -> Res<TExpr> {
        self.need_value(&l, pos)?;
        self.need_value(&r, pos)?;
        let eq = matches!(op, CmpOp::Eq | CmpOp::Ne);
        let (a, b) = if l.ty.is_numeric() && r.ty.is_numeric() {
            let (a, b, _) = self.promote(l, r, pos)?;
            (a, b)
        } else if l.ty == r.ty {
            match &l.ty {
                Ty::Str | Ty::Bool => (l, r),
                _ if eq => (l, r),
                other => return err(pos, format!("`{}` типін `<`, `>` арқылы салыстыруға болмайды", self.tn(other))),
            }
        } else if eq
            && matches!((&l.ty, &r.ty), (Ty::Class(a), Ty::Class(b)) if self.is_subclass(*a, *b) || self.is_subclass(*b, *a))
        {
            let common = self.common_ty(&l.ty, &r.ty).unwrap();
            let l2 = self.coerce(l, &common, pos)?;
            let r2 = self.coerce(r, &common, pos)?;
            (l2, r2)
        } else if eq && l.ty == Ty::Null && r.ty.is_nullable() {
            let t = r.ty.clone();
            (self.coerce(l, &t, pos)?, r)
        } else if eq && r.ty == Ty::Null && l.ty.is_nullable() {
            let t = l.ty.clone();
            let r2 = self.coerce(r, &t, pos)?;
            (l, r2)
        } else {
            return err(pos, format!("`{}` және `{}` типтерін салыстыруға болмайды", self.tn(&l.ty), self.tn(&r.ty)));
        };
        if eq && matches!(a.ty, Ty::List(_) | Ty::Dict(..) | Ty::Set(_) | Ty::Tuple(_)) && !matches!(b.kind, TK::Null) && !matches!(a.kind, TK::Null) {
            return Ok(texpr(TK::DeepEq(Box::new(a), Box::new(b), op == CmpOp::Ne), Ty::Bool));
        }
        Ok(texpr(TK::Cmp(op, Box::new(a), Box::new(b)), Ty::Bool))
    }

    fn index(&mut self, base: &Expr, idx: &Expr, pos: Pos) -> Res<TExpr> {
        let b = self.expr(base, None)?;
        match b.ty.clone() {
            Ty::List(t) => {
                let i = self.expr(idx, None)?;
                let i = self.to_int(i, idx.pos)?;
                Ok(texpr(TK::Index(Box::new(b), Box::new(i)), *t))
            }
            Ty::Str => {
                let i = self.expr(idx, None)?;
                let i = self.to_int(i, idx.pos)?;
                Ok(texpr(TK::Index(Box::new(b), Box::new(i)), Ty::Str))
            }
            Ty::Ptr(t) => {
                self.need_manual(pos)?;
                let i = self.expr(idx, None)?;
                let i = self.to_int(i, idx.pos)?;
                Ok(texpr(TK::Index(Box::new(b), Box::new(i)), *t))
            }
            Ty::Dict(k, v) => {
                let i = self.expr(idx, Some(&k))?;
                let i = self.coerce(i, &k, idx.pos)?;
                Ok(texpr(TK::Index(Box::new(b), Box::new(i)), *v))
            }
            Ty::Tuple(ts) => {
                let n = ts.len() as i64;
                let k = match idx.kind {
                    ExprKind::Int(v) => v,
                    _ => return err(idx.pos, "кортежді индекстеу үшін бүтін тұрақты қажет: `t[0]`"),
                };
                let k = if k < 0 { k + n } else { k };
                if k < 0 || k >= n {
                    return err(idx.pos, format!("кортеж индексі шектен тыс (элемент саны {})", n));
                }
                Ok(texpr(TK::TupleGet(Box::new(b), k as usize), ts[k as usize].clone()))
            }
            other => err(pos, format!("`{}` типін индекстеуге болмайды", self.tn(&other))),
        }
    }

    // ---------------------------------------------------------------- шақырулар

    fn call(&mut self, f: &Expr, args: &[Arg], pos: Pos, exp: Option<&Ty>) -> Res<TExpr> {
        let allow = std::mem::take(&mut self.cx.stmt_call);
        match &f.kind {
            ExprKind::Name(n) if matches!(self.lookup_var(n), Some((_, Ty::Func(..)))) => {
                let callee = self.expr(f, None)?;
                self.call_value(callee, args, pos)
            }
            ExprKind::Name(n) => {
                if let Some(&fid) = self.func_names.get(n) {
                    self.cx.stmt_call = allow;
                    let (call_args, ret) = self.match_args(fid, None, args, pos)?;
                    return Ok(texpr(TK::Call(fid, call_args), ret));
                }
                if let Some(&cid) = self.class_names.get(n) {
                    return self.construct(cid, args, pos);
                }
                self.builtin_call(n, args, pos, exp)
            }
            ExprKind::Index(b, _) if matches!(&b.kind, ExprKind::Name(n) if n == "тізім" || n == "сөздік" || n == "жиын") => {
                // бос контейнер: тізім[бүтін]()
                if !args.is_empty() {
                    return err(pos, "бос контейнер конструкторы аргумент қабылдамайды");
                }
                let t = self.expr_as_type(f)?;
                match t {
                    Ty::List(_) => Ok(texpr(TK::ListLit(vec![]), t)),
                    Ty::Dict(..) => Ok(texpr(TK::DictLit(vec![]), t)),
                    Ty::Set(_) => Ok(texpr(TK::SetLit(vec![]), t)),
                    _ => err(pos, "қате контейнер"),
                }
            }
            ExprKind::Field(obj, m) => {
                if let ExprKind::Name(cn) = &obj.kind {
                    if let (Some(&cid), None) = (self.class_names.get(cn), self.lookup_var(cn)) {
                        // Предок.әдіс(өзі, ...) — статикалық шақыру
                        let fid = match self.class_methods[cid].get(m) {
                            Some(f) => *f,
                            None => return err(pos, format!("`{}` класында `{}` әдісі жоқ", cn, m)),
                        };
                        self.cx.stmt_call = allow;
                        let (call_args, ret) = self.match_args(fid, None, args, pos)?;
                        return Ok(texpr(TK::Call(fid, call_args), ret));
                    }
                }
                let recv = self.expr(obj, None)?;
                self.cx.stmt_call = allow;
                self.method_call(recv, m, args, pos)
            }
            _ => {
                let callee = self.expr(f, None)?;
                if matches!(callee.ty, Ty::Func(..)) {
                    self.call_value(callee, args, pos)
                } else {
                    err(pos, "бұл өрнекті шақыруға болмайды")
                }
            }
        }
    }

    fn func_ret(&mut self, fid: FuncId, pos: Pos, allow_unknown: bool) -> Res<Ty> {
        if let Some(t) = &self.funcs[fid].ret {
            return Ok(t.clone());
        }
        match self.funcs[fid].state {
            St::Done => Ok(self.funcs[fid].final_ret.clone().unwrap_or(Ty::None)),
            // рекурсивті шақыру оператор болса, нәтиже қолданылмайды — типі маңызды емес
            St::Busy if allow_unknown => Ok(Ty::None),
            St::Busy => err(
                pos,
                format!("`{}` рекурсивті функциясына қайтару типін көрсетіңіз: `-> тип`", self.funcs[fid].name),
            ),
            St::Todo => {
                self.check_func(fid)?;
                Ok(self.funcs[fid].final_ret.clone().unwrap_or(Ty::None))
            }
        }
    }

    /// Аргументтерді параметрлермен салыстырады (орындық, атаулы, әдепкі).
    fn match_args(&mut self, fid: FuncId, recv: Option<TExpr>, args: &[Arg], pos: Pos) -> Res<(Vec<TExpr>, Ty)> {
        let allow_unknown = std::mem::take(&mut self.cx.stmt_call);
        let params = self.funcs[fid].params.clone();
        let defaults = self.funcs[fid].defaults.clone();
        let fname = self.funcs[fid].name.clone();
        let mut slots: Vec<Option<TExpr>> = vec![None; params.len()];
        let mut next = 0;
        if let Some(r) = recv {
            slots[0] = Some(r);
            next = 1;
        }
        let first_user = next;
        let mut seen_kw = false;
        for a in args {
            match &a.name {
                None => {
                    if seen_kw {
                        return err(a.value.pos, "орындық аргумент атаулы аргументтен кейін келе алмайды");
                    }
                    if next >= params.len() {
                        return err(
                            a.value.pos,
                            format!("`{}` функциясы {} аргумент алады, көбірек берілді", fname, params.len() - first_user),
                        );
                    }
                    let pty = params[next].1.clone();
                    let v = self.expr(&a.value, Some(&pty))?;
                    self.need_value(&v, a.value.pos)?;
                    slots[next] = Some(self.coerce(v, &pty, a.value.pos)?);
                    next += 1;
                }
                Some(name) => {
                    seen_kw = true;
                    let i = match params.iter().position(|(n, _)| n == name) {
                        Some(i) if i >= first_user => i,
                        _ => return err(a.value.pos, format!("`{}` функциясында `{}` параметрі жоқ", fname, name)),
                    };
                    if slots[i].is_some() {
                        return err(a.value.pos, format!("`{}` параметрі екі рет берілді", name));
                    }
                    let pty = params[i].1.clone();
                    let v = self.expr(&a.value, Some(&pty))?;
                    self.need_value(&v, a.value.pos)?;
                    slots[i] = Some(self.coerce(v, &pty, a.value.pos)?);
                }
            }
        }
        let mut out = Vec::new();
        for (i, s) in slots.into_iter().enumerate() {
            match s {
                Some(v) => out.push(v),
                None => match &defaults[i] {
                    Some(d) => out.push(d.clone()),
                    None => return err(pos, format!("`{}` функциясына `{}` аргументі жетіспейді", fname, params[i].0)),
                },
            }
        }
        let ret = self.func_ret(fid, pos, allow_unknown)?;
        Ok((out, ret))
    }

    fn construct(&mut self, cid: ClassId, args: &[Arg], pos: Pos) -> Res<TExpr> {
        let ty = Ty::Class(cid);
        let nfields = self.classes[cid].fields.len();
        if let Some(ctor) = self.class_ctor[cid] {
            let obj_dummy = texpr(TK::Null, ty.clone());
            let (mut call_args, ret) = self.match_args(ctor, Some(obj_dummy), args, pos)?;
            if ret != Ty::None {
                return err(pos, format!("`құру` әдісі мән қайтармауы керек (класс `{}`)", self.classes[cid].name));
            }
            call_args.remove(0);
            let fields = self.class_defaults[cid].clone();
            return Ok(texpr(TK::New { class: cid, fields, ctor: Some((ctor, call_args)) }, ty));
        }
        // автоматты конструктор: өрістер реті бойынша
        let mut slots: Vec<Option<TExpr>> = vec![None; nfields];
        let mut next = 0;
        let mut seen_kw = false;
        for a in args {
            let i = match &a.name {
                None => {
                    if seen_kw {
                        return err(a.value.pos, "орындық аргумент атаулы аргументтен кейін келе алмайды");
                    }
                    if next >= nfields {
                        return err(a.value.pos, format!("`{}` класында {} өріс бар, көбірек аргумент берілді", self.classes[cid].name, nfields));
                    }
                    next += 1;
                    next - 1
                }
                Some(n) => {
                    seen_kw = true;
                    match self.classes[cid].fields.iter().position(|f| &f.name == n) {
                        Some(i) => i,
                        None => return err(a.value.pos, format!("`{}` класында `{}` өрісі жоқ", self.classes[cid].name, n)),
                    }
                }
            };
            if slots[i].is_some() {
                return err(a.value.pos, "өріс екі рет берілді");
            }
            let fty = self.classes[cid].fields[i].ty.clone();
            let v = self.expr(&a.value, Some(&fty))?;
            self.need_value(&v, a.value.pos)?;
            slots[i] = Some(self.coerce(v, &fty, a.value.pos)?);
        }
        let mut fields = Vec::new();
        for (i, s) in slots.into_iter().enumerate() {
            match s {
                Some(v) => fields.push(Some(v)),
                None => match &self.class_defaults[cid][i] {
                    Some(d) => fields.push(Some(d.clone())),
                    None => {
                        // сілтеме өрістері (класс, көрсеткіш) өздігінен `жоқ` болады
                        let fty = self.classes[cid].fields[i].ty.clone();
                        if matches!(fty, Ty::Class(_) | Ty::Ptr(_)) {
                            fields.push(Some(texpr(TK::Null, fty)));
                            continue;
                        }
                        return err(
                            pos,
                            format!("`{}` құрылымына `{}` өрісінің мәні жетіспейді", self.classes[cid].name, self.classes[cid].fields[i].name),
                        );
                    }
                },
            }
        }
        Ok(texpr(TK::New { class: cid, fields, ctor: None }, ty))
    }

    fn arg_count(&self, name: &str, args: &[Arg], lo: usize, hi: usize, pos: Pos) -> Res<()> {
        if args.iter().any(|a| a.name.is_some()) {
            return err(pos, format!("`{}` атаулы аргументтерді қабылдамайды", name));
        }
        if args.len() < lo || args.len() > hi {
            let want = if lo == hi { format!("{}", lo) } else { format!("{}–{}", lo, hi) };
            return err(pos, format!("`{}` {} аргумент алады, бірақ {} берілді", name, want, args.len()));
        }
        Ok(())
    }

    fn arg(&mut self, a: &Arg, ty: Option<&Ty>) -> Res<TExpr> {
        let v = self.expr(&a.value, ty)?;
        self.need_value(&v, a.value.pos)?;
        match ty {
            Some(t) => self.coerce(v, t, a.value.pos),
            None => Ok(v),
        }
    }

    fn hw_builtin(&mut self, name: &str, sym: &'static str, kinds: &str, ret: bool, args: &[Arg], pos: Pos) -> Res<TExpr> {
        if !KERNEL_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            return err(pos, format!("`{}` тек ядро режимінде қолжетімді (`tulpar os build`)", name));
        }
        self.need_manual(pos)?;
        self.arg_count(name, args, kinds.len(), kinds.len(), pos)?;
        let mut v = Vec::new();
        for (a, k) in args.iter().zip(kinds.chars()) {
            let e = match k {
                'i' => self.arg(a, Some(&Ty::Int))?,
                'h' => self.arg(a, Some(&Ty::Func(vec![Ty::Int, Ty::Int], Box::new(Ty::None))))?,
                _ => {
                    let e = self.arg(a, None)?;
                    if !matches!(e.ty, Ty::Int | Ty::Ptr(_)) {
                        return err(a.value.pos, format!("`{}` мекенжайды (бүтін немесе көрсеткіш) күтеді, бірақ `{}` берілді", name, self.tn(&e.ty)));
                    }
                    e
                }
            };
            v.push(e);
        }
        Ok(texpr(TK::Builtin(Bi::Hw(sym, ret), v), if ret { Ty::Int } else { Ty::None }))
    }

    fn builtin_call(&mut self, name: &str, args: &[Arg], pos: Pos, _exp: Option<&Ty>) -> Res<TExpr> {
        if let Some(&(_, sym, kinds, ret)) = HW_BUILTINS.iter().find(|h| h.0 == name) {
            return self.hw_builtin(name, sym, kinds, ret, args, pos);
        }
        match name {
            "мекен" => {
                self.need_manual(pos)?;
                self.arg_count(name, args, 2, 2, pos)?;
                if !self.is_type_name(&args[0].value) {
                    return err(args[0].value.pos, "бірінші аргумент тип болуы керек: `мекен(байт, 0xB8000)`");
                }
                let t = self.expr_as_type(&args[0].value)?;
                if t.is_unit() {
                    return err(pos, "`жоқ` типіне көрсеткіш жасауға болмайды");
                }
                let a = self.arg(&args[1], Some(&Ty::Int))?;
                Ok(texpr(TK::Builtin(Bi::PtrCast, vec![a]), Ty::Ptr(Box::new(t))))
            }
            "мекенжай" => {
                self.need_manual(pos)?;
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                if !matches!(a.ty, Ty::Ptr(_)) {
                    return err(pos, "`мекенжай` көрсеткішті (`*T`) күтеді");
                }
                Ok(texpr(TK::Builtin(Bi::PtrCast, vec![a]), Ty::Int))
            }
            "жаз" | "көрсет" => {
                if args.iter().any(|a| a.name.is_some()) {
                    return err(pos, format!("`{}` атаулы аргументтерді қабылдамайды", name));
                }
                let mut v = Vec::new();
                for a in args {
                    let e = self.arg(a, None)?;
                    if e.ty == Ty::Null {
                        return err(a.value.pos, "`жоқ` мәнін басып шығаруға болмайды (типі белгісіз)");
                    }
                    v.push(e);
                }
                Ok(texpr(TK::Builtin(Bi::Print { nl: name == "жаз" }, v), Ty::None))
            }
            "оқы" => {
                self.arg_count(name, args, 0, 1, pos)?;
                let mut v = Vec::new();
                if let Some(a) = args.first() {
                    v.push(self.arg(a, Some(&Ty::Str))?);
                }
                Ok(texpr(TK::Builtin(Bi::Input, v), Ty::Str))
            }
            "ұзындық" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                if !matches!(a.ty, Ty::Str | Ty::List(_) | Ty::Dict(..) | Ty::Set(_)) {
                    return err(pos, format!("`ұзындық` `{}` типіне қолданылмайды", self.tn(&a.ty)));
                }
                Ok(texpr(TK::Builtin(Bi::Len, vec![a]), Ty::Int))
            }
            "бүтін" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                Ok(match a.ty {
                    Ty::Int => a,
                    Ty::Float => texpr(TK::Cast(CastKind::FloatToInt, Box::new(a)), Ty::Int),
                    Ty::Bool => texpr(TK::Cast(CastKind::BoolToInt, Box::new(a)), Ty::Int),
                    Ty::Byte => texpr(TK::Cast(CastKind::ByteToInt, Box::new(a)), Ty::Int),
                    Ty::Str => texpr(TK::Builtin(Bi::StrToInt, vec![a]), Ty::Int),
                    Ty::Ptr(_) => {
                        self.need_manual(pos)?;
                        texpr(TK::Cast(CastKind::Reinterpret, Box::new(a)), Ty::Int)
                    }
                    ref other => return err(pos, format!("`{}` типін `бүтін`-ге айналдыру мүмкін емес", self.tn(other))),
                })
            }
            "нақты" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                Ok(match a.ty {
                    Ty::Float => a,
                    Ty::Int | Ty::Byte => self.coerce(a, &Ty::Float, pos)?,
                    Ty::Str => texpr(TK::Builtin(Bi::StrToFloat, vec![a]), Ty::Float),
                    ref other => return err(pos, format!("`{}` типін `нақты`-ға айналдыру мүмкін емес", self.tn(other))),
                })
            }
            "мәтін" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                if a.ty == Ty::Null {
                    return err(pos, "`жоқ` мәнін мәтінге айналдыру мүмкін емес");
                }
                Ok(self.to_str(a))
            }
            "логика" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                self.truthy(a, pos)
            }
            "байт" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                match a.ty {
                    Ty::Byte => Ok(a),
                    Ty::Int => Ok(texpr(TK::Cast(CastKind::IntToByte, Box::new(a)), Ty::Byte)),
                    ref other => err(pos, format!("`{}` типін `байт`-қа айналдыру мүмкін емес", self.tn(other))),
                }
            }
            "абс" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                let a = if a.ty == Ty::Byte { self.to_int(a, pos)? } else { a };
                match a.ty {
                    Ty::Int => Ok(texpr(TK::Builtin(Bi::AbsI, vec![a]), Ty::Int)),
                    Ty::Float => Ok(texpr(TK::Builtin(Bi::AbsF, vec![a]), Ty::Float)),
                    ref other => err(pos, format!("`абс` `{}` типіне қолданылмайды", self.tn(other))),
                }
            }
            "мин" | "макс" if args.len() == 1 => {
                let l = self.arg(&args[0], None)?;
                match &l.ty {
                    Ty::List(t) if matches!(**t, Ty::Int | Ty::Float) => {
                        let rt = (**t).clone();
                        let bi = if name == "мин" { Bi::ListMin } else { Bi::ListMax };
                        Ok(texpr(TK::Builtin(bi, vec![l]), rt))
                    }
                    other => err(pos, format!("`{}` тізімі (`бүтін` немесе `нақты`) күтіледі, бірақ `{}` берілді", name, self.tn(other))),
                }
            }
            "қосынды" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let l = self.arg(&args[0], None)?;
                match &l.ty {
                    Ty::List(t) if matches!(**t, Ty::Int | Ty::Float) => {
                        let rt = (**t).clone();
                        Ok(texpr(TK::Builtin(Bi::ListSum, vec![l]), rt))
                    }
                    other => err(pos, format!("`қосынды` `тізім[бүтін]` немесе `тізім[нақты]` күтеді, бірақ `{}` берілді", self.tn(other))),
                }
            }
            "мин" | "макс" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let a = self.arg(&args[0], None)?;
                let b = self.arg(&args[1], None)?;
                let (a, b, t) = self.promote(a, b, pos)?;
                let bi = match (name, &t) {
                    ("мин", Ty::Int) => Bi::MinI,
                    ("макс", Ty::Int) => Bi::MaxI,
                    ("мин", _) => Bi::MinF,
                    _ => Bi::MaxF,
                };
                Ok(texpr(TK::Builtin(bi, vec![a, b]), t))
            }
            "түбір" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Float))?;
                Ok(texpr(TK::Builtin(Bi::Sqrt, vec![a]), Ty::Float))
            }
            "еден" | "төбе" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Float))?;
                let bi = if name == "еден" { Bi::Floor } else { Bi::Ceil };
                let r = texpr(TK::Builtin(bi, vec![a]), Ty::Float);
                Ok(texpr(TK::Cast(CastKind::FloatToInt, Box::new(r)), Ty::Int))
            }
            "жина" => {
                self.arg_count(name, args, 0, 0, pos)?;
                Ok(texpr(TK::Builtin(Bi::Gc, vec![]), Ty::None))
            }
            "жад" => {
                self.arg_count(name, args, 0, 0, pos)?;
                Ok(texpr(TK::Builtin(Bi::GcHeap, vec![]), Ty::Int))
            }
            "қате" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Str))?;
                Ok(texpr(TK::Builtin(Bi::Panic, vec![a]), Ty::None))
            }
            "бөл" => {
                self.need_manual(pos)?;
                self.arg_count(name, args, 2, 2, pos)?;
                if !self.is_type_name(&args[0].value) {
                    return err(args[0].value.pos, "бірінші аргумент тип болуы керек: `бөл(бүтін, 10)`");
                }
                let t = self.expr_as_type(&args[0].value)?;
                if t.is_unit() {
                    return err(pos, "`жоқ` типін бөлуге болмайды");
                }
                let n = self.arg(&args[1], Some(&Ty::Int))?;
                Ok(texpr(TK::Builtin(Bi::Alloc(t.clone()), vec![n]), Ty::Ptr(Box::new(t))))
            }
            "босат" => {
                self.need_manual(pos)?;
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], None)?;
                if !matches!(a.ty, Ty::Ptr(_)) {
                    return err(pos, "`босат` көрсеткішті (`*T`) күтеді");
                }
                Ok(texpr(TK::Builtin(Bi::Free, vec![a]), Ty::None))
            }
            "аргтар" => {
                self.arg_count(name, args, 0, 0, pos)?;
                Ok(texpr(TK::Builtin(Bi::Args, vec![]), Ty::List(Box::new(Ty::Str))))
            }
            "файл_оқы" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let p = self.arg(&args[0], Some(&Ty::Str))?;
                Ok(texpr(TK::Builtin(Bi::FileRead, vec![p]), Ty::Str))
            }
            "файл_жаз" | "файл_қос" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let p = self.arg(&args[0], Some(&Ty::Str))?;
                let t = self.arg(&args[1], Some(&Ty::Str))?;
                Ok(texpr(TK::Builtin(Bi::FileWrite(name == "файл_қос"), vec![p, t]), Ty::None))
            }
            "файл_бар" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let p = self.arg(&args[0], Some(&Ty::Str))?;
                Ok(texpr(TK::Builtin(Bi::FileExists, vec![p]), Ty::Bool))
            }
            "файл_жой" | "қалта_жаса" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let p = self.arg(&args[0], Some(&Ty::Str))?;
                let bi = if name == "файл_жой" { Bi::FileDelete } else { Bi::MakeDir };
                Ok(texpr(TK::Builtin(bi, vec![p]), Ty::None))
            }
            "файл_ауыстыр" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Str))?;
                let b = self.arg(&args[1], Some(&Ty::Str))?;
                Ok(texpr(TK::Builtin(Bi::FileRename, vec![a, b]), Ty::None))
            }
            "желі_күту" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let fd = self.arg(&args[0], Some(&Ty::Int))?;
                let ms = self.arg(&args[1], Some(&Ty::Int))?;
                Ok(texpr(TK::Builtin(Bi::NetTimeout, vec![fd, ms]), Ty::None))
            }
            "уақыт" => {
                self.arg_count(name, args, 0, 0, pos)?;
                Ok(texpr(TK::Builtin(Bi::Time, vec![]), Ty::Float))
            }
            "кездейсоқ" => {
                self.arg_count(name, args, 0, 0, pos)?;
                Ok(texpr(TK::Builtin(Bi::Rand, vec![]), Ty::Float))
            }
            "кездейсоқ_аралық" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Int))?;
                let b = self.arg(&args[1], Some(&Ty::Int))?;
                Ok(texpr(TK::Builtin(Bi::RandRange, vec![a, b]), Ty::Int))
            }
            "тұқым" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Int))?;
                Ok(texpr(TK::Builtin(Bi::Seed, vec![a]), Ty::None))
            }
            "шығу" => {
                self.arg_count(name, args, 0, 1, pos)?;
                let a = match args.first() {
                    Some(a) => self.arg(a, Some(&Ty::Int))?,
                    None => texpr(TK::Int(0), Ty::Int),
                };
                Ok(texpr(TK::Builtin(Bi::Exit, vec![a]), Ty::None))
            }
            "синус" | "косинус" | "тангенс" | "арктангенс" | "экспонента" | "логарифм" | "логарифм10" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Float))?;
                let k = match name {
                    "синус" => 0,
                    "косинус" => 1,
                    "тангенс" => 2,
                    "арктангенс" => 3,
                    "экспонента" => 4,
                    "логарифм" => 5,
                    _ => 6,
                };
                Ok(texpr(TK::Builtin(Bi::Math(k), vec![a]), Ty::Float))
            }
            "арктангенс2" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let y = self.arg(&args[0], Some(&Ty::Float))?;
                let x = self.arg(&args[1], Some(&Ty::Float))?;
                Ok(texpr(TK::Builtin(Bi::Math(7), vec![y, x]), Ty::Float))
            }
            "тыңда" | "тыңда_локал" | "қабылда" | "желі_оқы" | "желі_жап" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Int))?;
                let (bi, ty) = match name {
                    "тыңда" => (Bi::NetListen, Ty::Int),
                    "тыңда_локал" => (Bi::NetListenLocal, Ty::Int),
                    "қабылда" => (Bi::NetAccept, Ty::Int),
                    "желі_оқы" => (Bi::NetRead, Ty::Str),
                    _ => (Bi::NetClose, Ty::None),
                };
                Ok(texpr(TK::Builtin(bi, vec![a]), ty))
            }
            "қосыл" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let h = self.arg(&args[0], Some(&Ty::Str))?;
                let p = self.arg(&args[1], Some(&Ty::Int))?;
                Ok(texpr(TK::Builtin(Bi::NetConnect, vec![h, p]), Ty::Int))
            }
            "желі_жаз" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let fd = self.arg(&args[0], Some(&Ty::Int))?;
                let t = self.arg(&args[1], Some(&Ty::Str))?;
                Ok(texpr(TK::Builtin(Bi::NetWrite, vec![fd, t]), Ty::None))
            }
            "жиын" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let l = self.arg(&args[0], None)?;
                match &l.ty {
                    Ty::List(t) if matches!(**t, Ty::Int | Ty::Str | Ty::Bool | Ty::Byte) => {
                        let st = Ty::Set(t.clone());
                        Ok(texpr(TK::Builtin(Bi::SetFromList, vec![l]), st))
                    }
                    other => err(pos, format!("`жиын(...)` тізімнен жасалады (`тізім[бүтін|мәтін|логика|байт]`), бірақ `{}` берілді", self.tn(other))),
                }
            }
            "код" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Str))?;
                Ok(texpr(TK::Builtin(Bi::Ord, vec![a]), Ty::Int))
            }
            "таңба" => {
                self.arg_count(name, args, 1, 1, pos)?;
                let a = self.arg(&args[0], Some(&Ty::Int))?;
                Ok(texpr(TK::Builtin(Bi::Chr, vec![a]), Ty::Str))
            }
            "мысал" | "түрлендір" => {
                self.arg_count(name, args, 2, 2, pos)?;
                let x = self.arg(&args[0], None)?;
                if !matches!(x.ty, Ty::Class(_)) {
                    return err(args[0].value.pos, "бірінші аргумент класс объектісі болуы керек");
                }
                let cid = match &args[1].value.kind {
                    ExprKind::Name(n) if self.class_names.contains_key(n) => self.class_names[n],
                    _ => return err(args[1].value.pos, "екінші аргумент класс аты болуы керек"),
                };
                if name == "мысал" {
                    Ok(texpr(TK::Builtin(Bi::IsInstance(cid), vec![x]), Ty::Bool))
                } else {
                    Ok(texpr(TK::Builtin(Bi::Downcast(cid), vec![x]), Ty::Class(cid)))
                }
            }
            "нөмірле" => err(pos, "`нөмірле` тек `әр і, x ішінде нөмірле(тізім)` циклында қолданылады"),
            "аралық" => err(pos, "`аралық` тек `әр ... ішінде аралық(...)` циклында қолданылады"),
            "тізім" | "сөздік" => err(pos, format!("`{}[...]()` деп жазыңыз, мысалы `тізім[бүтін]()`", name)),
            other => err(pos, format!("анықталмаған функция `{}`", other)),
        }
    }

    fn method_call(&mut self, recv: TExpr, m: &str, args: &[Arg], pos: Pos) -> Res<TExpr> {
        match recv.ty.clone() {
            Ty::Class(cid) => {
                let fid = match self.class_methods[cid].get(m) {
                    Some(f) => *f,
                    None => {
                        // өріс ретіндегі функция мәні: объект.өріс(аргументтер)
                        if let Some(i) = self.classes[cid].fields.iter().position(|f| f.name == m && matches!(f.ty, Ty::Func(..))) {
                            let fty = self.classes[cid].fields[i].ty.clone();
                            let callee = texpr(TK::Field(Box::new(recv), i), fty);
                            return self.call_value(callee, args, pos);
                        }
                        return err(pos, format!("`{}` класында `{}` әдісі жоқ", self.classes[cid].name, m));
                    }
                };
                let virt = self.is_overridden(cid, m, fid);
                let (call_args, ret) = self.match_args(fid, Some(recv), args, pos)?;
                if virt {
                    if !self.used_virtual.iter().any(|n| n == m) {
                        self.used_virtual.push(m.to_string());
                    }
                    return Ok(texpr(TK::VCall(m.to_string(), call_args), ret));
                }
                Ok(texpr(TK::Call(fid, call_args), ret))
            }
            Ty::List(et) => {
                let et = *et;
                match m {
                    "қос" => {
                        self.arg_count(m, args, 1, 1, pos)?;
                        let x = self.arg(&args[0], Some(&et))?;
                        Ok(texpr(TK::Builtin(Bi::ListPush, vec![recv, x]), Ty::None))
                    }
                    "ал" => {
                        self.arg_count(m, args, 0, 0, pos)?;
                        Ok(texpr(TK::Builtin(Bi::ListPop, vec![recv]), et))
                    }
                    "тазала" => {
                        self.arg_count(m, args, 0, 0, pos)?;
                        Ok(texpr(TK::Builtin(Bi::ListClear, vec![recv]), Ty::None))
                    }
                    "кері" => {
                        self.arg_count(m, args, 0, 0, pos)?;
                        Ok(texpr(TK::Builtin(Bi::ListReverse, vec![recv]), Ty::None))
                    }
                    "индекс" => {
                        self.arg_count(m, args, 1, 1, pos)?;
                        let x = self.arg(&args[0], Some(&et))?;
                        Ok(texpr(TK::Builtin(Bi::ListIndex, vec![recv, x]), Ty::Int))
                    }
                    "өшір" => {
                        self.arg_count(m, args, 1, 1, pos)?;
                        let i = self.arg(&args[0], Some(&Ty::Int))?;
                        Ok(texpr(TK::Builtin(Bi::ListRemove, vec![recv, i]), et))
                    }
                    "кірістір" => {
                        self.arg_count(m, args, 2, 2, pos)?;
                        let i = self.arg(&args[0], Some(&Ty::Int))?;
                        let x = self.arg(&args[1], Some(&et))?;
                        Ok(texpr(TK::Builtin(Bi::ListInsert, vec![recv, i, x]), Ty::None))
                    }
                    "көшірме" => {
                        self.arg_count(m, args, 0, 0, pos)?;
                        let t = recv.ty.clone();
                        Ok(texpr(TK::Builtin(Bi::ListCopy, vec![recv]), t))
                    }
                    "сұрыпта" if args.len() == 1 => {
                        let exp = Ty::Func(vec![et.clone()], Box::new(Ty::Null));
                        let kf = self.expr(&args[0].value, Some(&exp))?;
                        let kt = match &kf.ty {
                            Ty::Func(ps, r) if ps.len() == 1 && ps[0] == et => (**r).clone(),
                            other => return err(args[0].value.pos, format!("кілт функциясы `({}) -> ...` типті болуы керек, бірақ `{}` берілді", self.tn(&et), self.tn(other))),
                        };
                        let kind = match kt {
                            Ty::Int | Ty::Bool | Ty::Byte => 0u8,
                            Ty::Float => 1,
                            Ty::Str => 2,
                            ref other => return err(args[0].value.pos, format!("кілт `{}` типті бола алмайды (бүтін, нақты немесе мәтін керек)", self.tn(other))),
                        };
                        let mut pre = Vec::new();
                        let lv = self.stash(recv, &mut pre);
                        let kfv = self.stash(kf, &mut pre);
                        let kl_ty = Ty::List(Box::new(kt.clone()));
                        let keys = self.temp(kl_ty.clone());
                        pre.push(TStmt::SetLocal(keys, texpr(TK::ListLit(vec![]), kl_ty.clone())));
                        let x = self.temp(et.clone());
                        let keyv = texpr(TK::Local(keys), kl_ty);
                        let call = texpr(TK::CallValue(Box::new(kfv), vec![texpr(TK::Local(x), et.clone())]), kt);
                        let push = TStmt::Expr(texpr(TK::Builtin(Bi::ListPush, vec![keyv.clone(), call]), Ty::None));
                        pre.push(TStmt::ForList { item: Target::Local(x), index: None, list: lv.clone(), body: vec![push] });
                        let sort = texpr(TK::Builtin(Bi::ListSortBy(kind), vec![lv, keyv]), Ty::None);
                        Ok(texpr(TK::Block(pre, Box::new(sort)), Ty::None))
                    }
                    "сұрыпта" => {
                        self.arg_count(m, args, 0, 0, pos)?;
                        if !matches!(et, Ty::Int | Ty::Float | Ty::Str | Ty::Byte) {
                            return err(pos, format!("`{}` элементтерін сұрыптауға болмайды", self.tn(&et)));
                        }
                        Ok(texpr(TK::Builtin(Bi::ListSort, vec![recv]), Ty::None))
                    }
                    _ => err(pos, format!("тізімде `{}` әдісі жоқ", m)),
                }
            }
            Ty::Str => match m {
                "үлкен" | "кіші" | "тазарт" => {
                    self.arg_count(m, args, 0, 0, pos)?;
                    let bi = match m {
                        "үлкен" => Bi::StrUpper,
                        "кіші" => Bi::StrLower,
                        _ => Bi::StrStrip,
                    };
                    Ok(texpr(TK::Builtin(bi, vec![recv]), Ty::Str))
                }
                "байт_саны" => {
                    self.arg_count(m, args, 0, 0, pos)?;
                    Ok(texpr(TK::Builtin(Bi::StrBytes, vec![recv]), Ty::Int))
                }
                "сан_ма" | "әріп_ма" => {
                    self.arg_count(m, args, 0, 0, pos)?;
                    let bi = if m == "сан_ма" { Bi::StrIsDigit } else { Bi::StrIsAlpha };
                    Ok(texpr(TK::Builtin(bi, vec![recv]), Ty::Bool))
                }
                "саны" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let s = self.arg(&args[0], Some(&Ty::Str))?;
                    Ok(texpr(TK::Builtin(Bi::StrCount, vec![recv, s]), Ty::Int))
                }
                "бөлу" if args.is_empty() => {
                    Ok(texpr(TK::Builtin(Bi::StrSplitWs, vec![recv]), Ty::List(Box::new(Ty::Str))))
                }
                "бөлу" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let s = self.arg(&args[0], Some(&Ty::Str))?;
                    Ok(texpr(TK::Builtin(Bi::StrSplit, vec![recv, s]), Ty::List(Box::new(Ty::Str))))
                }
                "біріктір" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let l = self.arg(&args[0], Some(&Ty::List(Box::new(Ty::Str))))?;
                    Ok(texpr(TK::Builtin(Bi::StrJoin, vec![recv, l]), Ty::Str))
                }
                "тап" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let s = self.arg(&args[0], Some(&Ty::Str))?;
                    Ok(texpr(TK::Builtin(Bi::StrFind, vec![recv, s]), Ty::Int))
                }
                "ауыстыр" => {
                    self.arg_count(m, args, 2, 2, pos)?;
                    let a = self.arg(&args[0], Some(&Ty::Str))?;
                    let b = self.arg(&args[1], Some(&Ty::Str))?;
                    Ok(texpr(TK::Builtin(Bi::StrReplace, vec![recv, a, b]), Ty::Str))
                }
                "байт" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let i = self.arg(&args[0], Some(&Ty::Int))?;
                    Ok(texpr(TK::Builtin(Bi::StrByte, vec![recv, i]), Ty::Byte))
                }
                "басталады" | "аяқталады" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let s = self.arg(&args[0], Some(&Ty::Str))?;
                    let bi = if m == "басталады" { Bi::StrStarts } else { Bi::StrEnds };
                    Ok(texpr(TK::Builtin(bi, vec![recv, s]), Ty::Bool))
                }
                _ => err(pos, format!("мәтінде `{}` әдісі жоқ", m)),
            },
            Ty::Set(et) => match m {
                "қос" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let x = self.arg(&args[0], Some(&et))?;
                    Ok(texpr(TK::Builtin(Bi::SetAdd, vec![recv, x]), Ty::None))
                }
                "жой" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let x = self.arg(&args[0], Some(&et))?;
                    Ok(texpr(TK::Builtin(Bi::DictDel, vec![recv, x]), Ty::None))
                }
                "тізім" => {
                    self.arg_count(m, args, 0, 0, pos)?;
                    Ok(texpr(TK::Builtin(Bi::DictKeys, vec![recv]), Ty::List(et)))
                }
                _ => err(pos, format!("жиында `{}` әдісі жоқ", m)),
            },
            Ty::Dict(kt, vt) => match m {
                "жой" => {
                    self.arg_count(m, args, 1, 1, pos)?;
                    let k = self.arg(&args[0], Some(&kt))?;
                    Ok(texpr(TK::Builtin(Bi::DictDel, vec![recv, k]), Ty::None))
                }
                "ал" => {
                    self.arg_count(m, args, 2, 2, pos)?;
                    let k = self.arg(&args[0], Some(&kt))?;
                    let d = self.arg(&args[1], Some(&vt))?;
                    Ok(texpr(TK::Builtin(Bi::DictGetOr, vec![recv, k, d]), *vt))
                }
                "кілттер" => {
                    self.arg_count(m, args, 0, 0, pos)?;
                    Ok(texpr(TK::Builtin(Bi::DictKeys, vec![recv]), Ty::List(kt)))
                }
                "мәндер" => {
                    self.arg_count(m, args, 0, 0, pos)?;
                    Ok(texpr(TK::Builtin(Bi::DictValues, vec![recv]), Ty::List(vt)))
                }
                _ => err(pos, format!("сөздікте `{}` әдісі жоқ", m)),
            },
            other => err(pos, format!("`{}` типінде `{}` әдісі жоқ", self.tn(&other), m)),
        }
    }

    // ---------------------------------------------------------------- операторлар

    fn block(&mut self, v: &[Stmt]) -> Res<Vec<TStmt>> {
        let mut out = Vec::new();
        for s in v {
            self.stmt(s, &mut out)?;
        }
        Ok(out)
    }

    fn zero_value(&self, t: &Ty) -> TExpr {
        match t {
            Ty::Int => texpr(TK::Int(0), Ty::Int),
            Ty::Byte => texpr(TK::Int(0), Ty::Byte),
            Ty::Float => texpr(TK::Float(0.0), Ty::Float),
            Ty::Bool => texpr(TK::Bool(false), Ty::Bool),
            other => texpr(TK::Null, other.clone()),
        }
    }

    /// Айнымалыны (бар болса — типін тексеріп) байлайды.
    fn bind(&mut self, name: &str, v: TExpr, pos: Pos) -> Res<TStmt> {
        match self.find_assignable(name) {
            Some((t, ty)) => {
                let v = self.coerce(v, &ty, pos)?;
                Ok(self.set_var(t, v))
            }
            None => {
                self.need_value(&v, pos)?;
                if v.ty == Ty::Null {
                    return err(pos, format!("`{}` айнымалысының типін анықтау мүмкін емес, тип көрсетіңіз: `{}: тип = жоқ`", name, name));
                }
                let t = self.create_var(name, v.ty.clone(), pos)?;
                Ok(self.set_var(t, v))
            }
        }
    }

    fn target_ty(&mut self, t: &Expr) -> Res<Option<Ty>> {
        match &t.kind {
            ExprKind::Name(n) => Ok(self.find_assignable(n).map(|x| x.1)),
            ExprKind::Field(..) | ExprKind::Index(..) | ExprKind::Unary(UnOp::Deref, _) => Ok(Some(self.expr(t, None)?.ty)),
            _ => Ok(None),
        }
    }

    fn store(&mut self, target: &Expr, v: TExpr, out: &mut Vec<TStmt>) -> Res<()> {
        let pos = target.pos;
        match &target.kind {
            ExprKind::Name(n) => {
                let s = self.bind(n, v, pos)?;
                out.push(s);
                Ok(())
            }
            ExprKind::Index(base, idx) => {
                let b = self.expr(base, None)?;
                match b.ty.clone() {
                    Ty::List(t) | Ty::Ptr(t) => {
                        if matches!(b.ty, Ty::Ptr(_)) {
                            self.need_manual(pos)?;
                        }
                        let i = self.expr(idx, None)?;
                        let i = self.to_int(i, idx.pos)?;
                        let v = self.coerce(v, &t, pos)?;
                        out.push(TStmt::SetIndex(b, i, v));
                    }
                    Ty::Dict(k, vt) => {
                        let i = self.expr(idx, Some(&k))?;
                        let i = self.coerce(i, &k, idx.pos)?;
                        let v = self.coerce(v, &vt, pos)?;
                        out.push(TStmt::SetIndex(b, i, v));
                    }
                    Ty::Str => return err(pos, "мәтін өзгермейді: символын өзгертуге болмайды"),
                    other => return err(pos, format!("`{}` типіне индекс бойынша жазуға болмайды", self.tn(&other))),
                }
                Ok(())
            }
            ExprKind::Field(obj, _) => {
                let o = self.expr(obj, None)?;
                let fe = self.expr(target, None)?;
                let idx = match fe.kind {
                    TK::Field(_, i) => i,
                    _ => unreachable!(),
                };
                let v = self.coerce(v, &fe.ty, pos)?;
                out.push(TStmt::SetField(o, idx, v));
                Ok(())
            }
            ExprKind::Unary(UnOp::Deref, p) => {
                self.need_manual(pos)?;
                let pe = self.expr(p, None)?;
                match pe.ty.clone() {
                    Ty::Ptr(t) => {
                        let v = self.coerce(v, &t, pos)?;
                        out.push(TStmt::SetDeref(pe, v));
                        Ok(())
                    }
                    _ => err(pos, "`*` тек көрсеткішке қолданылады"),
                }
            }
            _ => err(pos, "меншіктеу мақсаты жарамсыз"),
        }
    }

    /// Күрделі өрнекті уақытша айнымалыға салады (қайталап есептелмеу үшін).
    fn stash(&mut self, e: TExpr, out: &mut Vec<TStmt>) -> TExpr {
        match e.kind {
            TK::Local(_) | TK::Global(_) | TK::Int(_) | TK::Str(_) | TK::Bool(_) | TK::Float(_) => e,
            _ => {
                let ty = e.ty.clone();
                let t = self.temp(ty.clone());
                out.push(TStmt::SetLocal(t, e));
                texpr(TK::Local(t), ty)
            }
        }
    }

    fn stmt(&mut self, s: &Stmt, out: &mut Vec<TStmt>) -> Res<()> {
        if !matches!(s.kind, StmtKind::Pass | StmtKind::Seq(_) | StmtKind::Import(_) | StmtKind::Global(_)) {
            out.push(TStmt::Line(s.pos.file, s.pos.line));
        }
        self.stmt_inner(s, out)
    }

    fn stmt_inner(&mut self, s: &Stmt, out: &mut Vec<TStmt>) -> Res<()> {
        let pos = s.pos;
        match &s.kind {
            StmtKind::Pass | StmtKind::Import(_) => Ok(()),
            StmtKind::Seq(v) => {
                for st in v {
                    self.stmt(st, out)?;
                }
                Ok(())
            }
            StmtKind::Expr(e) => {
                self.cx.stmt_call = matches!(e.kind, ExprKind::Call(..));
                let v = self.expr(e, None);
                self.cx.stmt_call = false;
                out.push(TStmt::Expr(v?));
                Ok(())
            }
            StmtKind::Assign { target, value } => {
                let exp = self.target_ty(target)?;
                let v = self.expr(value, exp.as_ref())?;
                self.need_value(&v, value.pos)?;
                self.store(target, v, out)
            }
            StmtKind::AnnAssign { target, ty, value } => {
                let t = self.resolve_type(ty)?;
                if t.is_unit() {
                    return err(ty.pos(), "айнымалы `жоқ` типті бола алмайды");
                }
                let name = match &target.kind {
                    ExprKind::Name(n) => n.clone(),
                    _ => return err(pos, "тип аннотациясы тек айнымалы атына қойылады"),
                };
                if let Some((_, existing)) = self.find_assignable(&name) {
                    if existing != t {
                        return err(pos, format!("`{}` айнымалысы бұрын `{}` типімен анықталған", name, self.tn(&existing)));
                    }
                } else {
                    self.create_var(&name, t.clone(), pos)?;
                }
                let v = match value {
                    Some(e) => {
                        let v = self.expr(e, Some(&t))?;
                        self.need_value(&v, e.pos)?;
                        self.coerce(v, &t, e.pos)?
                    }
                    None => self.zero_value(&t),
                };
                let s = self.bind(&name, v, pos)?;
                out.push(s);
                Ok(())
            }
            StmtKind::TupleAssign { targets, values } if values.len() == 1 => {
                let v = self.expr(&values[0], None)?;
                let ts = match &v.ty {
                    Ty::Tuple(ts) if ts.len() == targets.len() => ts.clone(),
                    Ty::Tuple(ts) => return err(pos, format!("кортежде {} элемент бар, ал {} айнымалы берілген", ts.len(), targets.len())),
                    other => return err(pos, format!("`{}` типін ашуға болмайды (кортеж күтілді)", self.tn(other))),
                };
                let tmp = self.temp(v.ty.clone());
                out.push(TStmt::SetLocal(tmp, v.clone()));
                let tv = texpr(TK::Local(tmp), v.ty.clone());
                for (i, t) in targets.iter().enumerate() {
                    let el = texpr(TK::TupleGet(Box::new(tv.clone()), i), ts[i].clone());
                    self.store(t, el, out)?;
                }
                Ok(())
            }
            StmtKind::TupleAssign { targets, values } => {
                let mut tmps = Vec::new();
                for (t, v) in targets.iter().zip(values) {
                    let exp = self.target_ty(t)?;
                    let ve = self.expr(v, exp.as_ref())?;
                    self.need_value(&ve, v.pos)?;
                    let ve = match &exp {
                        Some(ty) => self.coerce(ve, ty, v.pos)?,
                        None => ve,
                    };
                    let ty = ve.ty.clone();
                    let tmp = self.temp(ty.clone());
                    out.push(TStmt::SetLocal(tmp, ve));
                    tmps.push(texpr(TK::Local(tmp), ty));
                }
                for (t, v) in targets.iter().zip(tmps) {
                    self.store(t, v, out)?;
                }
                Ok(())
            }
            StmtKind::AugAssign { target, op, value } => {
                let rhs = self.expr(value, None)?;
                match &target.kind {
                    ExprKind::Name(_) => {
                        let cur = self.expr(target, None)?;
                        let nv = self.binary(*op, cur, rhs, pos)?;
                        self.store(target, nv, out)
                    }
                    ExprKind::Index(base, idx) => {
                        let b = self.expr(base, None)?;
                        let b = self.stash(b, out);
                        let ity = if let Ty::Dict(k, _) = &b.ty { Some((**k).clone()) } else { None };
                        let i = self.expr(idx, ity.as_ref())?;
                        let i = match &ity {
                            Some(k) => self.coerce(i, k, idx.pos)?,
                            None => self.to_int(i, idx.pos)?,
                        };
                        let i = self.stash(i, out);
                        let (elem, is_ptr) = match &b.ty {
                            Ty::List(t) => ((**t).clone(), false),
                            Ty::Ptr(t) => ((**t).clone(), true),
                            Ty::Dict(_, v) => ((**v).clone(), false),
                            other => return err(pos, format!("`{}` типінде индекс бойынша өзгерту мүмкін емес", self.tn(other))),
                        };
                        if is_ptr {
                            self.need_manual(pos)?;
                        }
                        let cur = texpr(TK::Index(Box::new(b.clone()), Box::new(i.clone())), elem.clone());
                        let nv = self.binary(*op, cur, rhs, pos)?;
                        let nv = self.coerce(nv, &elem, pos)?;
                        out.push(TStmt::SetIndex(b, i, nv));
                        Ok(())
                    }
                    ExprKind::Field(obj, _) => {
                        let o = self.expr(obj, None)?;
                        let o = self.stash(o, out);
                        let (idx, fty) = match &o.ty {
                            Ty::Class(c) => {
                                let name = if let ExprKind::Field(_, n) = &target.kind { n } else { unreachable!() };
                                match self.classes[*c].fields.iter().position(|f| &f.name == name) {
                                    Some(i) => (i, self.classes[*c].fields[i].ty.clone()),
                                    None => return err(pos, format!("`{}` өрісі жоқ", name)),
                                }
                            }
                            other => return err(pos, format!("`{}` типінде өріс жоқ", self.tn(other))),
                        };
                        let cur = texpr(TK::Field(Box::new(o.clone()), idx), fty.clone());
                        let nv = self.binary(*op, cur, rhs, pos)?;
                        let nv = self.coerce(nv, &fty, pos)?;
                        out.push(TStmt::SetField(o, idx, nv));
                        Ok(())
                    }
                    ExprKind::Unary(UnOp::Deref, p) => {
                        self.need_manual(pos)?;
                        let pe = self.expr(p, None)?;
                        let pe = self.stash(pe, out);
                        let t = match &pe.ty {
                            Ty::Ptr(t) => (**t).clone(),
                            _ => return err(pos, "`*` тек көрсеткішке қолданылады"),
                        };
                        let cur = texpr(TK::Deref(Box::new(pe.clone())), t.clone());
                        let nv = self.binary(*op, cur, rhs, pos)?;
                        let nv = self.coerce(nv, &t, pos)?;
                        out.push(TStmt::SetDeref(pe, nv));
                        Ok(())
                    }
                    _ => err(pos, "меншіктеу мақсаты жарамсыз"),
                }
            }
            StmtKind::If { cond, then, els } => {
                let c = self.expr(cond, None)?;
                let c = self.truthy(c, cond.pos)?;
                let t = self.block(then)?;
                let e = self.block(els)?;
                out.push(TStmt::If { cond: c, then: t, els: e });
                Ok(())
            }
            StmtKind::While { cond, body } => {
                let c = self.expr(cond, None)?;
                let c = self.truthy(c, cond.pos)?;
                self.cx.loops += 1;
                let b = self.block(body)?;
                self.cx.loops -= 1;
                out.push(TStmt::While { cond: c, body: b });
                Ok(())
            }
            StmtKind::For { targets, iter, body } => self.for_stmt(targets, iter, body, pos, out),
            StmtKind::Break | StmtKind::Continue => {
                if self.cx.loops == 0 {
                    return err(pos, "`тоқта`/`жалғастыр` тек циклдың ішінде қолданылады");
                }
                out.push(if matches!(s.kind, StmtKind::Break) { TStmt::Break } else { TStmt::Continue });
                Ok(())
            }
            StmtKind::Return(e) => {
                if self.cx.is_main {
                    return err(pos, "`қайтар` функцияның ішінде ғана қолданылады");
                }
                match e {
                    None => {
                        match &self.cx.ret {
                            Some(t) if *t != Ty::None => {
                                return err(pos, format!("функция `{}` мәнін қайтаруы керек", self.tn(t)));
                            }
                            None => self.cx.ret = Some(Ty::None),
                            _ => {}
                        }
                        out.push(TStmt::Return(None));
                    }
                    Some(e) => {
                        let exp = self.cx.ret.clone();
                        let v = self.expr(e, exp.as_ref())?;
                        self.need_value(&v, e.pos)?;
                        let v = match exp {
                            Some(t) => {
                                if t == Ty::None {
                                    return err(e.pos, "бұл функция мән қайтармауы керек");
                                }
                                self.coerce(v, &t, e.pos)?
                            }
                            None => {
                                if v.ty == Ty::Null {
                                    return err(e.pos, "қайтару типін анықтау мүмкін емес, `-> тип` көрсетіңіз");
                                }
                                self.cx.ret = Some(v.ty.clone());
                                v
                            }
                        };
                        out.push(TStmt::Return(Some(v)));
                    }
                }
                Ok(())
            }
            StmtKind::Global(names) => {
                if self.cx.is_main {
                    return Ok(());
                }
                for n in names {
                    if !self.global_ids.contains_key(n) {
                        return err(pos, format!("`{}` глобал айнымалысы әлі анықталмаған", n));
                    }
                    self.cx.declared_global.insert(n.clone());
                }
                Ok(())
            }
            StmtKind::Manual(body) => {
                self.cx.manual += 1;
                let b = self.block(body);
                self.cx.manual -= 1;
                out.extend(b?);
                Ok(())
            }
            StmtKind::Assert(c, m) => {
                let ce = self.expr(c, None)?;
                let ce = self.truthy(ce, c.pos)?;
                let msg = match m {
                    Some(m) => {
                        let v = self.expr(m, Some(&Ty::Str))?;
                        self.coerce(v, &Ty::Str, m.pos)?
                    }
                    None => texpr(TK::Str(format!("тексеру сәтсіз ({}-жол)", pos.line)), Ty::Str),
                };
                let not = texpr(TK::Unary(UnOp::Not, Box::new(ce)), Ty::Bool);
                let panic = TStmt::Expr(texpr(TK::Builtin(Bi::Panic, vec![msg]), Ty::None));
                out.push(TStmt::If { cond: not, then: vec![panic], els: vec![] });
                Ok(())
            }
            StmtKind::FuncDef(_) | StmtKind::ClassDef(_) => {
                err(pos, "функция мен класс тек файлдың жоғарғы деңгейінде анықталады")
            }
        }
    }

    fn loop_target(&mut self, name: &str, ty: Ty, pos: Pos) -> Res<Target> {
        if self.cx.comp > 0 {
            self.check_not_reserved(name, pos)?;
            let id = self.new_local(name, ty);
            self.cx.names.insert(name.to_string(), id);
            return Ok(Target::Local(id));
        }
        match self.find_assignable(name) {
            Some((t, existing)) => {
                if existing != ty {
                    return self.mismatch(pos, &existing, &ty);
                }
                Ok(t)
            }
            None => self.create_var(name, ty, pos),
        }
    }

    fn for_parts(&mut self, targets: &[(String, Pos)], iter: &Expr, pos: Pos) -> Res<ForParts> {
        // аралық(...)
        if let ExprKind::Call(f, args) = &iter.kind {
            if matches!(&f.kind, ExprKind::Name(n) if n == "аралық") {
                if targets.len() != 1 {
                    return err(pos, "`аралық` циклында бір ғана айнымалы болады");
                }
                if args.is_empty() || args.len() > 3 || args.iter().any(|a| a.name.is_some()) {
                    return err(iter.pos, "`аралық` 1–3 аргумент алады: `аралық(соңы)`, `аралық(басы, соңы)`, `аралық(басы, соңы, қадам)`");
                }
                let mut vals = Vec::new();
                for a in args {
                    let v = self.expr(&a.value, Some(&Ty::Int))?;
                    vals.push(self.to_int(v, a.value.pos)?);
                }
                let (start, stop, step) = match vals.len() {
                    1 => (texpr(TK::Int(0), Ty::Int), vals.remove(0), texpr(TK::Int(1), Ty::Int)),
                    2 => {
                        let b = vals.remove(1);
                        (vals.remove(0), b, texpr(TK::Int(1), Ty::Int))
                    }
                    _ => {
                        if let TK::Int(0) = vals[2].kind {
                            return err(args[2].value.pos, "қадам нөл болмауы керек");
                        }
                        let step = vals.remove(2);
                        let b = vals.remove(1);
                        (vals.remove(0), b, step)
                    }
                };
                let var = self.loop_target(&targets[0].0, Ty::Int, targets[0].1)?;
                return Ok(ForParts { kind: ForKind::Range { var, start, stop, step }, prelude: vec![] });
            }
            if matches!(&f.kind, ExprKind::Name(n) if n == "нөмірле") {
                if targets.len() != 2 || args.len() != 1 || args[0].name.is_some() {
                    return err(iter.pos, "`әр і, x ішінде нөмірле(тізім)` түрінде жазыңыз");
                }
                let l = self.expr(&args[0].value, None)?;
                let et = match &l.ty {
                    Ty::List(t) => (**t).clone(),
                    other => return err(args[0].value.pos, format!("`нөмірле` тізімді күтеді, бірақ `{}` берілді", self.tn(other))),
                };
                let ix = self.loop_target(&targets[0].0, Ty::Int, targets[0].1)?;
                let item = self.loop_target(&targets[1].0, et, targets[1].1)?;
                return Ok(ForParts { kind: ForKind::List { item, index: Some(ix), list: l }, prelude: vec![] });
            }
        }
        let it = self.expr(iter, None)?;
        match it.ty.clone() {
            Ty::List(et) => {
                if targets.len() == 1 {
                    let item = self.loop_target(&targets[0].0, *et, targets[0].1)?;
                    return Ok(ForParts { kind: ForKind::List { item, index: None, list: it }, prelude: vec![] });
                }
                match *et {
                    Ty::Tuple(ts) if ts.len() == targets.len() => {
                        let tt = Ty::Tuple(ts.clone());
                        let tmp = self.temp(tt.clone());
                        let tv = texpr(TK::Local(tmp), tt);
                        let mut prelude = Vec::new();
                        for (i, (n, p)) in targets.iter().enumerate() {
                            let t = self.loop_target(n, ts[i].clone(), *p)?;
                            let el = texpr(TK::TupleGet(Box::new(tv.clone()), i), ts[i].clone());
                            prelude.push(self.set_var(t, el));
                        }
                        Ok(ForParts { kind: ForKind::List { item: Target::Local(tmp), index: None, list: it }, prelude })
                    }
                    other => err(
                        pos,
                        format!(
                            "тізім элементі `{}` — оны {} айнымалыға ашуға болмайды; индекс керек болса `нөмірле(...)` қолданыңыз",
                            self.tn(&other),
                            targets.len()
                        ),
                    ),
                }
            }
            Ty::Str => {
                if targets.len() != 1 {
                    return err(pos, "мәтін бойынша циклда бір ғана айнымалы болады");
                }
                let item = self.loop_target(&targets[0].0, Ty::Str, targets[0].1)?;
                Ok(ForParts { kind: ForKind::Str { item, s: it }, prelude: vec![] })
            }
            Ty::Set(t) => {
                if targets.len() != 1 {
                    return err(pos, "жиын бойынша циклда бір ғана айнымалы болады");
                }
                let key = self.loop_target(&targets[0].0, *t, targets[0].1)?;
                Ok(ForParts { kind: ForKind::Dict { key, val: None, dict: it }, prelude: vec![] })
            }
            Ty::Dict(k, v) => {
                let key = self.loop_target(&targets[0].0, *k, targets[0].1)?;
                let val = if targets.len() == 2 { Some(self.loop_target(&targets[1].0, *v, targets[1].1)?) } else { None };
                Ok(ForParts { kind: ForKind::Dict { key, val, dict: it }, prelude: vec![] })
            }
            other => err(iter.pos, format!("`{}` типі бойынша қайталауға болмайды", self.tn(&other))),
        }
    }

    fn for_stmt(&mut self, targets: &[(String, Pos)], iter: &Expr, body: &[Stmt], pos: Pos, out: &mut Vec<TStmt>) -> Res<()> {
        let parts = self.for_parts(targets, iter, pos)?;
        self.cx.loops += 1;
        let b = self.block(body);
        self.cx.loops -= 1;
        out.push(parts.into_stmt(b?));
        Ok(())
    }

    fn list_comp(&mut self, elem: &Expr, targets: &[(String, Pos)], iter: &Expr, cond: Option<&Expr>, exp: Option<&Ty>, pos: Pos) -> Res<TExpr> {
        let parts = self.for_parts(targets, iter, pos)?;
        let c = match cond {
            Some(c) => {
                let v = self.expr(c, None)?;
                Some(self.truthy(v, c.pos)?)
            }
            None => None,
        };
        let elem_exp = match exp {
            Some(Ty::List(t)) => Some((**t).clone()),
            _ => None,
        };
        let e = self.expr(elem, elem_exp.as_ref())?;
        self.need_value(&e, elem.pos)?;
        let et = match elem_exp {
            Some(t) => t,
            None => {
                if e.ty == Ty::Null {
                    return err(elem.pos, "тізім элементінің типін анықтау мүмкін емес");
                }
                e.ty.clone()
            }
        };
        let e = self.coerce(e, &et, elem.pos)?;
        let lt = Ty::List(Box::new(et));
        let tmp = self.temp(lt.clone());
        let lv = texpr(TK::Local(tmp), lt.clone());
        let push = TStmt::Expr(texpr(TK::Builtin(Bi::ListPush, vec![lv.clone(), e]), Ty::None));
        let body = match c {
            Some(c) => vec![TStmt::If { cond: c, then: vec![push], els: vec![] }],
            None => vec![push],
        };
        let init = TStmt::SetLocal(tmp, texpr(TK::ListLit(vec![]), lt.clone()));
        Ok(texpr(TK::Block(vec![init, parts.into_stmt(body)], Box::new(lv)), lt))
    }

    // ---------------------------------------------------------------- функцияларды тексеру

    fn check_func(&mut self, fid: FuncId) -> Res<()> {
        self.funcs[fid].state = St::Busy;
        let ast = self.funcs[fid].ast.clone();
        let params = self.funcs[fid].params.clone();
        let declared = self.funcs[fid].ret.clone();
        let name = self.funcs[fid].name.clone();
        let saved = std::mem::take(&mut self.cx);
        self.cx.ret = declared.clone();
        self.cx.ret_declared = declared.is_some();
        for (pn, pt) in &params {
            let id = self.new_local(pn, pt.clone());
            if self.cx.names.insert(pn.clone(), id).is_some() {
                self.cx = saved;
                return err(ast.pos, format!("`{}` параметрі қайталанды", pn));
            }
        }
        let body = if ast.external { Ok(vec![]) } else { self.block(&ast.body) };
        let body = match body {
            Ok(b) => b,
            Err(e) => {
                self.cx = saved;
                return Err(e);
            }
        };
        let ret = self.cx.ret.clone().unwrap_or(Ty::None);
        if ret != Ty::None && !ast.external && !terminates(&body) {
            self.cx = saved;
            return err(ast.pos, format!("`{}` функциясы барлық жолдарда мән қайтармайды", ast.name));
        }
        let cx = std::mem::replace(&mut self.cx, saved);
        self.funcs[fid].final_ret = Some(ret.clone());
        self.funcs[fid].done = Some(TFunc { name, nparams: params.len(), locals: cx.locals, ret, body, external: ast.external });
        self.funcs[fid].state = St::Done;
        Ok(())
    }

    fn make_slot(&mut self, f: &FuncDef, recv: Option<ClassId>, display: String) -> Res<FuncSlot> {
        let mut params = Vec::new();
        let mut defaults = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            if i == 0 && recv.is_some() {
                params.push((p.name.clone(), Ty::Class(recv.unwrap())));
                defaults.push(None);
                continue;
            }
            let ty = match &p.ty {
                Some(t) => self.resolve_type(t)?,
                None => return err(p.pos, format!("`{}` параметрінің типін көрсетіңіз", p.name)),
            };
            if ty.is_unit() {
                return err(p.pos, "параметр `жоқ` типті бола алмайды");
            }
            let d = match &p.default {
                Some(e) => {
                    let saved = std::mem::take(&mut self.cx);
                    self.cx.is_main = true;
                    let v = self.expr(e, Some(&ty)).and_then(|v| self.coerce(v, &ty, e.pos));
                    self.cx = saved;
                    Some(v?)
                }
                None => None,
            };
            params.push((p.name.clone(), ty));
            defaults.push(d);
        }
        if recv.is_some() && f.params.is_empty() {
            return err(f.pos, "әдістің бірінші параметрі объектінің өзі (`өзі`) болуы керек");
        }
        let ret = match &f.ret {
            Some(t) => Some(self.resolve_type(t)?),
            None => None,
        };
        Ok(FuncSlot { name: display, params, defaults, ret, ast: Rc::new(f.clone()), state: St::Todo, done: None, final_ret: None })
    }

    pub fn check_program(prog: &Program) -> Res<Module> {
        let mut c = Checker {
            classes: vec![],
            class_names: HashMap::new(),
            class_methods: vec![],
            class_ctor: vec![],
            class_defaults: vec![],
            class_children: vec![],
            used_virtual: vec![],
            overrides: vec![],
            funcs: vec![],
            func_names: HashMap::new(),
            globals: vec![],
            global_ids: HashMap::new(),
            global_names: HashSet::new(),
            cx: Ctx::default(),
        };

        // 1) класстардың аттары
        let mut bases: Vec<Option<(String, Pos)>> = Vec::new();
        for s in &prog.stmts {
            if let StmtKind::ClassDef(cd) = &s.kind {
                c.check_not_reserved(&cd.name, cd.pos)?;
                if c.class_names.contains_key(&cd.name) {
                    return err(cd.pos, format!("`{}` класы қайта анықталған", cd.name));
                }
                c.class_names.insert(cd.name.clone(), c.classes.len());
                c.classes.push(ClassInfo { name: cd.name.clone(), fields: vec![], base: None, has_header: false });
                c.class_methods.push(HashMap::new());
                c.class_ctor.push(None);
                c.class_defaults.push(vec![]);
                c.class_children.push(vec![]);
                bases.push(cd.base.clone());
            }
        }
        // 2) мұралау: ата-аналар, топологиялық рет
        for (cid, b) in bases.iter().enumerate() {
            if let Some((bn, bp)) = b {
                match c.class_names.get(bn) {
                    Some(&bid) if bid == cid => return err(*bp, "класс өзінен мұралана алмайды"),
                    Some(&bid) => {
                        c.classes[cid].base = Some(bid);
                        c.class_children[bid].push(cid);
                    }
                    None => return err(*bp, format!("белгісіз ата-ана класс `{}`", bn)),
                }
            }
        }
        let mut order: Vec<ClassId> = Vec::new();
        {
            let mut state = vec![0u8; c.classes.len()];
            fn visit(c: &Checker, i: usize, state: &mut Vec<u8>, order: &mut Vec<ClassId>) -> bool {
                if state[i] == 2 {
                    return true;
                }
                if state[i] == 1 {
                    return false;
                }
                state[i] = 1;
                if let Some(b) = c.classes[i].base {
                    if !visit(c, b, state, order) {
                        return false;
                    }
                }
                state[i] = 2;
                order.push(i);
                true
            }
            for i in 0..c.classes.len() {
                if !visit(&c, i, &mut state, &mut order) {
                    return err(Pos::default(), format!("`{}` класының мұралануында цикл бар", c.classes[i].name));
                }
            }
        }
        for i in 0..c.classes.len() {
            c.classes[i].has_header = c.classes[i].base.is_some() || !c.class_children[i].is_empty();
        }
        let class_asts: HashMap<String, &ClassDef> = prog
            .stmts
            .iter()
            .filter_map(|s| if let StmtKind::ClassDef(cd) = &s.kind { Some((cd.name.clone(), cd)) } else { None })
            .collect();
        // өрістер (ата-ана өрістері алдымен)
        for &cid in &order {
            let cd = class_asts[&c.classes[cid].name];
            let mut fields: Vec<FieldInfo> = match c.classes[cid].base {
                Some(b) => c.classes[b].fields.clone(),
                None => vec![],
            };
            for f in &cd.fields {
                if fields.iter().any(|x: &FieldInfo| x.name == f.name) {
                    return err(f.pos, format!("`{}` өрісі қайталанды (немесе ата-анадан мұраланған)", f.name));
                }
                let ty = c.resolve_type(&f.ty)?;
                if ty.is_unit() {
                    return err(f.pos, "өріс `жоқ` типті бола алмайды");
                }
                fields.push(FieldInfo { name: f.name.clone(), ty });
            }
            c.classes[cid].fields = fields;
        }
        // 3) глобал атаулар
        let mut gn = HashSet::new();
        for s in &prog.stmts {
            match &s.kind {
                StmtKind::FuncDef(f) => scan_func(f, &mut gn),
                StmtKind::ClassDef(cd) => {
                    for m in &cd.methods {
                        scan_func(m, &mut gn);
                    }
                    for f in &cd.fields {
                        if let Some(d) = &f.default {
                            let mut sc = Scan::default();
                            scan_expr(d, &mut sc);
                            gn.extend(sc.reads);
                        }
                    }
                }
                _ => {}
            }
        }
        c.global_names = gn;
        // 4) функциялар мен әдістер
        for s in &prog.stmts {
            match &s.kind {
                StmtKind::FuncDef(f) => {
                    c.check_not_reserved(&f.name, f.pos)?;
                    if c.func_names.contains_key(&f.name) || c.class_names.contains_key(&f.name) {
                        return err(f.pos, format!("`{}` қайта анықталған", f.name));
                    }
                    let slot = c.make_slot(f, None, f.name.clone())?;
                    c.func_names.insert(f.name.clone(), c.funcs.len());
                    c.funcs.push(slot);
                }
                _ => {}
            }
        }
        // әдістер (ата-ана әдістері алдымен, мұралау және қайта анықтау)
        for &cid in &order {
            let cd = class_asts[&c.classes[cid].name];
            if let Some(b) = c.classes[cid].base {
                c.class_methods[cid] = c.class_methods[b].clone();
                c.class_ctor[cid] = c.class_ctor[b];
            }
            let mut own: HashSet<String> = HashSet::new();
            for m in &cd.methods {
                if !own.insert(m.name.clone()) {
                    return err(m.pos, format!("`{}` әдісі қайталанды", m.name));
                }
                let slot = c.make_slot(m, Some(cid), format!("{}.{}", cd.name, m.name))?;
                let fid = c.funcs.len();
                if let Some(&pf) = c.class_methods[cid].get(&m.name) {
                    if m.name != "құру" {
                        let pp: Vec<Ty> = c.funcs[pf].params.iter().skip(1).map(|p| p.1.clone()).collect();
                        let np: Vec<Ty> = slot.params.iter().skip(1).map(|p| p.1.clone()).collect();
                        if pp != np {
                            return err(m.pos, format!("`{}` әдісі ата-анадағы параметрлермен сәйкес келмейді", m.name));
                        }
                        if let (Some(a), Some(b)) = (&slot.ret, &c.funcs[pf].ret) {
                            if a != b {
                                return err(m.pos, format!("`{}` әдісінің қайтару типі ата-анадағыдан өзгеше", m.name));
                            }
                        }
                        c.overrides.push((fid, pf));
                    }
                }
                c.funcs.push(slot);
                c.class_methods[cid].insert(m.name.clone(), fid);
                if m.name == "құру" {
                    c.class_ctor[cid] = Some(fid);
                }
            }
        }
        // өрістердің әдепкі мәндері
        for &cid in &order {
            let cd = class_asts[&c.classes[cid].name];
            let mut ds: Vec<Option<TExpr>> = match c.classes[cid].base {
                Some(b) => c.class_defaults[b].clone(),
                None => vec![],
            };
            for f in &cd.fields {
                let ty = c.classes[cid].fields.iter().find(|x| x.name == f.name).unwrap().ty.clone();
                ds.push(match &f.default {
                    Some(e) => {
                        let saved = std::mem::take(&mut c.cx);
                        c.cx.is_main = true;
                        let v = c.expr(e, Some(&ty)).and_then(|v| c.coerce(v, &ty, e.pos));
                        c.cx = saved;
                        Some(v?)
                    }
                    None => None,
                });
            }
            c.class_defaults[cid] = ds;
        }

        // 5) негізгі код
        let main_fid = c.funcs.len();
        let main_ast = FuncDef { name: "<main>".into(), params: vec![], ret: None, body: vec![], external: false, pos: Pos::default() };
        c.funcs.push(FuncSlot {
            name: "<main>".into(),
            params: vec![],
            defaults: vec![],
            ret: Some(Ty::None),
            ast: Rc::new(main_ast),
            state: St::Busy,
            done: None,
            final_ret: Some(Ty::None),
        });
        c.cx = Ctx::default();
        c.cx.is_main = true;
        c.cx.ret = Some(Ty::None);
        let mut main_body = Vec::new();
        for s in &prog.stmts {
            if matches!(s.kind, StmtKind::FuncDef(_) | StmtKind::ClassDef(_)) {
                continue;
            }
            c.stmt(s, &mut main_body)?;
        }
        let main_cx = std::mem::take(&mut c.cx);
        c.funcs[main_fid].done = Some(TFunc { name: "<main>".into(), nparams: 0, locals: main_cx.locals, ret: Ty::None, body: main_body, external: false });
        c.funcs[main_fid].state = St::Done;

        // 6) қалған функциялар
        let mut fid = 0;
        while fid < c.funcs.len() {
            if c.funcs[fid].state == St::Todo {
                c.check_func(fid)?;
            }
            fid += 1;
        }
        // ctor тексеруі
        for (cid, ctor) in c.class_ctor.clone().into_iter().enumerate() {
            if let Some(f) = ctor {
                if c.funcs[f].final_ret != Some(Ty::None) {
                    return err(c.funcs[f].ast.pos, format!("`құру` әдісі мән қайтармауы керек (класс `{}`)", c.classes[cid].name));
                }
            }
        }

        for &(child, parent) in &c.overrides {
            if c.funcs[child].final_ret != c.funcs[parent].final_ret {
                return err(c.funcs[child].ast.pos, format!("`{}` әдісінің қайтару типі ата-анадағыдан өзгеше", c.funcs[child].name));
            }
        }
        let mut vtables = Vec::new();
        for name in &c.used_virtual {
            let table: Vec<Option<FuncId>> = (0..c.classes.len()).map(|cid| c.class_methods[cid].get(name).copied()).collect();
            vtables.push((name.clone(), table));
        }
        let funcs = c.funcs.into_iter().map(|f| f.done.unwrap()).collect();
        Ok(Module { funcs, classes: c.classes, globals: c.globals, main: main_fid, vtables })
    }
}
