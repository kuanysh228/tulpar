//! HIR интерпретаторы — Tulpar-ды басқа бағдарламаға ендіру үшін (мысалы, бизнес-платформаның конфигурация тілі).
//!
//! Компиляцияланған кодпен бірдей нәтиже береді, бірақ:
//! * файлға, желіге, `қолмен:` жадына қолжетімділік жоқ — тек хост (`Host`) берген `сыртқы` функциялар;
//! * орындау қадамдары мен рекурсия тереңдігі шектеулі (шексіз цикл хостты қатырмайды);
//! * қате `Result` арқылы қайтарылады (хост транзакцияны кері қайтара алады).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use crate::ast::{BinOp, CmpOp, UnOp};
use crate::hir::*;

// ---------------------------------------------------------------------------------------- мәндер

/// Өзгермейтін мәтін; таңба санын алдын ала біледі (ASCII үшін индекстеу O(1)).
pub struct TStr {
    pub s: String,
    pub nchars: usize,
}

impl TStr {
    pub fn new(s: String) -> Rc<TStr> {
        let nchars = if s.is_ascii() { s.len() } else { s.chars().count() };
        Rc::new(TStr { s, nchars })
    }
    fn ascii(&self) -> bool {
        self.nchars == self.s.len()
    }
    /// таңба индексінен байт ығысуына
    fn boff(&self, ci: usize) -> usize {
        if self.ascii() {
            return ci;
        }
        self.s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(self.s.len())
    }
    fn slice(&self, lo: usize, hi: usize) -> Rc<TStr> {
        if self.ascii() {
            return TStr::new(self.s[lo..hi].to_string());
        }
        let bl = self.boff(lo);
        let bh = bl + self.s[bl..].char_indices().nth(hi - lo).map(|(b, _)| b).unwrap_or(self.s.len() - bl);
        TStr::new(self.s[bl..bh].to_string())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Int(i64),
    Bool(bool),
    Str(String),
}

impl Key {
    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(i) => Value::Int(*i),
            Key::Bool(b) => Value::Bool(*b),
            Key::Str(s) => Value::str(s),
        }
    }
}

/// Енгізу реті сақталатын хэш-кесте (сөздік пен жиын үшін)
#[derive(Default)]
pub struct Dict {
    entries: Vec<Option<(Key, Value)>>,
    index: HashMap<Key, usize>,
    live: usize,
}

impl Dict {
    pub fn new() -> Dict {
        Dict::default()
    }
    pub fn len(&self) -> usize {
        self.live
    }
    pub fn get(&self, k: &Key) -> Option<&Value> {
        self.index.get(k).and_then(|&i| self.entries[i].as_ref().map(|e| &e.1))
    }
    pub fn contains(&self, k: &Key) -> bool {
        self.index.contains_key(k)
    }
    pub fn insert(&mut self, k: Key, v: Value) {
        if let Some(&i) = self.index.get(&k) {
            self.entries[i].as_mut().unwrap().1 = v;
        } else {
            self.index.insert(k.clone(), self.entries.len());
            self.entries.push(Some((k, v)));
            self.live += 1;
        }
    }
    pub fn remove(&mut self, k: &Key) {
        if let Some(i) = self.index.remove(k) {
            self.entries[i] = None;
            self.live -= 1;
            if self.entries.len() > 32 && self.live * 2 < self.entries.len() {
                let old = std::mem::take(&mut self.entries);
                self.index.clear();
                for e in old.into_iter().flatten() {
                    self.index.insert(e.0.clone(), self.entries.len());
                    self.entries.push(Some(e));
                }
            }
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &(Key, Value)> {
        self.entries.iter().flatten()
    }
    pub fn clear(&mut self) {
        *self = Dict::default();
    }
}

pub struct Obj {
    pub class: ClassId,
    pub fields: Vec<Value>,
}

#[derive(Clone)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Rc<TStr>),
    List(Rc<RefCell<Vec<Value>>>),
    Dict(Rc<RefCell<Dict>>),
    Tuple(Rc<Vec<Value>>),
    Obj(Rc<RefCell<Obj>>),
    Func(FuncId),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "жоқ"),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(x) => write!(f, "{}", x),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "{:?}", s.s),
            Value::List(l) => f.debug_list().entries(l.borrow().iter()).finish(),
            Value::Dict(d) => f.debug_map().entries(d.borrow().iter().map(|(k, v)| (k, v))).finish(),
            Value::Tuple(t) => f.debug_list().entries(t.iter()).finish(),
            Value::Obj(_) => write!(f, "<объект>"),
            Value::Func(i) => write!(f, "<функция {}>", i),
        }
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(TStr::new(s.to_string()))
    }
    pub fn string(s: String) -> Value {
        Value::Str(TStr::new(s))
    }
    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(v)))
    }
    pub fn dict(d: Dict) -> Value {
        Value::Dict(Rc::new(RefCell::new(d)))
    }
    pub fn tuple(v: Vec<Value>) -> Value {
        Value::Tuple(Rc::new(v))
    }
    pub fn as_int(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            Value::Bool(b) => *b as i64,
            Value::Float(f) => *f as i64,
            _ => 0,
        }
    }
    pub fn as_f64(&self) -> f64 {
        match self {
            Value::Float(f) => *f,
            Value::Int(i) => *i as f64,
            _ => 0.0,
        }
    }
    pub fn as_bool(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
    /// мәтін (null болса — бос)
    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => &s.s,
            _ => "",
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn as_list(&self) -> Vec<Value> {
        match self {
            Value::List(l) => l.borrow().clone(),
            _ => vec![],
        }
    }
    /// сөздік/жиын жазбалары (кілт, мән)
    pub fn as_dict_items(&self) -> Vec<(Key, Value)> {
        match self {
            Value::Dict(d) => d.borrow().iter().cloned().collect(),
            _ => vec![],
        }
    }
    pub fn key(&self) -> Option<Key> {
        match self {
            Value::Int(i) => Some(Key::Int(*i)),
            Value::Bool(b) => Some(Key::Bool(*b)),
            Value::Str(s) => Some(Key::Str(s.s.clone())),
            Value::Null => Some(Key::Str(String::new())),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------- қателер

#[derive(Debug, Clone)]
pub struct RtError {
    pub msg: String,
    /// шақырулар тізбегі: ішкіден сыртқыға ("функция (файл:жол)")
    pub trace: Vec<String>,
}

impl RtError {
    pub fn render(&self) -> String {
        let mut s = String::new();
        if !self.trace.is_empty() {
            s.push_str("Шақырулар тізбегі (соңғысы төменде):\n");
            for t in self.trace.iter().rev() {
                let _ = writeln!(s, "  {}", t);
            }
        }
        let _ = write!(s, "Қате: {}", self.msg);
        s
    }
}

pub type R<T> = Result<T, Box<RtError>>;

fn rt<T>(msg: impl Into<String>) -> R<T> {
    Err(Box::new(RtError { msg: msg.into(), trace: vec![] }))
}

// ---------------------------------------------------------------------------------------- хост

/// Хост `&self` арқылы шақырылады (ішкі күйі `RefCell` ішінде), сондықтан `it` арқылы интерпретаторға
/// қайта кіруге болады (`it.call(..)`): мысалы, жазу оқиғаларын орындау.
pub trait Host {
    /// `сыртқы анықта` функциясын шақыру. Қате — мәтін (бағдарлама тоқтайды).
    fn call(&self, it: &mut Interp, name: &str, args: Vec<Value>) -> Result<Value, String>;
    fn print(&self, text: &str) {
        print!("{}", text);
    }
}

/// Сыртқы функциялары жоқ хост (командалық жол үшін)
pub struct NoHost;

impl Host for NoHost {
    fn call(&self, _it: &mut Interp, name: &str, _args: Vec<Value>) -> Result<Value, String> {
        Err(format!("`сыртқы` функция `{}` хостта жоқ", name))
    }
}

// ---------------------------------------------------------------------------------------- интерпретатор

enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

struct Frame {
    locals: Vec<Value>,
    line: (u16, u32),
}

pub struct Interp {
    module: Rc<Module>,
    globals: Vec<Value>,
    host: Rc<dyn Host>,
    vtables: HashMap<String, usize>,
    names: HashMap<String, FuncId>,
    files: Vec<String>,
    steps: u64,
    depth: usize,
    rng: u64,
    pub max_steps: u64,
    pub max_depth: usize,
}

fn zero(t: &Ty) -> Value {
    match t {
        Ty::Int | Ty::Byte => Value::Int(0),
        Ty::Float => Value::Float(0.0),
        Ty::Bool => Value::Bool(false),
        _ => Value::Null,
    }
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn to_lower(c: u32) -> u32 {
    match c {
        0x41..=0x5A => c + 32,
        0x410..=0x42F => c + 32,
        0x400..=0x40F => c + 0x50,
        0x460..=0x481 | 0x48A..=0x4BF | 0x4D0..=0x52F if c % 2 == 0 => c + 1,
        0x4C1..=0x4CD if c % 2 == 1 => c + 1,
        0x4C0 => 0x4CF,
        _ => c,
    }
}

fn to_upper(c: u32) -> u32 {
    match c {
        0x61..=0x7A => c - 32,
        0x430..=0x44F => c - 32,
        0x450..=0x45F => c - 0x50,
        0x460..=0x481 | 0x48A..=0x4BF | 0x4D0..=0x52F if c % 2 == 1 => c - 1,
        0x4C1..=0x4CE if c % 2 == 0 => c - 1,
        0x4CF => 0x4C0,
        _ => c,
    }
}

fn map_case(s: &str, f: fn(u32) -> u32) -> String {
    s.chars().map(|c| char::from_u32(f(c as u32)).unwrap_or(c)).collect()
}

fn fmt_float(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf".into() } else { "inf".into() };
    }
    let s = format!("{:?}", x);
    if let Some(e) = s.find('e') {
        let mut out = s[..=e].to_string();
        let mut rest = &s[e + 1..];
        if let Some(r) = rest.strip_prefix('-') {
            out.push('-');
            rest = r;
        } else {
            out.push('+');
        }
        if rest.len() < 2 {
            out.push('0');
        }
        out.push_str(rest);
        out
    } else {
        s
    }
}

fn fmt_fixed(x: f64, prec: i64) -> String {
    if x.is_nan() {
        "nan".into()
    } else if x.is_infinite() {
        if x < 0.0 { "-inf".into() } else { "inf".into() }
    } else if x.abs() >= 1e200 {
        fmt_float(x)
    } else {
        format!("{:.*}", prec.clamp(0, 40) as usize, x)
    }
}

fn norm_slice(len: i64, lo: i64, hi: i64) -> (usize, usize) {
    let mut lo = if lo < 0 { lo + len } else { lo };
    let mut hi = if hi < 0 { hi + len } else { hi };
    lo = lo.clamp(0, len);
    hi = hi.clamp(0, len);
    if hi < lo {
        hi = lo;
    }
    (lo as usize, hi as usize)
}

fn idx_err<T>(i: i64, len: usize) -> R<T> {
    rt(format!("индекс шектен тыс: {} (ұзындығы {})", i, len))
}

fn null_err<T>() -> R<T> {
    rt("нөлдік көрсеткішке (жоқ) қатынау")
}

fn floor_div(a: i64, b: i64) -> i64 {
    let q = a.wrapping_div(b);
    if a.wrapping_rem(b) != 0 && ((a < 0) != (b < 0)) { q - 1 } else { q }
}

fn floor_mod(a: i64, b: i64) -> i64 {
    let r = a.wrapping_rem(b);
    if r != 0 && ((r < 0) != (b < 0)) { r + b } else { r }
}

impl Interp {
    pub fn new(module: Module, host: Rc<dyn Host>) -> Interp {
        let mut vtables = HashMap::new();
        for (i, (n, _)) in module.vtables.iter().enumerate() {
            vtables.insert(n.clone(), i);
        }
        let mut names = HashMap::new();
        for (i, f) in module.funcs.iter().enumerate() {
            names.insert(f.name.clone(), i);
        }
        let globals = module.globals.iter().map(|g| zero(&g.ty)).collect();
        Interp {
            module: Rc::new(module),
            globals,
            host,
            vtables,
            names,
            files: crate::diag::file_names(),
            steps: 0,
            depth: 0,
            rng: 0x9E37_79B9_7F4A_7C15 ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1),
            max_steps: 200_000_000,
            max_depth: 1500,
        }
    }

    /// Қате трассасындағы файл аттары (реестр ағынға тәуелді, сондықтан қолмен беруге болады)
    pub fn set_file_names(&mut self, files: Vec<String>) {
        self.files = files;
    }

    pub fn set_host(&mut self, host: Rc<dyn Host>) {
        self.host = host;
    }

    /// Глобал код (модуль денесі) бір рет орындалады
    pub fn run_main(&mut self) -> R<()> {
        self.steps = 0;
        let main = self.module.main;
        self.call_id(main, vec![]).map(|_| ())
    }

    pub fn func_id(&self, name: &str) -> Option<FuncId> {
        self.names.get(name).copied()
    }

    pub fn has_func(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Аты бойынша функцияны шақыру (қадам шегі әр шақыруда жаңарады)
    pub fn call(&mut self, name: &str, args: Vec<Value>) -> R<Value> {
        match self.func_id(name) {
            Some(id) => {
                if self.depth == 0 {
                    self.steps = 0; // сыртқы шақыруда ғана қадам шегі жаңарады, ішкі (қайта кіру) шақыруда жоқ
                }
                self.call_id(id, args)
            }
            None => rt(format!("функция табылмады: {}", name)),
        }
    }

    pub fn class_id(&self, name: &str) -> Option<ClassId> {
        self.module.classes.iter().position(|c| c.name == name)
    }

    pub fn class_fields(&self, class: ClassId) -> Vec<(String, Ty)> {
        self.module.classes[class].fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect()
    }

    pub fn new_object(&self, class: ClassId) -> Value {
        let fields = self.module.classes[class].fields.iter().map(|f| zero(&f.ty)).collect();
        Value::Obj(Rc::new(RefCell::new(Obj { class, fields })))
    }

    pub fn get_field(&self, obj: &Value, name: &str) -> Option<Value> {
        if let Value::Obj(o) = obj {
            let o = o.borrow();
            let pos = self.module.classes[o.class].fields.iter().position(|f| f.name == name)?;
            return Some(o.fields[pos].clone());
        }
        None
    }

    pub fn set_field(&self, obj: &Value, name: &str, v: Value) -> bool {
        if let Value::Obj(o) = obj {
            let mut o = o.borrow_mut();
            if let Some(pos) = self.module.classes[o.class].fields.iter().position(|f| f.name == name) {
                o.fields[pos] = v;
                return true;
            }
        }
        false
    }

    pub fn class_of(&self, obj: &Value) -> Option<String> {
        if let Value::Obj(o) = obj {
            return Some(self.module.classes[o.borrow().class].name.clone());
        }
        None
    }

    // ------------------------------------------------------------------ функцияларды шақыру

    fn call_id(&mut self, fid: FuncId, args: Vec<Value>) -> R<Value> {
        let m = Rc::clone(&self.module);
        let f = &m.funcs[fid];
        if f.external {
            let host = Rc::clone(&self.host);
            return match host.call(self, &f.name, args) {
                Ok(v) => Ok(v),
                Err(e) => rt(e),
            };
        }
        if self.depth >= self.max_depth {
            return rt("стек толып кетті (рекурсия тым терең)");
        }
        self.depth += 1;
        let mut fr = Frame { locals: f.locals.iter().map(|l| zero(&l.ty)).collect(), line: (0, 0) };
        for (i, a) in args.into_iter().enumerate().take(f.nparams) {
            fr.locals[i] = a;
        }
        let r = self.exec_block(&f.body, &mut fr);
        self.depth -= 1;
        match r {
            Ok(Flow::Return(v)) => Ok(v),
            Ok(_) => Ok(Value::Null),
            Err(mut e) => {
                let file = self.files.get(fr.line.0 as usize).cloned().unwrap_or_default();
                e.trace.push(format!("\"{}\", {}-жол, {}", file, fr.line.1, f.name));
                Err(e)
            }
        }
    }

    // ------------------------------------------------------------------ операторлар

    fn step(&mut self) -> R<()> {
        self.steps += 1;
        if self.steps > self.max_steps {
            return rt("орындау қадамдарының шегі асып кетті (шексіз цикл болуы мүмкін)");
        }
        Ok(())
    }

    fn exec_block(&mut self, b: &[TStmt], fr: &mut Frame) -> R<Flow> {
        for s in b {
            match self.exec(s, fr)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn set_target(&mut self, t: &Target, v: Value, fr: &mut Frame) {
        match t {
            Target::Local(i) => fr.locals[*i] = v,
            Target::Global(i) => self.globals[*i] = v,
        }
    }

    fn exec(&mut self, s: &TStmt, fr: &mut Frame) -> R<Flow> {
        self.step()?;
        match s {
            TStmt::Line(f, l) => fr.line = (*f, *l),
            TStmt::Expr(e) => {
                self.eval(e, fr)?;
            }
            TStmt::SetLocal(i, e) => {
                let v = self.eval(e, fr)?;
                fr.locals[*i] = v;
            }
            TStmt::SetGlobal(i, e) => {
                let v = self.eval(e, fr)?;
                self.globals[*i] = v;
            }
            TStmt::SetIndex(c, i, v) => {
                let cv = self.eval(c, fr)?;
                let iv = self.eval(i, fr)?;
                let vv = self.eval(v, fr)?;
                match cv {
                    Value::List(l) => {
                        let mut l = l.borrow_mut();
                        let n = l.len();
                        let k = iv.as_int();
                        let idx = if k < 0 { k + n as i64 } else { k };
                        if idx < 0 || idx >= n as i64 {
                            return idx_err(k, n);
                        }
                        l[idx as usize] = vv;
                    }
                    Value::Dict(d) => match iv.key() {
                        Some(k) => d.borrow_mut().insert(k, vv),
                        None => return rt("сөздік кілті жарамсыз"),
                    },
                    Value::Null => return null_err(),
                    _ => return rt("бұл мәнге индекс бойынша жазу мүмкін емес"),
                }
            }
            TStmt::SetField(o, idx, v) => {
                let ov = self.eval(o, fr)?;
                let vv = self.eval(v, fr)?;
                match ov {
                    Value::Obj(o) => o.borrow_mut().fields[*idx] = vv,
                    _ => return null_err(),
                }
            }
            TStmt::SetDeref(..) => return rt("көрсеткіштер интерпретаторда қолжетімсіз"),
            TStmt::If { cond, then, els } => {
                let c = self.eval(cond, fr)?;
                return if c.as_bool() { self.exec_block(then, fr) } else { self.exec_block(els, fr) };
            }
            TStmt::While { cond, body } => loop {
                self.step()?;
                if !self.eval(cond, fr)?.as_bool() {
                    break;
                }
                match self.exec_block(body, fr)? {
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    _ => {}
                }
            },
            TStmt::ForRange { var, start, stop, step, body } => {
                let mut i = self.eval(start, fr)?.as_int();
                let stop = self.eval(stop, fr)?.as_int();
                let st = self.eval(step, fr)?.as_int();
                if st == 0 {
                    return rt("`аралық`: қадам нөл болмауы керек");
                }
                while (st > 0 && i < stop) || (st < 0 && i > stop) {
                    self.step()?;
                    self.set_target(var, Value::Int(i), fr);
                    match self.exec_block(body, fr)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    i = match i.checked_add(st) {
                        Some(x) => x,
                        None => break,
                    };
                }
            }
            TStmt::ForList { item, index, list, body } => {
                let lv = self.eval(list, fr)?;
                let l = match lv {
                    Value::List(l) => l,
                    Value::Null => return null_err(),
                    _ => return rt("тізім күтілді"),
                };
                let mut i = 0usize;
                loop {
                    self.step()?;
                    let x = {
                        let b = l.borrow();
                        if i >= b.len() {
                            break;
                        }
                        b[i].clone()
                    };
                    self.set_target(item, x, fr);
                    if let Some(ix) = index {
                        self.set_target(ix, Value::Int(i as i64), fr);
                    }
                    match self.exec_block(body, fr)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    i += 1;
                }
            }
            TStmt::ForStr { item, s, body } => {
                let sv = self.eval(s, fr)?;
                let chars: Vec<char> = sv.as_str().chars().collect();
                for c in chars {
                    self.step()?;
                    self.set_target(item, Value::string(c.to_string()), fr);
                    match self.exec_block(body, fr)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            TStmt::ForDict { key, val, dict, body } => {
                let dv = self.eval(dict, fr)?;
                let items = match &dv {
                    Value::Dict(d) => d.borrow().iter().cloned().collect::<Vec<_>>(),
                    Value::Null => return null_err(),
                    _ => return rt("сөздік күтілді"),
                };
                for (k, v) in items {
                    self.step()?;
                    self.set_target(key, k.to_value(), fr);
                    if let Some(vt) = val {
                        self.set_target(vt, v, fr);
                    }
                    match self.exec_block(body, fr)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            TStmt::Break => return Ok(Flow::Break),
            TStmt::Continue => return Ok(Flow::Continue),
            TStmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e, fr)?,
                    None => Value::Null,
                };
                return Ok(Flow::Return(v));
            }
        }
        Ok(Flow::Normal)
    }

    // ------------------------------------------------------------------ өрнектер

    fn eval_args(&mut self, args: &[TExpr], fr: &mut Frame) -> R<Vec<Value>> {
        let mut v = Vec::with_capacity(args.len());
        for a in args {
            v.push(self.eval(a, fr)?);
        }
        Ok(v)
    }

    fn eval(&mut self, e: &TExpr, fr: &mut Frame) -> R<Value> {
        Ok(match &e.kind {
            TK::Int(i) => Value::Int(*i),
            TK::Float(f) => Value::Float(*f),
            TK::Bool(b) => Value::Bool(*b),
            TK::Str(s) => Value::str(s),
            TK::Null => Value::Null,
            TK::Local(i) => fr.locals[*i].clone(),
            TK::Global(i) => self.globals[*i].clone(),
            TK::Unary(op, a) => {
                let v = self.eval(a, fr)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(i)) => Value::Int(i.wrapping_neg()),
                    (UnOp::Neg, Value::Float(f)) => Value::Float(-f),
                    (UnOp::Pos, v) => v,
                    (UnOp::Not, Value::Bool(b)) => Value::Bool(!b),
                    (UnOp::BitNot, Value::Int(i)) => Value::Int(!i),
                    _ => return rt("унарлық амал қолданылмайды"),
                }
            }
            TK::Binary(op, a, b) => {
                let x = self.eval(a, fr)?;
                let y = self.eval(b, fr)?;
                self.binary(*op, x, y)?
            }
            TK::Cmp(op, a, b) => {
                let x = self.eval(a, fr)?;
                let y = self.eval(b, fr)?;
                Value::Bool(self.compare(*op, &x, &y, &a.ty, &b.ty))
            }
            TK::And(a, b) => {
                if !self.eval(a, fr)?.as_bool() {
                    Value::Bool(false)
                } else {
                    Value::Bool(self.eval(b, fr)?.as_bool())
                }
            }
            TK::Or(a, b) => {
                if self.eval(a, fr)?.as_bool() {
                    Value::Bool(true)
                } else {
                    Value::Bool(self.eval(b, fr)?.as_bool())
                }
            }
            TK::Cast(k, a) => {
                let v = self.eval(a, fr)?;
                match k {
                    CastKind::IntToFloat => Value::Float(v.as_int() as f64),
                    CastKind::FloatToInt => Value::Int(v.as_f64() as i64),
                    CastKind::ByteToInt => Value::Int(v.as_int()),
                    CastKind::IntToByte => Value::Int(v.as_int() & 0xFF),
                    CastKind::BoolToInt => Value::Int(v.as_int()),
                    CastKind::Reinterpret => return rt("көрсеткіштер интерпретаторда қолжетімсіз"),
                }
            }
            TK::Call(fid, args) => {
                let a = self.eval_args(args, fr)?;
                self.call_id(*fid, a)?
            }
            TK::Builtin(bi, args) => {
                let a = self.eval_args(args, fr)?;
                self.builtin(bi, a, args, &e.ty)?
            }
            TK::Index(c, i) => {
                let cv = self.eval(c, fr)?;
                let iv = self.eval(i, fr)?;
                self.index(&cv, &iv)?
            }
            TK::Slice(c, lo, hi) => {
                let cv = self.eval(c, fr)?;
                let lo = match lo {
                    Some(x) => Some(self.eval(x, fr)?.as_int()),
                    None => None,
                };
                let hi = match hi {
                    Some(x) => Some(self.eval(x, fr)?.as_int()),
                    None => None,
                };
                match cv {
                    Value::Str(s) => {
                        let (l, h) = norm_slice(s.nchars as i64, lo.unwrap_or(0), hi.unwrap_or(i64::MAX));
                        Value::Str(s.slice(l, h))
                    }
                    Value::List(l) => {
                        let l = l.borrow();
                        let (a, b) = norm_slice(l.len() as i64, lo.unwrap_or(0), hi.unwrap_or(i64::MAX));
                        Value::list(l[a..b].to_vec())
                    }
                    Value::Null => {
                        if matches!(c.ty, Ty::Str) {
                            Value::str("")
                        } else {
                            return null_err();
                        }
                    }
                    _ => return rt("кесу қолданылмайды"),
                }
            }
            TK::Field(o, idx) => match self.eval(o, fr)? {
                Value::Obj(o) => o.borrow().fields[*idx].clone(),
                _ => return null_err(),
            },
            TK::ListLit(items) => Value::list(self.eval_args(items, fr)?),
            TK::DictLit(items) => {
                let mut d = Dict::new();
                for (k, v) in items {
                    let kv = self.eval(k, fr)?;
                    let vv = self.eval(v, fr)?;
                    match kv.key() {
                        Some(k) => d.insert(k, vv),
                        None => return rt("сөздік кілті жарамсыз"),
                    }
                }
                Value::dict(d)
            }
            TK::SetLit(items) => {
                let mut d = Dict::new();
                for k in items {
                    let kv = self.eval(k, fr)?;
                    match kv.key() {
                        Some(k) => d.insert(k, Value::Null),
                        None => return rt("жиын элементі жарамсыз"),
                    }
                }
                Value::dict(d)
            }
            TK::TupleLit(items) => Value::tuple(self.eval_args(items, fr)?),
            TK::TupleGet(t, i) => match self.eval(t, fr)? {
                Value::Tuple(v) => v[*i].clone(),
                _ => return null_err(),
            },
            TK::Block(stmts, last) => {
                for s in stmts {
                    match self.exec(s, fr)? {
                        Flow::Normal => {}
                        _ => return rt("ішкі қате: блок ішінде басқару ағыны"),
                    }
                }
                self.eval(last, fr)?
            }
            TK::DeepEq(a, b, neg) => {
                let x = self.eval(a, fr)?;
                let y = self.eval(b, fr)?;
                Value::Bool(self.deep_eq(&x, &y) != *neg)
            }
            TK::VCall(name, args) => {
                let a = self.eval_args(args, fr)?;
                let cid = match a.first() {
                    Some(Value::Obj(o)) => o.borrow().class,
                    _ => return null_err(),
                };
                let vi = match self.vtables.get(name) {
                    Some(&i) => i,
                    None => return rt(format!("виртуалды әдіс табылмады: {}", name)),
                };
                match self.module.vtables[vi].1.get(cid).copied().flatten() {
                    Some(fid) => self.call_id(fid, a)?,
                    None => return rt(format!("әдіс `{}` бұл класта жоқ", name)),
                }
            }
            TK::FuncRef(f) => Value::Func(*f),
            TK::CallValue(f, args) => {
                let fv = self.eval(f, fr)?;
                let a = self.eval_args(args, fr)?;
                match fv {
                    Value::Func(fid) => self.call_id(fid, a)?,
                    _ => return null_err(),
                }
            }
            TK::New { class, fields, ctor } => {
                let mut vals = Vec::new();
                for (i, f) in fields.iter().enumerate() {
                    vals.push(match f {
                        Some(fe) => self.eval(fe, fr)?,
                        None => zero(&self.module.classes[*class].fields[i].ty),
                    });
                }
                let obj = Value::Obj(Rc::new(RefCell::new(Obj { class: *class, fields: vals })));
                if let Some((fid, args)) = ctor {
                    let mut a = vec![obj.clone()];
                    a.extend(self.eval_args(args, fr)?);
                    self.call_id(*fid, a)?;
                }
                obj
            }
            TK::Deref(_) => return rt("көрсеткіштер интерпретаторда қолжетімсіз"),
            TK::Ternary(c, a, b) => {
                if self.eval(c, fr)?.as_bool() {
                    self.eval(a, fr)?
                } else {
                    self.eval(b, fr)?
                }
            }
            TK::FStr(parts) => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        FPartT::Lit(s) => out.push_str(s),
                        FPartT::Val(x) => {
                            let v = self.eval(x, fr)?;
                            self.repr(&v, &x.ty, false, 0, &mut out);
                        }
                    }
                }
                Value::string(out)
            }
            TK::Contains { item, container, negated } => {
                let iv = self.eval(item, fr)?;
                let cv = self.eval(container, fr)?;
                let found = match &cv {
                    Value::List(l) => l.borrow().iter().any(|x| self.deep_eq(x, &iv)),
                    Value::Str(s) => s.s.contains(iv.as_str()),
                    Value::Dict(d) => match iv.key() {
                        Some(k) => d.borrow().contains(&k),
                        None => false,
                    },
                    Value::Null => {
                        if matches!(container.ty, Ty::Str) {
                            iv.as_str().is_empty()
                        } else {
                            return null_err();
                        }
                    }
                    _ => false,
                };
                Value::Bool(found != *negated)
            }
        })
    }

    fn index(&self, c: &Value, i: &Value) -> R<Value> {
        match c {
            Value::List(l) => {
                let l = l.borrow();
                let k = i.as_int();
                let idx = if k < 0 { k + l.len() as i64 } else { k };
                if idx < 0 || idx >= l.len() as i64 {
                    return idx_err(k, l.len());
                }
                Ok(l[idx as usize].clone())
            }
            Value::Str(s) => {
                let k = i.as_int();
                let idx = if k < 0 { k + s.nchars as i64 } else { k };
                if idx < 0 || idx >= s.nchars as i64 {
                    return idx_err(k, s.nchars);
                }
                Ok(Value::Str(s.slice(idx as usize, idx as usize + 1)))
            }
            Value::Dict(d) => {
                let key = match i.key() {
                    Some(k) => k,
                    None => return rt("сөздік кілті жарамсыз"),
                };
                match d.borrow().get(&key) {
                    Some(v) => Ok(v.clone()),
                    None => {
                        let shown = match &key {
                            Key::Int(n) => n.to_string(),
                            Key::Bool(b) => (*b as i64).to_string(),
                            Key::Str(s) => format!("'{}'", s),
                        };
                        rt(format!("сөздікте кілт табылмады: {}", shown))
                    }
                }
            }
            Value::Null => {
                null_err()
            }
            _ => rt("индекстеу қолданылмайды"),
        }
    }

    fn binary(&self, op: BinOp, x: Value, y: Value) -> R<Value> {
        Ok(match (x, y) {
            (Value::Int(a), Value::Int(b)) => match op {
                BinOp::Add => Value::Int(a.wrapping_add(b)),
                BinOp::Sub => Value::Int(a.wrapping_sub(b)),
                BinOp::Mul => Value::Int(a.wrapping_mul(b)),
                BinOp::FloorDiv => {
                    if b == 0 {
                        return rt("нөлге бөлу");
                    }
                    Value::Int(floor_div(a, b))
                }
                BinOp::Mod => {
                    if b == 0 {
                        return rt("нөлге бөлу");
                    }
                    Value::Int(floor_mod(a, b))
                }
                BinOp::BitAnd => Value::Int(a & b),
                BinOp::BitOr => Value::Int(a | b),
                BinOp::BitXor => Value::Int(a ^ b),
                BinOp::Shl => Value::Int(a.wrapping_shl((b & 63) as u32)),
                BinOp::Shr => Value::Int(a.wrapping_shr((b & 63) as u32)),
                _ => return rt("бүтін амал қолданылмайды"),
            },
            (Value::Float(a), Value::Float(b)) => match op {
                BinOp::Add => Value::Float(a + b),
                BinOp::Sub => Value::Float(a - b),
                BinOp::Mul => Value::Float(a * b),
                BinOp::Div => Value::Float(a / b),
                _ => return rt("нақты амал қолданылмайды"),
            },
            (Value::Bool(a), Value::Bool(b)) => match op {
                BinOp::BitAnd => Value::Bool(a & b),
                BinOp::BitOr => Value::Bool(a | b),
                BinOp::BitXor => Value::Bool(a ^ b),
                _ => return rt("логикалық амал қолданылмайды"),
            },
            (Value::Str(a), Value::Str(b)) if op == BinOp::Add => {
                let mut s = String::with_capacity(a.s.len() + b.s.len());
                s.push_str(&a.s);
                s.push_str(&b.s);
                Value::string(s)
            }
            (Value::Null, Value::Str(b)) if op == BinOp::Add => Value::Str(b),
            (Value::Str(a), Value::Null) if op == BinOp::Add => Value::Str(a),
            (Value::Str(a), Value::Int(n)) | (Value::Int(n), Value::Str(a)) if op == BinOp::Mul => {
                if n <= 0 {
                    Value::str("")
                } else if (a.s.len() as u128) * (n as u128) > (1u128 << 28) {
                    return rt("мәтінді қайталау тым үлкен");
                } else {
                    Value::string(a.s.repeat(n as usize))
                }
            }
            (Value::List(a), Value::List(b)) if op == BinOp::Add => {
                let mut v = a.borrow().clone();
                v.extend(b.borrow().iter().cloned());
                Value::list(v)
            }
            (Value::List(a), Value::Int(n)) | (Value::Int(n), Value::List(a)) if op == BinOp::Mul => {
                let a = a.borrow();
                if (a.len() as u128) * (n.max(0) as u128) > (1u128 << 24) {
                    return rt("тізімді қайталау тым үлкен");
                }
                let mut v = Vec::new();
                for _ in 0..n.max(0) {
                    v.extend(a.iter().cloned());
                }
                Value::list(v)
            }
            _ => return rt("екілік амал осы типтерге қолданылмайды"),
        })
    }

    fn compare(&self, op: CmpOp, x: &Value, y: &Value, tx: &Ty, ty: &Ty) -> bool {
        use std::cmp::Ordering::*;
        let ord = match (x, y) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => match a.partial_cmp(b) {
                Some(o) => o,
                None => return matches!(op, CmpOp::Ne),
            },
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            _ if matches!(tx, Ty::Str) && matches!(ty, Ty::Str) => x.as_str().as_bytes().cmp(y.as_str().as_bytes()),
            // сілтемелер: сәйкестік
            (Value::Null, Value::Null) => Equal,
            (Value::List(a), Value::List(b)) => if Rc::ptr_eq(a, b) { Equal } else { Less },
            (Value::Dict(a), Value::Dict(b)) => if Rc::ptr_eq(a, b) { Equal } else { Less },
            (Value::Obj(a), Value::Obj(b)) => if Rc::ptr_eq(a, b) { Equal } else { Less },
            (Value::Tuple(a), Value::Tuple(b)) => if Rc::ptr_eq(a, b) { Equal } else { Less },
            (Value::Str(a), Value::Str(b)) => if Rc::ptr_eq(a, b) { Equal } else { Less },
            (Value::Func(a), Value::Func(b)) => a.cmp(b),
            _ => Less,
        };
        match op {
            CmpOp::Eq => ord == Equal,
            CmpOp::Ne => ord != Equal,
            CmpOp::Lt => ord == Less,
            CmpOp::Le => ord != Greater,
            CmpOp::Gt => ord == Greater,
            CmpOp::Ge => ord != Less,
        }
    }

    fn deep_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Float(x), Value::Float(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Str(x), Value::Str(y)) => x.s == y.s,
            (Value::Null, Value::Null) => true,
            (Value::Null, Value::Str(s)) | (Value::Str(s), Value::Null) => s.s.is_empty(),
            (Value::List(x), Value::List(y)) => {
                if Rc::ptr_eq(x, y) {
                    return true;
                }
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| self.deep_eq(p, q))
            }
            (Value::Tuple(x), Value::Tuple(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| self.deep_eq(p, q)),
            (Value::Dict(x), Value::Dict(y)) => {
                if Rc::ptr_eq(x, y) {
                    return true;
                }
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| self.deep_eq(v, w)))
            }
            (Value::Obj(x), Value::Obj(y)) => Rc::ptr_eq(x, y),
            (Value::Func(x), Value::Func(y)) => x == y,
            _ => false,
        }
    }

    // ------------------------------------------------------------------ бейнелеу (print / мәтін / f-жол)

    fn repr(&self, v: &Value, ty: &Ty, quote: bool, depth: u32, out: &mut String) {
        if depth > 24 {
            out.push_str("...");
            return;
        }
        match ty {
            Ty::Int | Ty::Byte => {
                let _ = write!(out, "{}", v.as_int());
            }
            Ty::Float => out.push_str(&fmt_float(v.as_f64())),
            Ty::Bool => out.push_str(if v.as_bool() { "рас" } else { "жалған" }),
            Ty::Str => {
                if quote {
                    out.push('\'');
                    for c in v.as_str().chars() {
                        match c {
                            '\'' => out.push_str("\\'"),
                            '\\' => out.push_str("\\\\"),
                            '\n' => out.push_str("\\n"),
                            '\t' => out.push_str("\\t"),
                            '\r' => out.push_str("\\r"),
                            c => out.push(c),
                        }
                    }
                    out.push('\'');
                } else {
                    out.push_str(v.as_str());
                }
            }
            Ty::List(et) => match v {
                Value::List(l) => {
                    out.push('[');
                    for (i, x) in l.borrow().iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        self.repr(x, et, true, depth + 1, out);
                    }
                    out.push(']');
                }
                _ => out.push_str("жоқ"),
            },
            Ty::Tuple(ts) => match v {
                Value::Tuple(items) => {
                    out.push('(');
                    for (i, (x, t)) in items.iter().zip(ts.iter()).enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        self.repr(x, t, true, depth + 1, out);
                    }
                    if items.len() == 1 {
                        out.push(',');
                    }
                    out.push(')');
                }
                _ => out.push_str("жоқ"),
            },
            Ty::Set(et) => match v {
                Value::Dict(d) => {
                    let d = d.borrow();
                    if d.len() == 0 {
                        out.push_str("жиын()");
                        return;
                    }
                    out.push('{');
                    for (i, (k, _)) in d.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        self.repr(&k.to_value(), et, true, depth + 1, out);
                    }
                    out.push('}');
                }
                _ => out.push_str("жоқ"),
            },
            Ty::Dict(kt, vt) => match v {
                Value::Dict(d) => {
                    out.push('{');
                    for (i, (k, x)) in d.borrow().iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        self.repr(&k.to_value(), kt, true, depth + 1, out);
                        out.push_str(": ");
                        self.repr(x, vt, true, depth + 1, out);
                    }
                    out.push('}');
                }
                _ => out.push_str("жоқ"),
            },
            Ty::Class(_) => match v {
                Value::Obj(o) => {
                    let o = o.borrow();
                    let ci = &self.module.classes[o.class];
                    out.push_str(&ci.name);
                    out.push('(');
                    for (i, f) in ci.fields.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(&f.name);
                        out.push('=');
                        self.repr(&o.fields[i], &f.ty, true, depth + 1, out);
                    }
                    out.push(')');
                }
                _ => out.push_str("жоқ"),
            },
            _ => out.push_str("жоқ"),
        }
    }

    // ------------------------------------------------------------------ ішкі функциялар

    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn list_of(v: &Value) -> R<Rc<RefCell<Vec<Value>>>> {
        match v {
            Value::List(l) => Ok(Rc::clone(l)),
            _ => null_err(),
        }
    }

    fn dict_of(v: &Value) -> R<Rc<RefCell<Dict>>> {
        match v {
            Value::Dict(d) => Ok(Rc::clone(d)),
            _ => null_err(),
        }
    }

    fn builtin(&mut self, bi: &Bi, a: Vec<Value>, args: &[TExpr], rty: &Ty) -> R<Value> {
        let unavailable = || rt::<Value>("бұл функция интерпретаторда қолжетімсіз (платформа `сыртқы` функциялары арқылы жасаңыз)");
        Ok(match bi {
            Bi::Print { nl } => {
                let mut out = String::new();
                for (i, (v, e)) in a.iter().zip(args.iter()).enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    self.repr(v, &e.ty, false, 0, &mut out);
                }
                if *nl {
                    out.push('\n');
                }
                self.host.print(&out);
                Value::Null
            }
            Bi::Input => Value::str(""),
            Bi::ToStr => {
                let mut out = String::new();
                self.repr(&a[0], &args[0].ty, false, 0, &mut out);
                Value::string(out)
            }
            Bi::StrToInt => {
                let s = a[0].as_str();
                let t = s.trim_matches(|c: char| c.is_ascii() && is_ws(c as u8));
                let b = t.as_bytes();
                let mut i = 0;
                let mut neg = false;
                if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                    neg = b[i] == b'-';
                    i += 1;
                }
                let mut v: i64 = 0;
                let mut ok = i < b.len();
                while i < b.len() {
                    let c = b[i];
                    if c == b'_' && i > 0 {
                        i += 1;
                        continue;
                    }
                    if !c.is_ascii_digit() {
                        ok = false;
                        break;
                    }
                    let d = (c - b'0') as i64;
                    v = match v.checked_mul(10).and_then(|x| if neg { x.checked_sub(d) } else { x.checked_add(d) }) {
                        Some(x) => x,
                        None => {
                            ok = false;
                            break;
                        }
                    };
                    i += 1;
                }
                if !ok {
                    return rt(format!("мәтінді бүтін санға айналдыру мүмкін емес: '{}'", s));
                }
                Value::Int(v)
            }
            Bi::StrToFloat => {
                let s = a[0].as_str();
                match s.trim_matches(|c: char| c.is_ascii() && is_ws(c as u8)).parse::<f64>() {
                    Ok(v) => Value::Float(v),
                    Err(_) => return rt(format!("мәтінді нақты санға айналдыру мүмкін емес: '{}'", s)),
                }
            }
            Bi::FloatFixed(p) => Value::string(fmt_fixed(a[0].as_f64(), *p as i64)),
            Bi::StrPad(al) => {
                let s = a[0].as_str();
                let n = s.chars().count() as i64;
                let w = a[1].as_int();
                if w <= n {
                    a[0].clone()
                } else {
                    let pad = (w - n) as usize;
                    let (l, r) = match al {
                        0 => (0, pad),
                        1 => (pad, 0),
                        _ => (pad / 2, pad - pad / 2),
                    };
                    Value::string(format!("{}{}{}", " ".repeat(l), s, " ".repeat(r)))
                }
            }
            Bi::AbsI => Value::Int(a[0].as_int().wrapping_abs()),
            Bi::AbsF => Value::Float(a[0].as_f64().abs()),
            Bi::MinI => Value::Int(a[0].as_int().min(a[1].as_int())),
            Bi::MaxI => Value::Int(a[0].as_int().max(a[1].as_int())),
            Bi::MinF => {
                let (x, y) = (a[0].as_f64(), a[1].as_f64());
                Value::Float(if y < x { y } else { x })
            }
            Bi::MaxF => {
                let (x, y) = (a[0].as_f64(), a[1].as_f64());
                Value::Float(if y > x { y } else { x })
            }
            Bi::Sqrt => Value::Float(a[0].as_f64().sqrt()),
            Bi::Floor => Value::Float(a[0].as_f64().floor()),
            Bi::Ceil => Value::Float(a[0].as_f64().ceil()),
            Bi::PowI => {
                let (mut x, mut b) = (a[0].as_int(), a[1].as_int());
                if b < 0 {
                    return rt("бүтін санды теріс дәрежеге шығаруға болмайды (`нақты` қолданыңыз)");
                }
                let mut r: i64 = 1;
                while b > 0 {
                    if b & 1 == 1 {
                        r = r.wrapping_mul(x);
                    }
                    x = x.wrapping_mul(x);
                    b >>= 1;
                }
                Value::Int(r)
            }
            Bi::PowF => Value::Float(a[0].as_f64().powf(a[1].as_f64())),
            Bi::FMod => {
                let (x, y) = (a[0].as_f64(), a[1].as_f64());
                if y == 0.0 {
                    return rt("нөлге бөлу");
                }
                Value::Float(x - (x / y).floor() * y)
            }
            Bi::Len => Value::Int(match &a[0] {
                Value::Str(s) => s.nchars as i64,
                Value::List(l) => l.borrow().len() as i64,
                Value::Dict(d) => d.borrow().len() as i64,
                Value::Tuple(t) => t.len() as i64,
                Value::Null => {
                    if matches!(args[0].ty, Ty::Str) {
                        0
                    } else {
                        return null_err();
                    }
                }
                _ => 0,
            }),
            Bi::StrUpper => Value::string(map_case(a[0].as_str(), to_upper)),
            Bi::StrLower => Value::string(map_case(a[0].as_str(), to_lower)),
            Bi::StrStrip => Value::string(a[0].as_str().trim_matches(|c: char| c.is_ascii() && is_ws(c as u8)).to_string()),
            Bi::StrSplit => {
                let sep = a[1].as_str();
                if sep.is_empty() {
                    return rt("`бөлу`: бөлгіш бос болмауы керек");
                }
                Value::list(a[0].as_str().split(sep).map(Value::str).collect())
            }
            Bi::StrJoin => {
                let l = Self::list_of(&a[1])?;
                let parts: Vec<String> = l.borrow().iter().map(|v| v.as_str().to_string()).collect();
                Value::string(parts.join(a[0].as_str()))
            }
            Bi::StrFind => {
                let (h, n) = (a[0].as_str(), a[1].as_str());
                match h.find(n) {
                    Some(b) => Value::Int(h[..b].chars().count() as i64),
                    None => Value::Int(-1),
                }
            }
            Bi::StrReplace => {
                let (s, from, to) = (a[0].as_str(), a[1].as_str(), a[2].as_str());
                if from.is_empty() { a[0].clone() } else { Value::string(s.replace(from, to)) }
            }
            Bi::StrByte => {
                let s = a[0].as_str().as_bytes();
                let i = a[1].as_int();
                if i < 0 || i >= s.len() as i64 {
                    return idx_err(i, s.len());
                }
                Value::Int(s[i as usize] as i64)
            }
            Bi::StrStarts => Value::Bool(a[0].as_str().starts_with(a[1].as_str())),
            Bi::StrEnds => Value::Bool(a[0].as_str().ends_with(a[1].as_str())),
            Bi::ListPush => {
                Self::list_of(&a[0])?.borrow_mut().push(a[1].clone());
                Value::Null
            }
            Bi::ListPop => match Self::list_of(&a[0])?.borrow_mut().pop() {
                Some(v) => v,
                None => return rt("`ал`: тізім бос"),
            },
            Bi::ListClear => {
                Self::list_of(&a[0])?.borrow_mut().clear();
                Value::Null
            }
            Bi::ListReverse => {
                Self::list_of(&a[0])?.borrow_mut().reverse();
                Value::Null
            }
            Bi::ListSort => {
                let l = Self::list_of(&a[0])?;
                let mut l = l.borrow_mut();
                match &args[0].ty {
                    Ty::List(t) if matches!(**t, Ty::Float) => l.sort_by(|x, y| x.as_f64().partial_cmp(&y.as_f64()).unwrap_or(std::cmp::Ordering::Equal)),
                    Ty::List(t) if matches!(**t, Ty::Str) => l.sort_by(|x, y| x.as_str().as_bytes().cmp(y.as_str().as_bytes())),
                    _ => l.sort_by_key(|x| x.as_int()),
                }
                Value::Null
            }
            Bi::ListSortBy(kind) => {
                let l = Self::list_of(&a[0])?;
                let keys = Self::list_of(&a[1])?.borrow().clone();
                let mut l = l.borrow_mut();
                let mut pairs: Vec<(Value, Value)> = keys.into_iter().zip(l.drain(..)).collect();
                match kind {
                    0 => pairs.sort_by_key(|p| p.0.as_int()),
                    1 => pairs.sort_by(|x, y| x.0.as_f64().partial_cmp(&y.0.as_f64()).unwrap_or(std::cmp::Ordering::Equal)),
                    _ => pairs.sort_by(|x, y| x.0.as_str().as_bytes().cmp(y.0.as_str().as_bytes())),
                }
                l.extend(pairs.into_iter().map(|p| p.1));
                Value::Null
            }
            Bi::DictDel => {
                let d = Self::dict_of(&a[0])?;
                if let Some(k) = a[1].key() {
                    d.borrow_mut().remove(&k);
                }
                Value::Null
            }
            Bi::DictGetOr => {
                let d = Self::dict_of(&a[0])?;
                let r = a[1].key().and_then(|k| d.borrow().get(&k).cloned());
                r.unwrap_or_else(|| a[2].clone())
            }
            Bi::DictKeys => Value::list(Self::dict_of(&a[0])?.borrow().iter().map(|(k, _)| k.to_value()).collect()),
            Bi::DictValues => Value::list(Self::dict_of(&a[0])?.borrow().iter().map(|(_, v)| v.clone()).collect()),
            Bi::StrBytes => Value::Int(a[0].as_str().len() as i64),
            Bi::Ord => match a[0].as_str().chars().next() {
                Some(c) => Value::Int(c as i64),
                None => return rt("`код`: бос мәтін"),
            },
            Bi::Chr => {
                let n = a[0].as_int();
                match u32::try_from(n).ok().and_then(char::from_u32) {
                    Some(c) => Value::string(c.to_string()),
                    None => return rt("`таңба`: жарамсыз Unicode коды"),
                }
            }
            Bi::StrIsDigit => {
                let s = a[0].as_str();
                Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            }
            Bi::StrIsAlpha => {
                let s = a[0].as_str();
                Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic()))
            }
            Bi::StrSplitWs => Value::list(
                a[0].as_str().split(|c: char| c.is_ascii() && is_ws(c as u8)).filter(|p| !p.is_empty()).map(Value::str).collect(),
            ),
            Bi::StrCount => {
                let (h, n) = (a[0].as_str(), a[1].as_str());
                if n.is_empty() {
                    Value::Int(h.chars().count() as i64 + 1)
                } else {
                    Value::Int(h.matches(n).count() as i64)
                }
            }
            Bi::SetAdd => {
                let d = Self::dict_of(&a[0])?;
                match a[1].key() {
                    Some(k) => d.borrow_mut().insert(k, Value::Null),
                    None => return rt("жиын элементі жарамсыз"),
                }
                Value::Null
            }
            Bi::SetOp(k) => {
                let x = Self::dict_of(&a[0])?;
                let y = Self::dict_of(&a[1])?;
                let (x, y) = (x.borrow(), y.borrow());
                let mut r = Dict::new();
                match k {
                    0 => {
                        for (key, _) in x.iter().chain(y.iter()) {
                            r.insert(key.clone(), Value::Null);
                        }
                    }
                    1 => {
                        for (key, _) in x.iter() {
                            if y.contains(key) {
                                r.insert(key.clone(), Value::Null);
                            }
                        }
                    }
                    _ => {
                        for (key, _) in x.iter() {
                            if !y.contains(key) {
                                r.insert(key.clone(), Value::Null);
                            }
                        }
                    }
                }
                Value::dict(r)
            }
            Bi::SetFromList => {
                let l = Self::list_of(&a[0])?;
                let mut r = Dict::new();
                for v in l.borrow().iter() {
                    if let Some(k) = v.key() {
                        r.insert(k, Value::Null);
                    }
                }
                Value::dict(r)
            }
            Bi::Alloc(_) | Bi::Free => return unavailable(),
            Bi::Gc => Value::Null,
            Bi::GcHeap => Value::Int(0),
            Bi::Panic => return rt(a[0].as_str().to_string()),
            Bi::Args => Value::list(vec![]),
            Bi::FileRead | Bi::FileWrite(_) | Bi::FileExists | Bi::FileDelete | Bi::FileRename | Bi::MakeDir => return unavailable(),
            Bi::NetListen | Bi::NetListenLocal | Bi::NetAccept | Bi::NetConnect | Bi::NetRead | Bi::NetWrite | Bi::NetClose | Bi::NetTimeout => {
                return unavailable()
            }
            Bi::Time => Value::Float(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
            ),
            Bi::Rand => Value::Float((self.next_u64() >> 11) as f64 / 9007199254740992.0),
            Bi::RandRange => {
                let (lo, hi) = (a[0].as_int(), a[1].as_int());
                if hi < lo {
                    return rt("`кездейсоқ_аралық`: төменгі шек жоғарғыдан үлкен");
                }
                let span = (hi as i128 - lo as i128 + 1) as u128;
                Value::Int(lo.wrapping_add((self.next_u64() as u128 % span) as i64))
            }
            Bi::Seed => {
                self.rng = (a[0].as_int() as u64) ^ 0x1234_5678_9ABC_DEF1;
                if self.rng == 0 {
                    self.rng = 1;
                }
                Value::Null
            }
            Bi::Exit => return rt(format!("шығу({}) шақырылды", a[0].as_int())),
            Bi::Math(k) => {
                let x = a[0].as_f64();
                Value::Float(match k {
                    0 => x.sin(),
                    1 => x.cos(),
                    2 => x.tan(),
                    3 => x.atan(),
                    4 => x.exp(),
                    5 => x.ln(),
                    6 => x.log10(),
                    _ => x.atan2(a[1].as_f64()),
                })
            }
            Bi::ListIndex => {
                let l = Self::list_of(&a[0])?;
                let pos = l.borrow().iter().position(|v| self.deep_eq(v, &a[1]));
                Value::Int(pos.map(|p| p as i64).unwrap_or(-1))
            }
            Bi::ListRemove => {
                let l = Self::list_of(&a[0])?;
                let mut l = l.borrow_mut();
                let n = l.len();
                let k = a[1].as_int();
                let idx = if k < 0 { k + n as i64 } else { k };
                if idx < 0 || idx >= n as i64 {
                    return idx_err(k, n);
                }
                l.remove(idx as usize)
            }
            Bi::ListInsert => {
                let l = Self::list_of(&a[0])?;
                let mut l = l.borrow_mut();
                let n = l.len() as i64;
                let k = a[1].as_int();
                let idx = if k < 0 { (k + n).max(0) } else { k.min(n) };
                l.insert(idx as usize, a[2].clone());
                Value::Null
            }
            Bi::ListCopy => Value::list(Self::list_of(&a[0])?.borrow().clone()),
            Bi::ListSum | Bi::ListMin | Bi::ListMax => {
                let l = Self::list_of(&a[0])?;
                let l = l.borrow();
                let is_f = rty.is_float();
                if !matches!(bi, Bi::ListSum) && l.is_empty() {
                    return rt("`мин`/`макс`: тізім бос");
                }
                if is_f {
                    let mut acc = if matches!(bi, Bi::ListSum) { 0.0 } else { l[0].as_f64() };
                    for v in l.iter() {
                        let x = v.as_f64();
                        acc = match bi {
                            Bi::ListSum => acc + x,
                            Bi::ListMin => if x < acc { x } else { acc },
                            _ => if x > acc { x } else { acc },
                        };
                    }
                    Value::Float(acc)
                } else {
                    let mut acc = if matches!(bi, Bi::ListSum) { 0 } else { l[0].as_int() };
                    for v in l.iter() {
                        let x = v.as_int();
                        acc = match bi {
                            Bi::ListSum => acc.wrapping_add(x),
                            Bi::ListMin => acc.min(x),
                            _ => acc.max(x),
                        };
                    }
                    Value::Int(acc)
                }
            }
            Bi::IsInstance(cid) => Value::Bool(match &a[0] {
                Value::Obj(o) => self.is_subclass(o.borrow().class, *cid),
                _ => false,
            }),
            Bi::Downcast(cid) => match &a[0] {
                Value::Obj(o) => {
                    if self.is_subclass(o.borrow().class, *cid) {
                        a[0].clone()
                    } else {
                        return rt(format!("түрлендіру мүмкін емес: объект `{}` класының данасы емес", self.module.classes[*cid].name));
                    }
                }
                Value::Null => Value::Null,
                _ => return null_err(),
            },
        })
    }

    fn is_subclass(&self, mut c: ClassId, target: ClassId) -> bool {
        loop {
            if c == target {
                return true;
            }
            match self.module.classes[c].base {
                Some(b) => c = b,
                None => return false,
            }
        }
    }
}
