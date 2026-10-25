//! Tulpar компиляторының кітапханасы: талдау, типтерді тексеру, HIR және ендірілген интерпретатор.

pub mod ast;
pub mod check;
pub mod codegen;
pub mod diag;
pub mod hir;
pub mod inline;
pub mod interp;
pub mod ir;
pub mod lexer;
pub mod loader;
pub mod lower;
pub mod opt;
pub mod parser;
pub mod regalloc;
