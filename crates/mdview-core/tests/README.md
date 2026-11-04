# mdview-core tests

Run them all with `cargo test -p mdview-core` (a few seconds; no GPUI).

| File | What it checks |
| --- | --- |
| `snapshots.rs` | Every `fixtures/*.md` parsed and dumped as text, compared with `snapshots/*.snap` |
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

Read every changed snapshot before accepting it: a snapshot records what
the parser *should* produce, not just what it does.

Fixtures are byte-exact (`fixtures/.gitattributes` disables line-ending
conversion; `crlf.md` has CRLFs, `breaks.md` and `headings.md` have
meaningful trailing whitespace), so don't let an editor "clean them up".
