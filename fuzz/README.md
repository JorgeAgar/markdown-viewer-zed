# Fuzzing mdview-core

Three libFuzzer targets (cargo-fuzz layout). Each checks the same
invariants as the property tests (`crates/mdview-core/tests/common/
invariants.rs`, included with `#[path]`), so a crash means either a panic
or a broken invariant.

| Target | Fuzzes |
| --- | --- |
| `parse` | `markdown::parse` (and through it the highlighter and inline LaTeX) |
| `highlight` | `highlight::highlight` for every language name |
| `latex` | `latex::to_unicode`, alone and as inline math in a paragraph |

`dict` is a dictionary of Markdown/TeX/code syntax fragments.

## With nightly and cargo-fuzz

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run parse fuzz/corpus/parse crates/mdview-core/tests/fixtures -- -dict=fuzz/dict -max_len=4096
cargo +nightly fuzz run highlight fuzz/corpus/highlight crates/mdview-core/tests/code -- -dict=fuzz/dict
cargo +nightly fuzz run latex -- -dict=fuzz/dict -max_total_time=300
```

## With stable Rust only

libFuzzer only needs SanitizerCoverage, which stable `rustc` provides
(without AddressSanitizer, which safe Rust rarely needs). From `fuzz/`:

```sh
export RUSTFLAGS="-Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4 \
  -Cllvm-args=-sanitizer-coverage-inline-8bit-counters \
  -Cllvm-args=-sanitizer-coverage-pc-table \
  -Cllvm-args=-sanitizer-coverage-trace-compares --cfg fuzzing"
# --target keeps these flags away from build scripts.
cargo build --release --target x86_64-unknown-linux-gnu --bins
mkdir -p corpus/parse artifacts
target/x86_64-unknown-linux-gnu/release/parse corpus/parse ../crates/mdview-core/tests/fixtures \
  -dict=dict -max_len=4096 -max_total_time=300 -artifact_prefix=artifacts/
```

A crashing input is written to `artifacts/`. Reproduce it by passing the
file to the target binary, then add it as a test in
`crates/mdview-core` (a fixture, or a unit test next to the fix) before
fixing. `corpus/`, `artifacts/` and `target/` are not committed.
