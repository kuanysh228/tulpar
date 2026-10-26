<p align="center">
  <img src="assets/banner.png" alt="Tulpar — a programming language with Kazakh syntax that compiles to native code" width="560">
</p>

# Tulpar (Тұлпар)

**English** · [Русский](README.ru.md) · [Қазақша](README.kk.md)

![License: MIT](https://img.shields.io/badge/license-MIT-green) ![Language: Tulpar](https://img.shields.io/badge/language-Tulpar_(.tlp)-e8b923) ![Platform: Linux x86-64](https://img.shields.io/badge/platform-linux--x86--64-blue) ![Written in Rust](https://img.shields.io/badge/compiler-Rust-orange)

A programming language with **Kazakh syntax** that is as simple as Python but **compiles to native machine code**.
Source files use the `.tlp` extension. The compiler and the runtime are written **from scratch**: no LLVM, no C compiler, no libc.

* static types with **type inference** (`x = 5` is a `бүтін`, an integer)
* an automatic **garbage collector**, plus optional **manual memory management** (`бөл` / `босат`)
* Python-style indentation, classes, inheritance, tuples, comprehensions, f-strings, lambdas, sets, imports
* fast: 17–140x faster than CPython on the benchmarks below, within 1.0–2.4x of C `-O2` and Rust
* error messages in Kazakh, with source line and caret; runtime errors print a stack trace with file and line

```python
анықта фиб(n: бүтін) -> бүтін:
    егер n < 2:
        қайтар n
    қайтар фиб(n - 1) + фиб(n - 2)

әр і ішінде аралық(10):
    жаз(f"фиб({і}) = {фиб(і)}")
```

## Install and run

Requirements: Linux x86-64, Rust (cargo) to build the compiler once, GNU binutils (`as`, `ld`).

```bash
cargo build --release          # compiler + runtime
./install.sh                   # optional: installs to ~/.local/bin/tulpar
tests/run_tests.sh             # test suite

tulpar run   file.tlp          # compile and run
tulpar build file.tlp -o prog  # standalone executable (static ELF, no libc)
tulpar asm   file.tlp          # show generated assembly
tulpar check file.tlp          # type-check only
```

Options: `-O0` disables optimizations and register allocation (for debugging), `--ir` dumps the IR.
After the one-time build you only need the `tulpar` binary; `cargo` is not used to compile `.tlp` programs.

## Language tour

### Keywords

| Tulpar | Python | | Tulpar | Python |
|---|---|---|---|---|
| `анықта` | `def` | | `және` / `немесе` / `емес` | `and` / `or` / `not` |
| `қайтар` | `return` | | `рас` / `жалған` / `жоқ` | `True` / `False` / `None` |
| `егер` / `әйтпесе егер` / `әйтпесе` | `if` / `elif` / `else` | | `класс` | `class` |
| `әзірше` | `while` | | `глобал` | `global` |
| `әр x ішінде …` | `for x in …` | | `тоқта` / `жалғастыр` / `өткіз` | `break` / `continue` / `pass` |
| `x ішінде y`, `x емес ішінде y` | `x in y`, `x not in y` | | `тексер` | `assert` |
| `a егер шарт әйтпесе b` | `a if cond else b` | | `қолмен:` | manual memory block |
| `лямбда x: …` | `lambda x: …` | | `импорт "file"` | include another file |

### Types

| Tulpar | meaning |
|---|---|
| `бүтін` | 64-bit signed integer (wraps on overflow) |
| `нақты` | `float64` |
| `логика` | `рас` / `жалған` (true / false) |
| `мәтін` | immutable UTF-8 string; **indexing and slicing are by character** (`ұзындық("Қазақстан") == 9`) |
| `байт` | 0..255 (for manual memory) |
| `тізім[T]`, `сөздік[K, V]`, `жиын[T]` | dynamic array, ordered hash map, set (keys/elements: `бүтін`/`мәтін`/`логика`/`байт`) |
| `(T1, T2, …)` | tuple (multiple return values) |
| `(T1) -> R` | function value |
| `Class` | user classes (reference semantics) |
| `*T` | pointer (only inside `қолмен:` blocks) |

Variables are written `x = 5`; the type is inferred from the first assignment. Empty containers need a type:
`x: тізім[бүтін] = []`. `бүтін` converts to `нақты` automatically in mixed arithmetic; use `бүтін(x)` to go the other way.

### Functions

```python
анықта қосу(a: бүтін, b: бүтін = 10) -> бүтін:     # parameter types are required, return type is inferred
    қайтар a + b

жаз(қосу(1), қосу(b=2, a=3))                      # keyword arguments, default values

анықта бөлу(a: бүтін, b: бүтін) -> (бүтін, бүтін):  # multiple return values
    қайтар a // b, a % b
q, r = бөлу(17, 5)
```

Recursive functions that return a value need `-> type`. Use `глобал x` to assign a top-level variable inside a function.

### Functions as values, lambdas

```python
анықта екі_есе(x: бүтін) -> бүтін:
    қайтар x * 2

анықта қолдан(f: (бүтін) -> бүтін, x: бүтін) -> бүтін:    # function type: (params) -> result
    қайтар f(f(x))

жаз(қолдан(екі_есе, 3), қолдан(лямбда x: x + 100, 1))     # 12 201
сөздер.сұрыпта(лямбда w: ұзындық(w))                       # stable sort by key
```

A lambda takes its type from context (`f: (бүтін) -> бүтін = лямбда x: x + 1`) and does not capture outer variables (no closures).

### Classes and inheritance

```python
класс Жануар:
    аты: мәтін
    жас: бүтін = 1                       # field default

    анықта құру(өзі, аты: мәтін):        # constructor (optional)
        өзі.аты = аты
    анықта дыбыс(өзі) -> мәтін:
        қайтар "..."

класс Ит(Жануар):
    анықта дыбыс(өзі) -> мәтін:           # virtual method (override)
        қайтар "Әу!"

үй: тізім[Жануар] = [Ит("Ақбөрі"), Жануар("Х")]
әр ж ішінде үй:
    жаз(ж.аты, ж.дыбыс())
жаз(мысал(үй[0], Ит), түрлендір(үй[0], Ит).аты)    # isinstance and checked downcast
# Жануар.құру(өзі, аты) calls the parent method directly (like super)
```

Without `құру` the constructor takes the fields in order: `Нүкте(1.0, 2.0)` or `Нүкте(y=2.0, x=1.0)`.
Single inheritance. `жоқ` (null) can be assigned to any class/list/dict/string/pointer.

### Collections

```python
a = [3, 1, 2]
a.қос(5); a.сұрыпта(); жаз(a, a[-1], a[1:3], ұзындық(a))       # [1, 2, 3, 5] 5 [2, 3] 4
d = {"бір": 1}
d["екі"] = 2
әр кілт, мән ішінде d: жаз(кілт, мән)
жаз([x * x әр x ішінде аралық(10) егер x % 2 == 0])             # comprehension
әр і, x ішінде нөмірле(a): жаз(і, x)                             # enumerate
жаз([1, 2] == [1, 2], (1, "а") == (1, "а"))                      # structural equality
```

| List | | String | | Dict |
|---|---|---|---|---|
| `.қос(x)` `.ал()` | append / pop | `.үлкен()` `.кіші()` | case (Cyrillic and Kazakh letters included) | `.жой(k)` |
| `.тазала()` `.кері()` `.сұрыпта()` | clear / reverse / sort | `.тазарт()` | strip | `.ал(k, default)` |
| `.индекс(x)` `.өшір(i)` `.кірістір(i, x)` `.көшірме()` | | `.бөлу(sep)` `.біріктір(list)` | split / join | `.кілттер()` `.мәндер()` |
| `қосынды(l)` `мин(l)` `макс(l)` | | `.тап(s)` `.ауыстыр(a, b)` | find / replace | |
| | | `.басталады(p)` `.аяқталады(p)` `.байт(i)` `.байт_саны()` | | |

Sets: `{1, 2, 3}`, `жиын[мәтін]()`, `жиын(list)`, `x ішінде s`, `.қос(x)`, `.жой(x)`, `.тізім()`, operators `|` `&` `-`.
Strings: `код("А")` (code point), `таңба(1179)`, `.сан_ма()`, `.әріп_ма()`, `.бөлу()` (split on whitespace), `.саны(s)`.

f-strings: `f"{x}"` or `ф"{x}"`; formatting: `{x:.2f}`, `{x:8}`, `{s:>10}`, `{s:<10}`, `{s:^10}`.

### Built-in functions

`жаз(...)` / `көрсет(...)` (no newline), `оқы([prompt])`, `ұзындық`, `бүтін/нақты/мәтін/логика/байт(x)`,
`абс`, `мин`, `макс`, `түбір`, `еден`, `төбе`, `синус`, `косинус`, `тангенс`, `арктангенс`, `арктангенс2`,
`экспонента`, `логарифм`, `логарифм10`, `ПИ`, `Е`, `**`,
`файл_оқы(path)`, `файл_жаз(path, text)`, `файл_қос(path, text)`, `файл_бар(path)`, `файл_жой(path)`, `файл_ауыстыр(old, new)` (atomic rename), `қалта_жаса(path)` (mkdir), `желі_күту(fd, ms)` (socket read/write timeout: `желі_оқы` then returns `""`), `аргтар()`,
`тыңда`, `тыңда_локал`, `қабылда`, `қосыл`, `желі_оқы`, `желі_жаз`, `желі_жап`, `уақыт()`, `кездейсоқ()`, `кездейсоқ_аралық(a, b)`, `тұқым(n)`, `шығу(code)`, `қате(message)`, `тексер cond, message`.

## Embedding (interpreter)

Besides compiling to native code, Tulpar can run inside another program (the library crate `tulpar`):

```bash
tulpar interp file.tlp            # run with the built-in interpreter
```

```rust
let prog = tulpar::loader::load("config/main.tlp")?;
let module = tulpar::check::Checker::check_program(&prog)?;
let mut it = tulpar::interp::Interp::new(module, std::rc::Rc::new(MyHost));   // MyHost: impl tulpar::interp::Host (call(&self, it, name, args))
it.run_main()?;                                                       // top-level code, once
let r = it.call("есепте", vec![Value::Int(5)])?;                      // call a Tulpar function from Rust
```

* `сыртқы анықта аты(a: бүтін) -> мәтін` declares a function implemented by the host (`Host::call`); programs that use it cannot be compiled to native code.
* The interpreter has no file, network or raw-memory access (`файл_*`, `тыңда`, `қолмен:` are errors), limits the number of steps (`max_steps`) and recursion depth (`max_depth`), and reports errors as `Result` with a call trace, so the host can roll back a transaction.
* The host receives `&mut Interp` in every call, so it may re-enter the interpreter (e.g. run event handlers of a record being saved).
* Host code can create and read class instances (`class_id`, `new_object`, `get_field`, `set_field`).
* Output of interpreted programs matches the compiled output (`tests/run_tests.sh` runs every test in both modes).

## Memory management

**Default: automatic (GC).** Strings, lists, dicts, classes and tuples live in the GC heap; nothing needs to be freed.
The collector is a conservative *mark-sweep* (the stack and global variables are roots) with size-class spans; it understands interior pointers.
Objects without pointers (strings, `тізім[бүтін]`, …) are not scanned.
`жина()` forces a collection, `жад()` returns the heap size in bytes.

**Manual management** is allowed only inside `қолмен:` blocks:

```python
қолмен:
    p = бөл(бүтін, 1000)       # *бүтін, zeroed, not managed by the GC
    әр і ішінде аралық(1000):
        p[і] = і * і            # p[i] or *p
    жаз(p[10], *p)
    p2 = p + 5                  # pointer arithmetic (scaled by element size)
    босат(p)                    # you must free it yourself
```

* `бөл(T, n)` allocates `n` elements of `*T`; 1 byte per element for `байт`, 8 bytes otherwise
* manual blocks **may hold references to GC objects** (`бөл(мәтін, 4)`, `бөл(Class, 4)`): the GC treats them as roots
* double free, or freeing a pointer that did not come from `бөл`, stops the program with an error

## Networking (TCP, HTTP)

```python
сервер = тыңда_локал(8080)            # listen on localhost only (тыңда(8080) = all interfaces)
әзірше рас:
    клиент = қабылда(сервер)          # wait for the next connection
    сұраныс = желі_оқы(клиент)        # one read (up to 64 KB), "" when closed
    дене = "Сәлем!"
    желі_жаз(клиент, f"HTTP/1.1 200 OK\r\nContent-Length: {дене.байт_саны()}\r\n\r\n{дене}")
    желі_жап(клиент)

с = қосыл("127.0.0.1", 8080)          # client; -1 on failure (IPv4 and localhost only, no DNS)
```

Full examples: `examples/http_server.tlp`, `examples/http_client.tlp`. No HTTPS/TLS or DNS yet; single-threaded (requests are handled one at a time).

## Multi-file programs

```python
импорт "утилиталар"        # includes утилиталар.tlp (path relative to the importing file), once
жаз(квадрат(12))
```

`импорт` splices the file in place (like an include): names are shared and top-level code runs where the import is.

## Runtime errors

The program stops with exit code `1`, a clear message and a **call stack (file, line, function)**:
index out of range, missing dict key, division by zero, null pointer (`жоқ`), stack overflow (1 GB virtual stack),
failed `тексер`, `қате("…")`, `ал` on an empty list.

```
Шақырулар тізбегі (соңғысы төменде):
  "prog.tlp", 16-жол, <main>
  "prog.tlp", 13-жол, сыртқы
  "prog.tlp", 7-жол, тереңірек
Қате: нөлдік көрсеткішке (жоқ) қатынау
```

(Small inlined functions may appear under their caller's name.)

## Writing an OS

`tulpar os build|run|iso file.tlp` compiles a program into a **bare-metal x86-64 kernel** (Multiboot: GRUB, `qemu -kernel`, ISO/USB)
instead of a Linux process. The whole language works in the kernel, including strings, lists, dicts, classes and the garbage collector,
and `жаз` prints to COM1 and the VGA text screen. Inside a `қолмен:` block there are built-ins for hardware: I/O ports
(`порт_оқы8/16/32`, `порт_жаз8/16/32`), volatile MMIO (`оқы8..64`, `жаз8..64`), `мекен(T, address)` / `мекенжай(p)` to turn numbers into
pointers and back, control/model-specific registers, `cpuid`, `hlt`/`sti`/`cli`, and interrupt handlers written in Tulpar
(`үзіліс_орнату(vector, function)`).

```bash
tulpar os run examples/os/shell.tlp        # an interactive shell kernel in QEMU (COM1 in your terminal)
tulpar os iso examples/os/timer.tlp -o os.iso
```

Details, the full list of built-ins and limitations: [docs/os.md](docs/os.md). Needs `qemu-system-x86_64` (for `run`), binutils, `grub-mkrescue` (for `iso`).

## Project layout

```
compiler/src/
  lexer.rs      indentation-aware tokens, f-strings, Unicode identifiers
  parser.rs     recursive-descent parser -> ast.rs
  check.rs      type checking/inference, AST -> hir.rs (typed tree)
  lower.rs      HIR -> ir.rs (three-address IR with virtual registers, basic blocks)
  opt.rs        CFG simplification, constants, copies, mov coalescing, DCE, if-conversion (cmov)
  inline.rs     inlining of small functions
  regalloc.rs   linear-scan register allocator (loop-depth weights, callee/caller-saved)
  codegen.rs    x86-64 (Intel syntax), System V ABI -> `as` + `ld`
runtime/src/    (no_std, no libc; raw syscalls)
  gc.rs         memory: spans, size classes, mark-sweep, manual allocator
  str.rs list.rs dict.rs fmt.rs math.rs misc.rs net.rs sys.rs (+ line-table stack traces)
kernel/         bare-metal runtime for `tulpar os` (no_std): Multiboot boot, GDT/IDT/PIC, COM1+VGA console; reuses gc/str/list/dict from runtime/
  kernel.ld     linker script (kernel at 1 MiB)
editors/vscode/ VS Code syntax highlighting for .tlp
tools/fuzz.py   differential fuzzer
tests/          *.tlp + *.out (expected output), errors/ (error tests), run_tests.sh
bench/          benchmarks (Tulpar / C / Rust / Python)
```

Object layouts: strings `[byte length][char count][bytes]`, lists `[length][capacity][data][flags]`, classes — 8-byte fields
(in a hierarchy word 0 is the class id; virtual calls go through `vtable[id]`). All 8-byte words are scanned conservatively.

## Performance

`bench/` (Linux x86-64, 8 cores). Rust is `rustc -O`, C is `gcc -O2`, Python is CPython 3.14 (`bench/bench.py`). Wall-clock time of a single run:

| benchmark | Tulpar | Rust | C `-O2` | Python 3 |
|---|---|---|---|---|
| Collatz, 3M numbers | 0.59 s | 0.34 s | 0.47 s | 33 s |
| Sieve of Eratosthenes, 2·10⁷ | 0.13 s | 0.11 s | 0.10 s | 2.2 s |
| float loop, 10⁸ | 0.13 s | 0.12 s | 0.13 s | 18 s |
| `фиб(42)` | 1.55 s | 0.65 s | 0.39 s | 36 s |

Tulpar is close to Rust/C on plain loops, ~2.4x slower on recursion and ~1.7x on branchy code: there is no recursion unrolling,
vectorization or smarter `cmov` selection yet. Optimizations: inlining, linear-scan register allocation, if-conversion (`cmov`),
constant folding, copy propagation, dead-code elimination, loop inversion.

The differential fuzzer (`python3 tools/fuzz.py 0 500`) compares random programs between `-O0`, optimized mode and a Python reference.

## Limitations

* Networking: TCP/IPv4 only, no TLS or DNS, single-threaded; Linux x86-64 only; needs `as` and `ld` (no built-in assembler/linker yet)
* `импорт` is include-like (no namespaces); lambdas do not capture variables; no user-defined generics; no exceptions (`try`)
* Integers are 64-bit (no big integers, they wrap on overflow)
* The GC is single-threaded and conservative (a false pointer can occasionally keep some memory alive)
* Strings are UTF-8; case conversion covers ASCII, Cyrillic and Kazakh letters

## License

[MIT](LICENSE)
