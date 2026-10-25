//! Рекурсивті төмендеу парсері: токендер -> AST.

use crate::ast::*;
use crate::diag::{Diag, Pos, Res};
use crate::lexer::{self, BinTok, FPart, Kw, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
}

pub fn parse(src: &str, file: u16) -> Res<Program> {
    let toks = lexer::lex(src, file)?;
    let mut p = Parser { toks, i: 0 };
    let mut stmts = Vec::new();
    while !p.at(&Tok::Eof) {
        if p.at(&Tok::Newline) {
            p.bump();
            continue;
        }
        stmts.push(p.stmt()?);
    }
    Ok(Program { stmts })
}

fn bintok(b: BinTok) -> BinOp {
    match b {
        BinTok::Add => BinOp::Add,
        BinTok::Sub => BinOp::Sub,
        BinTok::Mul => BinOp::Mul,
        BinTok::Div => BinOp::Div,
        BinTok::FloorDiv => BinOp::FloorDiv,
        BinTok::Mod => BinOp::Mod,
        BinTok::Pow => BinOp::Pow,
        BinTok::And => BinOp::BitAnd,
        BinTok::Or => BinOp::BitOr,
        BinTok::Xor => BinOp::BitXor,
        BinTok::Shl => BinOp::Shl,
        BinTok::Shr => BinOp::Shr,
    }
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i.min(self.toks.len() - 1)].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.i + n).min(self.toks.len() - 1)].tok
    }
    fn pos(&self) -> Pos {
        self.toks[self.i.min(self.toks.len() - 1)].pos
    }
    fn at(&self, t: &Tok) -> bool {
        self.peek() == t
    }
    fn at_kw(&self, k: Kw) -> bool {
        matches!(self.peek(), Tok::Kw(x) if *x == k)
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.i.min(self.toks.len() - 1)].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: Kw) -> bool {
        if self.at_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn err<T>(&self, msg: impl Into<String>) -> Res<T> {
        Err(Diag::new(self.pos(), msg))
    }
    fn expect(&mut self, t: Tok) -> Res<Token> {
        if self.at(&t) {
            Ok(self.bump())
        } else {
            self.err(format!("{} күтілді, бірақ {} табылды", lexer::tok_name(&t), lexer::tok_name(self.peek())))
        }
    }
    fn expect_kw(&mut self, k: Kw) -> Res<Token> {
        self.expect(Tok::Kw(k))
    }
    fn ident(&mut self) -> Res<(String, Pos)> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                let p = self.pos();
                self.bump();
                Ok((s, p))
            }
            other => self.err(format!("атау күтілді, бірақ {} табылды", lexer::tok_name(&other))),
        }
    }

    // ------------------------------------------------------------ операторлар

    fn block(&mut self) -> Res<Vec<Stmt>> {
        self.expect(Tok::Colon)?;
        if self.eat(&Tok::Newline) {
            self.expect(Tok::Indent).map_err(|_| Diag::new(self.pos(), "шегіністі блок күтілді"))?;
            let mut v = Vec::new();
            loop {
                if self.at(&Tok::Newline) {
                    self.bump();
                    continue;
                }
                if self.at(&Tok::Dedent) || self.at(&Tok::Eof) {
                    break;
                }
                v.push(self.stmt()?);
            }
            self.eat(&Tok::Dedent);
            if v.is_empty() {
                return self.err("блок бос болмауы керек (`өткіз` қолданыңыз)");
            }
            Ok(v)
        } else {
            // бір жолдық блок
            Ok(vec![self.simple_stmt()?])
        }
    }

    pub fn stmt(&mut self) -> Res<Stmt> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Kw(Kw::Def) => {
                let f = self.funcdef(false)?;
                Ok(Stmt { kind: StmtKind::FuncDef(f), pos })
            }
            Tok::Kw(Kw::Extern) => {
                self.bump();
                let f = self.funcdef(true)?;
                Ok(Stmt { kind: StmtKind::FuncDef(f), pos })
            }
            Tok::Kw(Kw::Class) => self.classdef(),
            Tok::Kw(Kw::If) => self.if_stmt(),
            Tok::Kw(Kw::While) => {
                self.bump();
                let cond = self.expr()?;
                let body = self.block()?;
                Ok(Stmt { kind: StmtKind::While { cond, body }, pos })
            }
            Tok::Kw(Kw::For) => {
                self.bump();
                let mut targets = vec![self.ident()?];
                while self.eat(&Tok::Comma) {
                    targets.push(self.ident()?);
                }
                if targets.len() > 2 {
                    return Err(Diag::new(pos, "`әр` циклында ең көбі екі айнымалы болады"));
                }
                self.expect_kw(Kw::In)?;
                let iter = self.expr()?;
                let body = self.block()?;
                Ok(Stmt { kind: StmtKind::For { targets, iter, body }, pos })
            }
            Tok::Kw(Kw::Manual) => {
                self.bump();
                let body = self.block()?;
                Ok(Stmt { kind: StmtKind::Manual(body), pos })
            }
            _ => self.simple_stmt(),
        }
    }

    fn if_stmt(&mut self) -> Res<Stmt> {
        let pos = self.pos();
        self.expect_kw(Kw::If)?;
        let cond = self.expr()?;
        let then = self.block()?;
        let mut els = Vec::new();
        if self.at_kw(Kw::Else) {
            if matches!(self.peek_at(1), Tok::Kw(Kw::If)) {
                self.bump();
                els = vec![self.if_stmt()?];
            } else {
                self.bump();
                els = self.block()?;
            }
        }
        Ok(Stmt { kind: StmtKind::If { cond, then, els }, pos })
    }

    fn simple_stmt(&mut self) -> Res<Stmt> {
        let pos = self.pos();
        let first = self.simple_kind()?;
        let kind = if self.at(&Tok::Semi) {
            let mut v = vec![Stmt { kind: first, pos }];
            while self.eat(&Tok::Semi) {
                if self.at(&Tok::Newline) || self.at(&Tok::Eof) || self.at(&Tok::Dedent) {
                    break;
                }
                let p = self.pos();
                let k = self.simple_kind()?;
                v.push(Stmt { kind: k, pos: p });
            }
            StmtKind::Seq(v)
        } else {
            first
        };
        if !self.at(&Tok::Eof) && !self.at(&Tok::Dedent) {
            if !self.eat(&Tok::Newline) {
                return self.err(format!("өрнектен кейін жол соңы күтілді, бірақ {} табылды", lexer::tok_name(self.peek())));
            }
        }
        Ok(Stmt { kind, pos })
    }

    fn simple_kind(&mut self) -> Res<StmtKind> {
        let kind = match self.peek().clone() {
            Tok::Kw(Kw::Return) => {
                self.bump();
                if self.at(&Tok::Newline) || self.at(&Tok::Eof) || self.at(&Tok::Semi) {
                    StmtKind::Return(None)
                } else {
                    let first = self.expr()?;
                    if self.at(&Tok::Comma) {
                        let epos = first.pos;
                        let mut v = vec![first];
                        while self.eat(&Tok::Comma) {
                            v.push(self.expr()?);
                        }
                        StmtKind::Return(Some(Expr { kind: ExprKind::Tuple(v), pos: epos }))
                    } else {
                        StmtKind::Return(Some(first))
                    }
                }
            }
            Tok::Kw(Kw::Break) => {
                self.bump();
                StmtKind::Break
            }
            Tok::Kw(Kw::Continue) => {
                self.bump();
                StmtKind::Continue
            }
            Tok::Kw(Kw::Pass) => {
                self.bump();
                StmtKind::Pass
            }
            Tok::Kw(Kw::Global) => {
                self.bump();
                let mut names = vec![self.ident()?.0];
                while self.eat(&Tok::Comma) {
                    names.push(self.ident()?.0);
                }
                StmtKind::Global(names)
            }
            Tok::Kw(Kw::Import) => {
                self.bump();
                match self.peek().clone() {
                    Tok::Str(s) => {
                        self.bump();
                        StmtKind::Import(s)
                    }
                    other => return self.err(format!("`импорт` соңында файл жолы (жол) күтілді, бірақ {} табылды", lexer::tok_name(&other))),
                }
            }
            Tok::Kw(Kw::Assert) => {
                self.bump();
                let c = self.expr()?;
                let m = if self.eat(&Tok::Comma) { Some(self.expr()?) } else { None };
                StmtKind::Assert(c, m)
            }
            Tok::Kw(Kw::Def) | Tok::Kw(Kw::Class) | Tok::Kw(Kw::If) | Tok::Kw(Kw::While) | Tok::Kw(Kw::For) | Tok::Kw(Kw::Manual) => {
                return self.err("бұл құрылымды бір жолдық блокта қолдануға болмайды");
            }
            _ => self.expr_stmt()?,
        };
        Ok(kind)
    }

    fn expr_stmt(&mut self) -> Res<StmtKind> {
        let first = self.expr()?;
        match self.peek().clone() {
            Tok::Comma => {
                let mut targets = vec![first];
                while self.eat(&Tok::Comma) {
                    targets.push(self.expr()?);
                }
                self.expect(Tok::Assign).map_err(|_| Diag::new(self.pos(), "бірнеше өрнек үтірмен тек меншіктеуде қолданылады: `a, b = x, y`"))?;
                let mut values = vec![self.expr()?];
                while self.eat(&Tok::Comma) {
                    values.push(self.expr()?);
                }
                if values.len() != targets.len() && values.len() != 1 {
                    return self.err(format!("меншіктеуде сол жақта {} , оң жақта {} мән", targets.len(), values.len()));
                }
                Ok(StmtKind::TupleAssign { targets, values })
            }
            Tok::Assign => {
                self.bump();
                let value = self.expr()?;
                if self.at(&Tok::Assign) {
                    return self.err("тізбектелген меншіктеу (a = b = c) қолдау таппайды");
                }
                Ok(StmtKind::Assign { target: first, value })
            }
            Tok::AugAssign(op) => {
                self.bump();
                let value = self.expr()?;
                Ok(StmtKind::AugAssign { target: first, op: bintok(op), value })
            }
            Tok::Colon => {
                self.bump();
                let ty = self.type_expr()?;
                let value = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
                Ok(StmtKind::AnnAssign { target: first, ty, value })
            }
            _ => Ok(StmtKind::Expr(first)),
        }
    }

    fn type_expr(&mut self) -> Res<TypeExpr> {
        let pos = self.pos();
        if self.eat(&Tok::Star) {
            let inner = self.type_expr()?;
            return Ok(TypeExpr::Ptr(Box::new(inner), pos));
        }
        if self.eat_kw(Kw::None) {
            return Ok(TypeExpr::Name("жоқ".into(), pos));
        }
        if self.eat(&Tok::LParen) {
            let mut v = Vec::new();
            if !self.at(&Tok::RParen) {
                v.push(self.type_expr()?);
                while self.eat(&Tok::Comma) {
                    if self.at(&Tok::RParen) {
                        break;
                    }
                    v.push(self.type_expr()?);
                }
            }
            self.expect(Tok::RParen)?;
            if self.eat(&Tok::Arrow) {
                let r = self.type_expr()?;
                return Ok(TypeExpr::Func(v, Box::new(r), pos));
            }
            if v.is_empty() {
                return self.err("бос жақша тип болып табылмайды");
            }
            return Ok(TypeExpr::Tuple(v, pos));
        }
        let (name, _) = self.ident()?;
        if self.eat(&Tok::LBracket) {
            let mut args = vec![self.type_expr()?];
            while self.eat(&Tok::Comma) {
                args.push(self.type_expr()?);
            }
            self.expect(Tok::RBracket)?;
            return Ok(TypeExpr::Generic(name, args, pos));
        }
        Ok(TypeExpr::Name(name, pos))
    }

    fn funcdef(&mut self, external: bool) -> Res<FuncDef> {
        let pos = self.pos();
        self.expect_kw(Kw::Def)?;
        let (name, _) = self.ident()?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            let (pname, ppos) = self.ident()?;
            let ty = if self.eat(&Tok::Colon) { Some(self.type_expr()?) } else { None };
            let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
            params.push(Param { name: pname, ty, default, pos: ppos });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        let ret = if self.eat(&Tok::Arrow) { Some(self.type_expr()?) } else { None };
        let body = if external {
            self.eat(&Tok::Newline);
            vec![]
        } else {
            self.block()?
        };
        Ok(FuncDef { name, params, ret, body, external, pos })
    }

    fn classdef(&mut self) -> Res<Stmt> {
        let pos = self.pos();
        self.expect_kw(Kw::Class)?;
        let (name, _) = self.ident()?;
        let base = if self.eat(&Tok::LParen) {
            let b = self.ident()?;
            self.expect(Tok::RParen)?;
            Some(b)
        } else {
            None
        };
        self.expect(Tok::Colon)?;
        self.expect(Tok::Newline)?;
        self.expect(Tok::Indent).map_err(|_| Diag::new(self.pos(), "класс денесі шегіністі болуы керек"))?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        loop {
            if self.at(&Tok::Newline) {
                self.bump();
                continue;
            }
            if self.at(&Tok::Dedent) || self.at(&Tok::Eof) {
                break;
            }
            match self.peek().clone() {
                Tok::Kw(Kw::Def) => methods.push(self.funcdef(false)?),
                Tok::Kw(Kw::Pass) => {
                    self.bump();
                    self.eat(&Tok::Newline);
                }
                Tok::Str(_) => {
                    self.bump();
                    self.eat(&Tok::Newline);
                }
                Tok::Ident(fname) => {
                    let fpos = self.pos();
                    self.bump();
                    self.expect(Tok::Colon)?;
                    let ty = self.type_expr()?;
                    let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
                    if !self.at(&Tok::Dedent) {
                        self.expect(Tok::Newline)?;
                    }
                    fields.push(FieldDef { name: fname, ty, default, pos: fpos });
                }
                other => return self.err(format!("класс ішінде өріс (`аты: тип`) немесе әдіс күтілді, бірақ {} табылды", lexer::tok_name(&other))),
            }
        }
        self.eat(&Tok::Dedent);
        Ok(Stmt { kind: StmtKind::ClassDef(ClassDef { name, base, fields, methods, pos }), pos })
    }

    // ------------------------------------------------------------ өрнектер

    pub fn expr(&mut self) -> Res<Expr> {
        if self.at_kw(Kw::Lambda) {
            let pos = self.pos();
            self.bump();
            let mut params = Vec::new();
            while !self.at(&Tok::Colon) {
                params.push(self.ident()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::Colon)?;
            let body = self.expr()?;
            return Ok(Expr { kind: ExprKind::Lambda { params, body: Box::new(body) }, pos });
        }
        let e = self.or_expr()?;
        if self.at_kw(Kw::If) {
            let pos = e.pos;
            self.bump();
            let cond = self.or_expr()?;
            self.expect_kw(Kw::Else)?;
            let els = self.expr()?;
            return Ok(Expr { kind: ExprKind::Ternary { cond: Box::new(cond), then: Box::new(e), els: Box::new(els) }, pos });
        }
        Ok(e)
    }

    fn or_expr(&mut self) -> Res<Expr> {
        let mut l = self.and_expr()?;
        while self.at_kw(Kw::Or) {
            let pos = self.pos();
            self.bump();
            let r = self.and_expr()?;
            l = Expr { kind: ExprKind::Or(Box::new(l), Box::new(r)), pos };
        }
        Ok(l)
    }

    fn and_expr(&mut self) -> Res<Expr> {
        let mut l = self.not_expr()?;
        while self.at_kw(Kw::And) {
            let pos = self.pos();
            self.bump();
            let r = self.not_expr()?;
            l = Expr { kind: ExprKind::And(Box::new(l), Box::new(r)), pos };
        }
        Ok(l)
    }

    fn not_expr(&mut self) -> Res<Expr> {
        if self.at_kw(Kw::Not) {
            let pos = self.pos();
            self.bump();
            let e = self.not_expr()?;
            return Ok(Expr { kind: ExprKind::Unary(UnOp::Not, Box::new(e)), pos });
        }
        self.comparison()
    }

    fn is_simple(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Name(_) => true,
            ExprKind::Field(b, _) => Self::is_simple(b),
            ExprKind::Unary(UnOp::Neg, b) => Self::is_simple(b),
            _ => false,
        }
    }

    fn comparison(&mut self) -> Res<Expr> {
        let first = self.bitor()?;
        let mut left = first;
        let mut result: Option<Expr> = None;
        loop {
            let pos = self.pos();
            enum C {
                Cmp(CmpOp),
                In(bool),
            }
            let c = match self.peek() {
                Tok::EqEq => C::Cmp(CmpOp::Eq),
                Tok::Ne => C::Cmp(CmpOp::Ne),
                Tok::Lt => C::Cmp(CmpOp::Lt),
                Tok::Le => C::Cmp(CmpOp::Le),
                Tok::Gt => C::Cmp(CmpOp::Gt),
                Tok::Ge => C::Cmp(CmpOp::Ge),
                Tok::Kw(Kw::In) => C::In(false),
                Tok::Kw(Kw::Not) if matches!(self.peek_at(1), Tok::Kw(Kw::In)) => C::In(true),
                _ => break,
            };
            self.bump();
            if matches!(c, C::In(true)) {
                self.bump();
            }
            let right = self.bitor()?;
            if result.is_some() && !Self::is_simple(&left) {
                return Err(Diag::new(pos, "салыстыру тізбегінің ортасында күрделі өрнек болмауы керек"));
            }
            let cmp = match c {
                C::Cmp(op) => Expr { kind: ExprKind::Cmp(op, Box::new(left.clone()), Box::new(right.clone())), pos },
                C::In(neg) => Expr {
                    kind: ExprKind::Contains { item: Box::new(left.clone()), container: Box::new(right.clone()), negated: neg },
                    pos,
                },
            };
            result = Some(match result {
                None => cmp,
                Some(prev) => Expr { kind: ExprKind::And(Box::new(prev), Box::new(cmp)), pos },
            });
            left = right;
        }
        Ok(match result {
            Some(r) => r,
            None => left,
        })
    }

    fn binlevel(&mut self, next: fn(&mut Parser) -> Res<Expr>, ops: &[(Tok, BinOp)]) -> Res<Expr> {
        let mut l = next(self)?;
        loop {
            let mut found = None;
            for (t, op) in ops {
                if self.at(t) {
                    found = Some(*op);
                    break;
                }
            }
            let op = match found {
                Some(op) => op,
                None => break,
            };
            let pos = self.pos();
            self.bump();
            let r = next(self)?;
            l = Expr { kind: ExprKind::Binary(op, Box::new(l), Box::new(r)), pos };
        }
        Ok(l)
    }

    fn bitor(&mut self) -> Res<Expr> {
        self.binlevel(Parser::bitxor, &[(Tok::Pipe, BinOp::BitOr)])
    }
    fn bitxor(&mut self) -> Res<Expr> {
        self.binlevel(Parser::bitand, &[(Tok::Caret, BinOp::BitXor)])
    }
    fn bitand(&mut self) -> Res<Expr> {
        self.binlevel(Parser::shift, &[(Tok::Amp, BinOp::BitAnd)])
    }
    fn shift(&mut self) -> Res<Expr> {
        self.binlevel(Parser::arith, &[(Tok::Shl, BinOp::Shl), (Tok::Shr, BinOp::Shr)])
    }
    fn arith(&mut self) -> Res<Expr> {
        self.binlevel(Parser::term, &[(Tok::Plus, BinOp::Add), (Tok::Minus, BinOp::Sub)])
    }
    fn term(&mut self) -> Res<Expr> {
        self.binlevel(
            Parser::unary,
            &[(Tok::Star, BinOp::Mul), (Tok::Slash, BinOp::Div), (Tok::SlashSlash, BinOp::FloorDiv), (Tok::Percent, BinOp::Mod)],
        )
    }

    fn unary(&mut self) -> Res<Expr> {
        let pos = self.pos();
        let op = match self.peek() {
            Tok::Minus => Some(UnOp::Neg),
            Tok::Plus => Some(UnOp::Pos),
            Tok::Tilde => Some(UnOp::BitNot),
            Tok::Star => Some(UnOp::Deref),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.unary()?;
            // тура сан литералын бүктеу: -5
            if op == UnOp::Neg {
                if let ExprKind::Int(v) = e.kind {
                    return Ok(Expr { kind: ExprKind::Int(v.wrapping_neg()), pos });
                }
                if let ExprKind::Float(v) = e.kind {
                    return Ok(Expr { kind: ExprKind::Float(-v), pos });
                }
            }
            return Ok(Expr { kind: ExprKind::Unary(op, Box::new(e)), pos });
        }
        self.power()
    }

    fn power(&mut self) -> Res<Expr> {
        let base = self.postfix()?;
        if self.at(&Tok::StarStar) {
            let pos = self.pos();
            self.bump();
            let exp = self.unary()?;
            return Ok(Expr { kind: ExprKind::Binary(BinOp::Pow, Box::new(base), Box::new(exp)), pos });
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Res<Expr> {
        let mut e = self.atom()?;
        loop {
            let pos = self.pos();
            let epos = e.pos;
            match self.peek() {
                Tok::LParen => {
                    self.bump();
                    let mut args = Vec::new();
                    while !self.at(&Tok::RParen) {
                        // аты бар аргумент: `аты=мән`
                        let mut name = None;
                        if let Tok::Ident(n) = self.peek().clone() {
                            if matches!(self.peek_at(1), Tok::Assign) {
                                self.bump();
                                self.bump();
                                name = Some(n);
                            }
                        }
                        let value = self.expr()?;
                        args.push(Arg { name, value });
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    e = Expr { kind: ExprKind::Call(Box::new(e), args), pos: epos };
                }
                Tok::LBracket => {
                    self.bump();
                    // кесінді немесе индекс
                    let lo = if self.at(&Tok::Colon) { None } else { Some(self.expr()?) };
                    if self.eat(&Tok::Colon) {
                        let hi = if self.at(&Tok::RBracket) { None } else { Some(Box::new(self.expr()?)) };
                        self.expect(Tok::RBracket)?;
                        e = Expr { kind: ExprKind::Slice(Box::new(e), lo.map(Box::new), hi), pos: epos };
                    } else {
                        let first = lo.unwrap();
                        let idx = if self.at(&Tok::Comma) {
                            let mut v = vec![first];
                            while self.eat(&Tok::Comma) {
                                if self.at(&Tok::RBracket) {
                                    break;
                                }
                                v.push(self.expr()?);
                            }
                            Expr { kind: ExprKind::Tuple(v), pos }
                        } else {
                            first
                        };
                        self.expect(Tok::RBracket)?;
                        e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(idx)), pos: epos };
                    }
                }
                Tok::Dot => {
                    self.bump();
                    let (name, _) = self.ident()?;
                    e = Expr { kind: ExprKind::Field(Box::new(e), name), pos: epos };
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn atom(&mut self) -> Res<Expr> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Int(v), pos })
            }
            Tok::Float(v) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Float(v), pos })
            }
            Tok::Str(s) => {
                self.bump();
                // көрші жол литералдарын біріктіру
                let mut s = s;
                while let Tok::Str(n) = self.peek().clone() {
                    self.bump();
                    s.push_str(&n);
                }
                Ok(Expr { kind: ExprKind::Str(s), pos })
            }
            Tok::FStr(parts) => {
                self.bump();
                let mut out = Vec::new();
                for p in parts {
                    match p {
                        FPart::Lit(s) => out.push(FStrPart::Lit(s)),
                        FPart::Expr { src, spec, pos: epos } => {
                            let toks = lexer::lex_at(&src, epos, false)?;
                            let mut sub = Parser { toks, i: 0 };
                            let e = sub.expr()?;
                            if !sub.at(&Tok::Eof) {
                                return Err(Diag::new(epos, "f-жолдағы өрнек қате"));
                            }
                            out.push(FStrPart::Expr { expr: e, spec });
                        }
                    }
                }
                Ok(Expr { kind: ExprKind::FStr(out), pos })
            }
            Tok::Kw(Kw::True) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Bool(true), pos })
            }
            Tok::Kw(Kw::False) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Bool(false), pos })
            }
            Tok::Kw(Kw::None) => {
                self.bump();
                Ok(Expr { kind: ExprKind::None, pos })
            }
            Tok::Ident(n) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Name(n), pos })
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr()?;
                if self.at(&Tok::Comma) {
                    let mut v = vec![e];
                    while self.eat(&Tok::Comma) {
                        if self.at(&Tok::RParen) {
                            break;
                        }
                        v.push(self.expr()?);
                    }
                    self.expect(Tok::RParen)?;
                    return Ok(Expr { kind: ExprKind::Tuple(v), pos });
                }
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Tok::LBracket => {
                self.bump();
                let mut v = Vec::new();
                while !self.at(&Tok::RBracket) {
                    v.push(self.expr()?);
                    if v.len() == 1 && self.at_kw(Kw::For) {
                        // генератор: [өрнек әр x ішінде тізбек егер шарт]
                        self.bump();
                        let mut targets = vec![self.ident()?];
                        while self.eat(&Tok::Comma) {
                            targets.push(self.ident()?);
                        }
                        self.expect_kw(Kw::In)?;
                        let iter = self.or_expr()?;
                        let cond = if self.eat_kw(Kw::If) { Some(Box::new(self.or_expr()?)) } else { None };
                        self.expect(Tok::RBracket)?;
                        let elem = v.pop().unwrap();
                        return Ok(Expr { kind: ExprKind::ListComp { elem: Box::new(elem), targets, iter: Box::new(iter), cond }, pos });
                    }
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RBracket)?;
                Ok(Expr { kind: ExprKind::List(v), pos })
            }
            Tok::LBrace => {
                self.bump();
                let mut v = Vec::new();
                // жиын литералы: {a, b} (қос нүктесіз)
                if !self.at(&Tok::RBrace) {
                    let save = self.i;
                    let first = self.expr()?;
                    if !self.at(&Tok::Colon) {
                        let mut items = vec![first];
                        while self.eat(&Tok::Comma) {
                            if self.at(&Tok::RBrace) {
                                break;
                            }
                            items.push(self.expr()?);
                        }
                        self.expect(Tok::RBrace)?;
                        return Ok(Expr { kind: ExprKind::Set(items), pos });
                    }
                    self.i = save;
                }
                while !self.at(&Tok::RBrace) {
                    let k = self.expr()?;
                    self.expect(Tok::Colon)?;
                    let val = self.expr()?;
                    v.push((k, val));
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RBrace)?;
                Ok(Expr { kind: ExprKind::Dict(v), pos })
            }
            other => self.err(format!("өрнек күтілді, бірақ {} табылды", lexer::tok_name(&other))),
        }
    }
}
