
use std::path::{Path, PathBuf};

use tulpar::{check, codegen, inline, loader, lower, opt};
use std::process::{exit, Command};

const USAGE: &str = "\
Tulpar — қазақ синтаксисі бар компиляцияланатын тіл (.tlp)

Қолдану:
  tulpar run   файл.tlp [аргументтер...]   компиляциялап іске қосу
  tulpar build файл.tlp [-o шығыс]         орындалатын файл жасау
  tulpar asm   файл.tlp [-o файл.s]        ассемблер кодын шығару
  tulpar check файл.tlp                    тек синтаксис пен типтерді тексеру
  tulpar interp файл.tlp                   интерпретатормен іске қосу (компиляциясыз; ендіру режимі)

Операциялық жүйе (ядро) режимі — жалаң x86-64 темірде / QEMU-да:
  tulpar os build файл.tlp [-o ядро.elf]   Multiboot ядросын жасау (GRUB немесе `qemu -kernel`)
  tulpar os run   файл.tlp [--gui] [--mem 128M]   жасап, QEMU-да іске қосу (COM1 = терминал)
  tulpar os iso   файл.tlp [-o ос.iso]     GRUB жүктелетін ISO (grub-mkrescue; VirtualBox, USB, нақты темір)
  tulpar os asm | check файл.tlp           ядро режимінде ассемблер / тексеру

Опциялар:
  -O0        регистр бөлгішсіз (барлығы стекте) — түзету үшін
  --ir       IR-ды stderr-ге шығару (түзету)
  --gui      (os run) QEMU терезесін ашу (VGA мәтін); әдепкіде тек COM1 терминалда
  --mem N    (os run) QEMU жады, мысалы 256M
";

const KERNEL_LD: &str = include_str!("../../kernel/kernel.ld");

struct Opts {
    spill_all: bool,
    dump_ir: bool,
    kernel: bool,
}

fn compile_to_asm(path: &str, o: &Opts) -> Result<String, String> {
    check::set_kernel_mode(o.kernel);
    let prog = loader::load(path).map_err(|d| d.render())?;
    let module = check::Checker::check_program(&prog).map_err(|d| d.render())?;
    if let Some(f) = module.funcs.iter().find(|f| f.external) {
        return Err(format!("қате: `сыртқы` функция `{}` тек платформа ішінде (интерпретатормен) жұмыс істейді; компиляцияланбайды\n", f.name));
    }
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
    Ok(codegen::gen_module_ex(&module, &funcs, &cx, o.spill_all, o.kernel))
}

fn find_lib(file: &str, env: &str) -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var(env) {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().unwrap();
    for cand in [dir.join(file), dir.join("../release").join(file), dir.join("../lib").join(file)] {
        if cand.exists() {
            return Ok(cand);
        }
    }
    Err(format!("`{}` табылмады: `cargo build --release` орындаңыз немесе {} айнымалысын көрсетіңіз", file, env))
}

fn find_runtime() -> Result<PathBuf, String> {
    find_lib("libtulpar_rt.a", "TULPAR_RT")
}

fn run_tool(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("`{}` іске қосылмады: {} (binutils орнатылған ба?)", what, e))?;
    if !out.status.success() {
        return Err(format!("`{}` қатесі:\n{}{}", what, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

/// Ядро: ассемблер -> `ld` (kernel.ld) -> 32 биттік ELF орауышы (QEMU `-kernel` тек ELF32 қабылдайды).
fn build_kernel(path: &str, out: &Path, o: &Opts) -> Result<(), String> {
    let asm = compile_to_asm(path, o)?;
    let rt = find_lib("libtulpar_kernel.a", "TULPAR_KERNEL_RT")?;
    let tmp = std::env::temp_dir().join(format!("tulpar-os-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let (s, obj, ld, elf64) = (tmp.join("prog.s"), tmp.join("prog.o"), tmp.join("kernel.ld"), tmp.join("kernel64.elf"));
    std::fs::write(&s, asm).map_err(|e| e.to_string())?;
    std::fs::write(&ld, KERNEL_LD).map_err(|e| e.to_string())?;
    let r = (|| {
        run_tool(Command::new("as").arg("--64").arg("-o").arg(&obj).arg(&s), "as")?;
        run_tool(
            Command::new("ld")
                .args(["-m", "elf_x86_64", "-static", "-z", "noexecstack", "-z", "max-page-size=4096", "--gc-sections", "-T"])
                .arg(&ld)
                .arg("-o")
                .arg(&elf64)
                .arg(&obj)
                .arg(&rt),
            "ld",
        )?;
        run_tool(Command::new("objcopy").args(["-O", "elf32-i386"]).arg(&elf64).arg(out), "objcopy")
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn build(path: &str, out: &Path, o: &Opts) -> Result<(), String> {
    if o.kernel {
        return build_kernel(path, out, o);
    }
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
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{}", USAGE);
        return;
    }
    let kernel = args[0] == "os";
    if kernel {
        args.remove(0);
        if args.is_empty() {
            print!("{}", USAGE);
            exit(2);
        }
    }
    let cmd = args[0].as_str();
    let mut file = None;
    let mut output: Option<String> = None;
    let mut opts = Opts { spill_all: false, dump_ir: false, kernel };
    let mut rest: Vec<String> = Vec::new();
    let mut gui = false;
    let mut mem = "128M".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" if i + 1 < args.len() => {
                output = Some(args[i + 1].clone());
                i += 1;
            }
            "-O0" => opts.spill_all = true,
            "--ir" => opts.dump_ir = true,
            "--gui" => gui = true,
            "--mem" if i + 1 < args.len() => {
                mem = args[i + 1].clone();
                i += 1;
            }
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
            check::set_kernel_mode(kernel);
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
        "interp" => {
            let prog = loader::load(&file).unwrap_or_else(|d| fail(d.render()));
            let module = check::Checker::check_program(&prog).unwrap_or_else(|d| fail(d.render()));
            let files = tulpar::diag::file_names();
            // терең рекурсия үшін үлкен стекті ағында
            let h = std::thread::Builder::new()
                .stack_size(1 << 30)
                .spawn(move || {
                    let mut it = tulpar::interp::Interp::new(module, std::rc::Rc::new(tulpar::interp::NoHost));
                    it.set_file_names(files);
                    it.max_depth = 20000;
                    it.max_steps = u64::MAX;
                    it.run_main().map_err(|e| e.render())
                })
                .unwrap();
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(msg)) => fail(msg),
                Err(_) => fail("ішкі қате: интерпретатор құлады".into()),
            }
        }
        "iso" if kernel => {
            let out = output.unwrap_or_else(|| Path::new(&file).with_extension("iso").to_string_lossy().into_owned());
            let dir = std::env::temp_dir().join(format!("tulpar-iso-{}", std::process::id()));
            let r = (|| {
                std::fs::create_dir_all(dir.join("boot/grub")).map_err(|e| e.to_string())?;
                build_kernel(&file, &dir.join("boot/kernel.elf"), &opts)?;
                std::fs::write(
                    dir.join("boot/grub/grub.cfg"),
                    "set timeout=0\nset default=0\nmenuentry \"Tulpar OS\" {\n    multiboot /boot/kernel.elf\n    boot\n}\n",
                )
                .map_err(|e| e.to_string())?;
                // Ядро Multiboot/BIOS үшін; i386-pc каталогы болса EFI бөлігі (mtools қажет) жасалмайды
                let mut g = Command::new("grub-mkrescue");
                if Path::new("/usr/lib/grub/i386-pc").is_dir() {
                    g.args(["-d", "/usr/lib/grub/i386-pc"]);
                }
                run_tool(g.arg("-o").arg(&out).arg(&dir), "grub-mkrescue")
            })();
            let _ = std::fs::remove_dir_all(&dir);
            r.unwrap_or_else(|e| fail(e));
            println!("{}", out);
        }
        "run" if kernel => {
            let out = std::env::temp_dir().join(format!("tulpar-os-run-{}.elf", std::process::id()));
            build(&file, &out, &opts).unwrap_or_else(|e| fail(e));
            let mut q = Command::new("qemu-system-x86_64");
            q.arg("-kernel").arg(&out).args(["-m", &mem, "-no-reboot", "-serial", "stdio"]);
            q.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
            if !gui {
                q.args(["-display", "none"]);
            }
            let status = q.status();
            let _ = std::fs::remove_file(&out);
            match status {
                // isa-debug-exit: QEMU коды = (код << 1) | 1
                Ok(s) => match s.code() {
                    Some(c) if c & 1 == 1 => exit(c >> 1),
                    Some(c) => exit(c),
                    None => exit(1),
                },
                Err(e) => fail(format!("қате: `qemu-system-x86_64` іске қосылмады: {} (QEMU орнатылған ба?)", e)),
            }
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
