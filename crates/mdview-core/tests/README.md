# mdview-core tests

Run them all with `cargo test -p mdview-core` (a few seconds; no GPUI).

| File | What it checks |
| --- | --- |
| `snapshots.rs` | Every `fixtures/*.md` parsed and dumped as text, compared with `snapshots/*.snap` |
| `highlight.rs` | A token snapshot of `code/<lang>.txt` for every lexer; the language list, aliases and samples stay in sync with `src/highlight.rs`; edge cases per lexer |
| `latex.rs` | Snapshot tables (`input ⇒ output`, italics marked) for every symbol, function and accent `src/latex.rs` knows (kept in sync with the source), structures, environments and malformed input |
| `props.rs` | Property tests (proptest) with Markdown-, TeX- and code-shaped generators: no panics, all invariants, determinism, CRLF = LF, no lost text, inline styles vs a model, slugs, every highlighter language, inline math mapping |
| `common/` | Shared helpers: the dump format (`dump.rs`), the invariants (`invariants.rs`), the language list (`langs.rs`) |

## Snapshots

The dump format is described at the top of `common/dump.rs`. To add a case,
drop a new `.md` file in `fixtures/`; it becomes a test automatically. When
an output change is intended:

```sh
cargo insta test -p mdview-core --review   # or: cargo insta review
# without cargo-insta:
INSTA_UPDATE=always cargo test -p mdview-core && git diff tests/snapshots
```

A new language in `src/highlight.rs` must be added to `common/langs.rs`
(the test `language_list_matches_the_source` fails otherwise) and get a
sample in `code/<canonical name>.txt`.

Read every changed snapshot before accepting it: a snapshot records what
the parser *should* produce, not just what it does.

Fixtures are byte-exact (`fixtures/.gitattributes` disables line-ending
conversion; `crlf.md` has CRLFs, `breaks.md` and `headings.md` have
meaningful trailing whitespace), so don't let an editor "clean them up".

## Property tests

`cargo test` runs 512 cases per property. For a deeper run:
`PROPTEST_CASES=100000 cargo test -p mdview-core --release --test props`.
A failure is minimized and saved in `tests/props.proptest-regressions`;
commit that file (it replays the case first on every run) along with the
fix and an explicit regression test.
