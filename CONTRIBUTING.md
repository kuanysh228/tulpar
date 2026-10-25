# Contributing to Tulpar

Thanks for your interest! Tulpar is a compiled language with Kazakh syntax (`.tlp`).

## Build and test

Requirements: Linux x86-64, Rust (stable), GNU binutils (`as`, `ld`).

```bash
cargo build --release
tests/run_tests.sh                 # every test runs optimized and with -O0
python3 tools/fuzz.py 0 200        # differential fuzzer (random programs vs. a Python reference)
```

Compiler warnings are treated as bugs: `cargo build --release` must print none.

## Where things are

See "Project layout" in the README. In short: `compiler/src` is the compiler pipeline
(lexer → parser → type checker → IR → optimizer → register allocator → x86-64), `runtime/src` is the libc-free runtime
(GC, strings, collections, TCP), `tests/` holds `.tlp` programs with expected `.out` files and `tests/errors/` with
programs that must fail (the first line `# ШЫҒЫС: <text>` is the expected message fragment).

## Guidelines

* Add a test with every language feature or bug fix: a `tests/<name>.tlp` plus its expected `tests/<name>.out`
  (review the output by hand before saving it), or a `tests/errors/*.tlp` for diagnostics.
* Compiler and runtime messages are in Kazakh; keep that.
* Optimizer changes must keep `tests/run_tests.sh` and the fuzzer green.
* Keep the README in all three languages (English, Russian, Kazakh) consistent.

By contributing you agree that your contribution is licensed under the MIT license.
