//! Лексер: UTF-8 бастапқы кодын токендерге айналдырады (Python сияқты шегіністермен).

use crate::diag::{Diag, Pos};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kw {
    Def,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Pass,
    And,
    Or,
    Not,
    True,
    False,
    None,
    Class,
    Global,
    Manual,
    Assert,
    Import,
    Lambda,
}

pub fn keyword(s: &str) -> Option<Kw> {
    Some(match s {
        "анықта" => Kw::Def,
        "қайтар" => Kw::Return,
        "егер" => Kw::If,
        "әйтпесе" => Kw::Else,
        "әзірше" => Kw::While,
        "әр" => Kw::For,
        "ішінде" => Kw::In,
        "тоқта" => Kw::Break,
        "жалғастыр" => Kw::Continue,
        "өткіз" => Kw::Pass,
        "және" => Kw::And,
        "немесе" => Kw::Or,
        "емес" => Kw::Not,
        "рас" => Kw::True,
        "жалған" => Kw::False,
        "жоқ" => Kw::None,
        "класс" => Kw::Class,
        "глобал" => Kw::Global,
        "қолмен" => Kw::Manual,
        "тексер" => Kw::Assert,
        "импорт" => Kw::Import,
        "лямбда" => Kw::Lambda,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum FPart {
    Lit(String),
    Expr { src: String, spec: Option<String>, pos: Pos },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinTok {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    FStr(Vec<FPart>),
    Ident(String),
    Kw(Kw),
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Dot,
    Arrow,
    Plus,
    Minus,
    Star,
    StarStar,
    Slash,
    SlashSlash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    Ne,
    Assign,
    AugAssign(BinTok),
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
}

pub fn tok_name(t: &Tok) -> String {
    match t {
        Tok::Int(_) | Tok::Float(_) => "сан".into(),
        Tok::Str(_) | Tok::FStr(_) => "жол".into(),
        Tok::Ident(s) => format!("`{}`", s),
        Tok::Kw(k) => format!("`{}`", kw_text(*k)),
        Tok::LParen => "`(`".into(),
        Tok::RParen => "`)`".into(),
        Tok::LBracket => "`[`".into(),
        Tok::RBracket => "`]`".into(),
        Tok::LBrace => "`{`".into(),
        Tok::RBrace => "`}`".into(),
        Tok::Comma => "`,`".into(),
        Tok::Colon => "`:`".into(),
        Tok::Dot => "`.`".into(),
        Tok::Arrow => "`->`".into(),
        Tok::Assign => "`=`".into(),
        Tok::Newline => "жол соңы".into(),
        Tok::Indent => "шегініс".into(),
        Tok::Dedent => "шегіністің аяқталуы".into(),
        Tok::Eof => "файл соңы".into(),
        other => format!("{:?}", other),
    }
}

pub fn kw_text(k: Kw) -> &'static str {
    match k {
        Kw::Def => "анықта",
        Kw::Return => "қайтар",
        Kw::If => "егер",
        Kw::Else => "әйтпесе",
        Kw::While => "әзірше",
        Kw::For => "әр",
        Kw::In => "ішінде",
        Kw::Break => "тоқта",
        Kw::Continue => "жалғастыр",
        Kw::Pass => "өткіз",
        Kw::And => "және",
        Kw::Or => "немесе",
        Kw::Not => "емес",
        Kw::True => "рас",
        Kw::False => "жалған",
        Kw::None => "жоқ",
        Kw::Class => "класс",
        Kw::Global => "глобал",
        Kw::Manual => "қолмен",
        Kw::Assert => "тексер",
        Kw::Import => "импорт",
        Kw::Lambda => "лямбда",
    }
}

pub struct Lexer<'a> {
    src: &'a [char],
    i: usize,
    line: u32,
    col: u32,
    depth: i32,
    indents: Vec<u32>,
    out: Vec<Token>,
    at_line_start: bool,
    base: Pos,
    file: u16,
}

pub fn lex(src: &str, file: u16) -> Result<Vec<Token>, Diag> {
    lex_at(src, Pos { line: 1, col: 1, file }, true)
}

/// `top` = false — f-жолдың ішіндегі өрнек үшін (шегініссіз, `Newline` жоқ).
pub fn lex_at(src: &str, base: Pos, top: bool) -> Result<Vec<Token>, Diag> {
    let chars: Vec<char> = src.chars().collect();
    let mut lx = Lexer {
        src: &chars,
        i: 0,
        line: base.line,
        col: base.col,
        depth: if top { 0 } else { 1 },
        indents: vec![0],
        out: Vec::new(),
        at_line_start: top,
        base,
        file: base.file,
    };
    lx.run(top)?;
    Ok(lx.out)
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn is_ident_cont(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

impl<'a> Lexer<'a> {
    fn pos(&self) -> Pos {
        Pos { line: self.line, col: self.col, file: self.file }
    }
    fn peek(&self) -> Option<char> {
        self.src.get(self.i).copied()
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        self.src.get(self.i + n).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.src.get(self.i).copied()?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }
    fn push(&mut self, tok: Tok, pos: Pos) {
        self.out.push(Token { tok, pos });
    }
    fn err<T>(&self, pos: Pos, msg: impl Into<String>) -> Result<T, Diag> {
        Err(Diag::new(pos, msg))
    }

    fn run(&mut self, top: bool) -> Result<(), Diag> {
        let _ = self.base;
        loop {
            if self.at_line_start && self.depth == 0 {
                // шегіністі өлшеу
                let mut width = 0u32;
                loop {
                    match self.peek() {
                        Some(' ') => {
                            width += 1;
                            self.bump();
                        }
                        Some('\t') => {
                            return self.err(self.pos(), "табуляция қолдау таппайды, бос орын (пробел) қолданыңыз");
                        }
                        Some('\r') => {
                            self.bump();
                        }
                        _ => break,
                    }
                }
                match self.peek() {
                    None => break,
                    Some('\n') => {
                        self.bump();
                        continue;
                    }
                    Some('#') => {
                        while let Some(c) = self.peek() {
                            if c == '\n' {
                                break;
                            }
                            self.bump();
                        }
                        continue;
                    }
                    _ => {}
                }
                let pos = self.pos();
                let cur = *self.indents.last().unwrap();
                if width > cur {
                    self.indents.push(width);
                    self.push(Tok::Indent, pos);
                } else if width < cur {
                    while width < *self.indents.last().unwrap() {
                        self.indents.pop();
                        self.push(Tok::Dedent, pos);
                    }
                    if width != *self.indents.last().unwrap() {
                        return self.err(pos, "шегініс алдыңғы деңгейлермен сәйкес келмейді");
                    }
                }
                self.at_line_start = false;
            }

            let c = match self.peek() {
                Some(c) => c,
                None => break,
            };
            let pos = self.pos();
            match c {
                ' ' | '\r' => {
                    self.bump();
                }
                '\t' => {
                    return self.err(pos, "табуляция қолдау таппайды, бос орын (пробел) қолданыңыз");
                }
                '\n' => {
                    self.bump();
                    if self.depth == 0 {
                        if !matches!(self.out.last().map(|t| &t.tok), Some(Tok::Newline) | None) {
                            self.push(Tok::Newline, pos);
                        }
                        self.at_line_start = true;
                    }
                }
                '#' => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                '\\' if self.peek_at(1) == Some('\n') => {
                    self.bump();
                    self.bump();
                }
                '"' | '\'' => {
                    let s = self.string(c, pos, false)?;
                    self.push(s, pos);
                }
                c if c.is_ascii_digit() || (c == '.' && self.peek_at(1).map_or(false, |d| d.is_ascii_digit())) => {
                    let t = self.number(pos)?;
                    self.push(t, pos);
                }
                c if is_ident_start(c) => {
                    // f-жол: ф"..." немесе f"..."
                    if (c == 'ф' || c == 'f') && matches!(self.peek_at(1), Some('"') | Some('\'')) {
                        self.bump();
                        let q = self.peek().unwrap();
                        let s = self.string(q, pos, true)?;
                        self.push(s, pos);
                        continue;
                    }
                    let mut s = String::new();
                    while let Some(c) = self.peek() {
                        if is_ident_cont(c) {
                            s.push(c);
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    match keyword(&s) {
                        Some(k) => self.push(Tok::Kw(k), pos),
                        None => self.push(Tok::Ident(s), pos),
                    }
                }
                _ => {
                    self.bump();
                    let t = self.punct(c, pos)?;
                    self.push(t, pos);
                }
            }
        }
        if top {
            if !matches!(self.out.last().map(|t| &t.tok), Some(Tok::Newline) | None) {
                let p = self.pos();
                self.push(Tok::Newline, p);
            }
            let p = self.pos();
            while self.indents.len() > 1 {
                self.indents.pop();
                self.push(Tok::Dedent, p);
            }
        }
        let p = self.pos();
        self.push(Tok::Eof, p);
        Ok(())
    }

    fn punct(&mut self, c: char, pos: Pos) -> Result<Tok, Diag> {
        let n = self.peek();
        let aug = |lx: &mut Lexer, op: BinTok| {
            lx.bump();
            Tok::AugAssign(op)
        };
        Ok(match c {
            '(' => {
                self.depth += 1;
                Tok::LParen
            }
            ')' => {
                self.depth -= 1;
                Tok::RParen
            }
            '[' => {
                self.depth += 1;
                Tok::LBracket
            }
            ']' => {
                self.depth -= 1;
                Tok::RBracket
            }
            '{' => {
                self.depth += 1;
                Tok::LBrace
            }
            '}' => {
                self.depth -= 1;
                Tok::RBrace
            }
            ',' => Tok::Comma,
            ':' => Tok::Colon,
            ';' => Tok::Semi,
            '.' => Tok::Dot,
            '~' => Tok::Tilde,
            '+' => {
                if n == Some('=') {
                    aug(self, BinTok::Add)
                } else {
                    Tok::Plus
                }
            }
            '-' => match n {
                Some('=') => aug(self, BinTok::Sub),
                Some('>') => {
                    self.bump();
                    Tok::Arrow
                }
                _ => Tok::Minus,
            },
            '*' => match n {
                Some('*') => {
                    self.bump();
                    if self.peek() == Some('=') {
                        aug(self, BinTok::Pow)
                    } else {
                        Tok::StarStar
                    }
                }
                Some('=') => aug(self, BinTok::Mul),
                _ => Tok::Star,
            },
            '/' => match n {
                Some('/') => {
                    self.bump();
                    if self.peek() == Some('=') {
                        aug(self, BinTok::FloorDiv)
                    } else {
                        Tok::SlashSlash
                    }
                }
                Some('=') => aug(self, BinTok::Div),
                _ => Tok::Slash,
            },
            '%' => {
                if n == Some('=') {
                    aug(self, BinTok::Mod)
                } else {
                    Tok::Percent
                }
            }
            '&' => {
                if n == Some('=') {
                    aug(self, BinTok::And)
                } else {
                    Tok::Amp
                }
            }
            '|' => {
                if n == Some('=') {
                    aug(self, BinTok::Or)
                } else {
                    Tok::Pipe
                }
            }
            '^' => {
                if n == Some('=') {
                    aug(self, BinTok::Xor)
                } else {
                    Tok::Caret
                }
            }
            '<' => match n {
                Some('<') => {
                    self.bump();
                    if self.peek() == Some('=') {
                        aug(self, BinTok::Shl)
                    } else {
                        Tok::Shl
                    }
                }
                Some('=') => {
                    self.bump();
                    Tok::Le
                }
                _ => Tok::Lt,
            },
            '>' => match n {
                Some('>') => {
                    self.bump();
                    if self.peek() == Some('=') {
                        aug(self, BinTok::Shr)
                    } else {
                        Tok::Shr
                    }
                }
                Some('=') => {
                    self.bump();
                    Tok::Ge
                }
                _ => Tok::Gt,
            },
            '=' => {
                if n == Some('=') {
                    self.bump();
                    Tok::EqEq
                } else {
                    Tok::Assign
                }
            }
            '!' => {
                if n == Some('=') {
                    self.bump();
                    Tok::Ne
                } else {
                    return self.err(pos, "`!` белгісі жарамсыз, `емес` қолданыңыз");
                }
            }
            other => return self.err(pos, format!("белгісіз таңба `{}`", other)),
        })
    }

    fn number(&mut self, pos: Pos) -> Result<Tok, Diag> {
        // 0x / 0b / 0o
        if self.peek() == Some('0') {
            let radix = match self.peek_at(1) {
                Some('x') | Some('X') => 16,
                Some('b') | Some('B') => 2,
                Some('o') | Some('O') => 8,
                _ => 0,
            };
            if radix != 0 {
                self.bump();
                self.bump();
                let mut s = String::new();
                while let Some(c) = self.peek() {
                    if c == '_' {
                        self.bump();
                    } else if c.is_digit(radix) {
                        s.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
                if s.is_empty() {
                    return self.err(pos, "сан литералы толық емес");
                }
                return match u64::from_str_radix(&s, radix) {
                    Ok(v) => Ok(Tok::Int(v as i64)),
                    Err(_) => self.err(pos, "сан тым үлкен"),
                };
            }
        }
        let mut s = String::new();
        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c == '_' {
                self.bump();
            } else if c.is_ascii_digit() {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if self.peek() == Some('.') && self.peek_at(1).map_or(true, |d| !is_ident_start(d) && d != '.') {
            is_float = true;
            s.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c == '_' {
                    self.bump();
                } else if c.is_ascii_digit() {
                    s.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some('e') | Some('E'))
            && (self.peek_at(1).map_or(false, |d| d.is_ascii_digit())
                || (matches!(self.peek_at(1), Some('+') | Some('-')) && self.peek_at(2).map_or(false, |d| d.is_ascii_digit())))
        {
            is_float = true;
            s.push('e');
            self.bump();
            if let Some(c @ ('+' | '-')) = self.peek() {
                s.push(c);
                self.bump();
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if is_float {
            if s.starts_with('.') {
                s.insert(0, '0');
            }
            match s.parse::<f64>() {
                Ok(v) => Ok(Tok::Float(v)),
                Err(_) => self.err(pos, "нақты сан литералы қате"),
            }
        } else {
            match s.parse::<u64>() {
                Ok(v) if v <= i64::MAX as u64 + 1 => Ok(Tok::Int(v as i64)),
                _ => self.err(pos, "бүтін сан тым үлкен"),
            }
        }
    }

    fn escape(&mut self, pos: Pos) -> Result<char, Diag> {
        let c = match self.bump() {
            Some(c) => c,
            None => return self.err(pos, "жол аяқталмаған"),
        };
        Ok(match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '0' => '\0',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            'x' => {
                let h: String = [self.bump(), self.bump()].iter().flatten().collect();
                match u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    Some(c) => c,
                    None => return self.err(pos, "`\\x` қатесі"),
                }
            }
            'u' => {
                let mut h = String::new();
                for _ in 0..4 {
                    if let Some(d) = self.bump() {
                        h.push(d);
                    }
                }
                match u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    Some(c) => c,
                    None => return self.err(pos, "`\\u` қатесі"),
                }
            }
            other => return self.err(pos, format!("белгісіз escape `\\{}`", other)),
        })
    }

    fn string(&mut self, q: char, pos: Pos, is_f: bool) -> Result<Tok, Diag> {
        self.bump(); // ашатын тырнақша
        let triple = self.peek() == Some(q) && self.peek_at(1) == Some(q);
        if triple {
            self.bump();
            self.bump();
        }
        // Екі режим: қарапайым жол және f-жол.
        let mut parts: Vec<FPart> = Vec::new();
        let mut cur = String::new();
        loop {
            let c = match self.peek() {
                Some(c) => c,
                None => return self.err(pos, "жол жабылмаған"),
            };
            if c == q {
                if triple {
                    if self.peek_at(1) == Some(q) && self.peek_at(2) == Some(q) {
                        self.bump();
                        self.bump();
                        self.bump();
                        break;
                    }
                    self.bump();
                    cur.push(c);
                    continue;
                } else {
                    self.bump();
                    break;
                }
            }
            if c == '\n' && !triple {
                return self.err(pos, "жол жабылмаған");
            }
            if c == '\\' {
                self.bump();
                let e = self.escape(pos)?;
                cur.push(e);
                continue;
            }
            if is_f && c == '{' {
                if self.peek_at(1) == Some('{') {
                    self.bump();
                    self.bump();
                    cur.push('{');
                    continue;
                }
                self.bump();
                let epos = self.pos();
                // өрнек мәтінін `}` дейін жинау
                let mut depth = 0i32;
                let mut expr = String::new();
                let mut spec: Option<String> = None;
                loop {
                    let d = match self.bump() {
                        Some(d) => d,
                        None => return self.err(pos, "f-жолдағы `{` жабылмаған"),
                    };
                    match d {
                        '(' | '[' | '{' => {
                            depth += 1;
                            expr.push(d);
                        }
                        ')' | ']' => {
                            depth -= 1;
                            expr.push(d);
                        }
                        '}' => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                            expr.push(d);
                        }
                        '"' | '\'' if d != q || triple => {
                            expr.push(d);
                            loop {
                                match self.bump() {
                                    Some(e) if e == d => {
                                        expr.push(e);
                                        break;
                                    }
                                    Some(e) => expr.push(e),
                                    None => return self.err(pos, "f-жол жабылмаған"),
                                }
                            }
                        }
                        ':' if depth == 0 => {
                            let mut sp = String::new();
                            loop {
                                match self.bump() {
                                    Some('}') => break,
                                    Some(e) => sp.push(e),
                                    None => return self.err(pos, "f-жолдағы `{` жабылмаған"),
                                }
                            }
                            spec = Some(sp);
                            break;
                        }
                        _ => expr.push(d),
                    }
                }
                if !cur.is_empty() {
                    parts.push(FPart::Lit(std::mem::take(&mut cur)));
                }
                if expr.trim().is_empty() {
                    return self.err(epos, "f-жолда бос өрнек");
                }
                parts.push(FPart::Expr { src: expr, spec, pos: epos });
                continue;
            }
            if is_f && c == '}' {
                if self.peek_at(1) == Some('}') {
                    self.bump();
                    self.bump();
                    cur.push('}');
                    continue;
                }
                return self.err(self.pos(), "f-жолда жалғыз `}`; `}}` деп жазыңыз");
            }
            self.bump();
            cur.push(c);
        }
        if is_f {
            if !cur.is_empty() || parts.is_empty() {
                parts.push(FPart::Lit(cur));
            }
            Ok(Tok::FStr(parts))
        } else {
            Ok(Tok::Str(cur))
        }
    }
}
