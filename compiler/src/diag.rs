//! Қателер туралы хабарламалар және бастапқы файлдар тізілімі.

use std::cell::RefCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
    /// файл тізілімінің индексі (импорт үшін)
    pub file: u16,
}

thread_local! {
    static FILES: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Файлды тіркейді (жол, мәтін) -> индекс
pub fn register_file(path: &str, src: &str) -> u16 {
    FILES.with(|f| {
        let mut f = f.borrow_mut();
        f.push((path.to_string(), src.to_string()));
        (f.len() - 1) as u16
    })
}

pub fn file_names() -> Vec<String> {
    FILES.with(|f| f.borrow().iter().map(|(p, _)| p.clone()).collect())
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub pos: Pos,
    pub msg: String,
}

impl Diag {
    pub fn new(pos: Pos, msg: impl Into<String>) -> Diag {
        Diag { pos, msg: msg.into() }
    }

    /// Қатені тұрған жолымен және `^` көрсеткішімен шығарады.
    pub fn render(&self) -> String {
        let (file, src) = FILES.with(|f| f.borrow().get(self.pos.file as usize).cloned().unwrap_or_default());
        let mut s = format!("қате: {}\n  --> {}:{}:{}\n", self.msg, file, self.pos.line, self.pos.col);
        if self.pos.line >= 1 {
            if let Some(line) = src.lines().nth(self.pos.line as usize - 1) {
                let num = self.pos.line.to_string();
                let pad = " ".repeat(num.len());
                s.push_str(&format!("{} |\n{} | {}\n{} | ", pad, num, line, pad));
                for _ in 1..self.pos.col {
                    s.push(' ');
                }
                s.push_str("^\n");
            }
        }
        s
    }
}

pub type Res<T> = Result<T, Diag>;
