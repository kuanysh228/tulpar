//! Файлдарды жүктеу: `импорт "файл.tlp"` өрнектерін ашып, бір бағдарламаға біріктіреді.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ast::*;
use crate::diag::{register_file, Diag, Pos, Res};
use crate::parser;

pub fn load(path: &str) -> Res<Program> {
    let mut seen = HashSet::new();
    let stmts = load_file(Path::new(path), &mut seen, Pos::default())?;
    Ok(Program { stmts })
}

fn load_file(path: &Path, seen: &mut HashSet<PathBuf>, at: Pos) -> Res<Vec<Stmt>> {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canon) {
        return Ok(vec![]); // бұрын қосылған
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return Err(Diag::new(at, format!("`{}` файлын оқу мүмкін емес: {}", path.display(), e))),
    };
    let id = register_file(&path.to_string_lossy(), &src);
    let prog = parser::parse(&src, id)?;
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut out = Vec::new();
    for s in prog.stmts {
        match &s.kind {
            StmtKind::Import(rel) => {
                let mut p = dir.join(rel);
                if p.extension().is_none() {
                    p.set_extension("tlp");
                }
                out.extend(load_file(&p, seen, s.pos)?);
            }
            _ => out.push(s),
        }
    }
    Ok(out)
}
