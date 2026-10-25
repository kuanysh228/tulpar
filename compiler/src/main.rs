mod ast;
mod check;
mod codegen;
mod diag;
mod hir;
mod inline;
mod ir;
mod lexer;
mod loader;
mod lower;
mod opt;
mod parser;
mod regalloc;

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const USAGE: &str = "\
Tulpar — қазақ синтаксисі бар компиляцияланатын тіл (.tlp)

Қолдану:
  tulpar run   файл.tlp [аргументтер...]   компиляциялап іске қосу
  tulpar build файл.tlp [-o шығыс]         орындалатын файл жасау
  tulpar asm   файл.tlp [-o файл.s]        ассемблер кодын шығару
  tulpar check файл.tlp                    тек синтаксис пен типтерді тексеру

Опциялар:
  -O0        регистр бөлгішсіз (барлығы стекте) — түзету үшін
  --ir       IR-ды stderr-ге шығару (түзету)
";

struct Opts {
    spill_all: bool,
    dump_ir: bool,
}

fn compile_to_asm(path: &str, o: &Opts) -> Result<String, String> {
    let prog = loader::load(path).map_err(|d| d.render())?;
    let module = check::Checker::check_program(&prog).map_err(|d| d.render())?;
    let (mut funcs, cx) = lower::lower_module(&module);
    if !o.spill_all {
        for f in funcs.iter_mut() {
            opt::optimize(f);
        }
        inline::inline_all(&mut funcs);
        for f in funcs.iter_mut() {
            opt::optimize(f);
        }
    }
    if o.dump_ir {
        for f in &funcs {
            eprintln!("{:#?}", f);
        }
    }
    Ok(codegen::gen_module(&module, &funcs, &cx, o.spill_all))
}

fn find_runtime() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("TULPAR_RT") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().unwrap();
    for cand in [dir.join("libtulpar_rt.a"), dir.join("../release/libtulpar_rt.a"), dir.join("../lib/libtulpar_rt.a")] {
        if cand.exists() {
            return Ok(cand);
        }
    }
    Err("`libtulpar_rt.a` табылмады: `cargo build --release` орындаңыз немесе TULPAR_RT айнымалысын көрсетіңіз".into())
}

fn run_tool(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("`{}` іске қосылмады: {} (binutils орнатылған ба?)", what, e))?;
    if !out.status.success() {
        return Err(format!("`{}` қатесі:\n{}{}", what, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

fn build(path: &str, out: &Path, o: &Opts) -> Result<(), String> {
    let asm = compile_to_asm(path, o)?;
    let rt = find_runtime()?;
    let tmp = std::env::temp_dir().join(format!("tulpar-{}-{}", std::process::id(), out.file_name().and_then(|s| s.to_str()).unwrap_or("p")));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let s = tmp.join("prog.s");
    let obj = tmp.join("prog.o");
    std::fs::write(&s, asm).map_err(|e| e.to_string())?;
    let r = (|| {
        run_tool(Command::new("as").arg("--64").arg("-o").arg(&obj).arg(&s), "as")?;
        run_tool(
            Command::new("ld").arg("-static").arg("-z").arg("noexecstack").arg("--gc-sections").arg("-o").arg(out).arg(&obj).arg(&rt),
            "ld",
        )
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{}", USAGE);
        return;
    }
    let cmd = args[0].as_str();
    let mut file = None;
    let mut output: Option<String> = None;
    let mut opts = Opts { spill_all: false, dump_ir: false };
    let mut rest: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" if i + 1 < args.len() => {
                output = Some(args[i + 1].clone());
                i += 1;
            }
            "-O0" => opts.spill_all = true,
            "--ir" => opts.dump_ir = true,
            a => {
                if file.is_none() {
                    file = Some(a.to_string());
                } else {
                    rest.push(a.to_string());
                }
            }
        }
        i += 1;
    }
    let file = match file {
        Some(f) => f,
        None => {
            eprint!("{}", USAGE);
            exit(2);
        }
    };
    let fail = |msg: String| -> ! {
        eprint!("{}", msg);
        if !msg.ends_with('\n') {
            eprintln!();
        }
        exit(1)
    };
    match cmd {
        "check" => {
            let prog = loader::load(&file).unwrap_or_else(|d| fail(d.render()));
            check::Checker::check_program(&prog).unwrap_or_else(|d| fail(d.render()));
            println!("{}: қате жоқ", file);
        }
        "asm" => {
            let asm = compile_to_asm(&file, &opts).unwrap_or_else(|e| fail(e));
            let out = output.unwrap_or_else(|| Path::new(&file).with_extension("s").to_string_lossy().into_owned());
            std::fs::write(&out, asm).unwrap_or_else(|e| fail(format!("қате: {}", e)));
            println!("{}", out);
        }
        "build" => {
            let out = output.unwrap_or_else(|| Path::new(&file).with_extension("").to_string_lossy().into_owned());
            build(&file, Path::new(&out), &opts).unwrap_or_else(|e| fail(e));
            println!("{}", out);
        }
        "run" => {
            let out = std::env::temp_dir().join(format!("tulpar-run-{}", std::process::id()));
            build(&file, &out, &opts).unwrap_or_else(|e| fail(e));
            let status = Command::new(&out).args(&rest).status();
            let _ = std::fs::remove_file(&out);
            match status {
                Ok(s) => exit(s.code().unwrap_or(1)),
                Err(e) => fail(format!("қате: бағдарлама іске қосылмады: {}", e)),
            }
        }
        _ => {
            eprint!("{}", USAGE);
            exit(2);
        }
    }
}
