//! Snapshot tests: every `tests/fixtures/*.md` is parsed and its document
//! dumped (see `common/dump.rs` for the format) and compared with
//! `tests/snapshots/snapshots__fixtures@<name>.md.snap`. Adding a fixture adds
//! a test. After an intended change: `cargo insta review` (or
//! `INSTA_UPDATE=always cargo test -p mdview-core`, then read the diff).

mod common;

use common::{dump, invariants};

#[test]
fn fixtures() {
    insta::glob!("fixtures/*.md", |path| {
        let src = std::fs::read_to_string(path).unwrap();
        let doc = invariants::parse_and_check(&src);
        insta::assert_snapshot!(dump::document(&doc));
    });
}

/// Inputs that are awkward to keep in a file (editors and git like to
/// "fix" them).
#[test]
fn inline_inputs() {
    for (name, src) in [
        ("cr_only_line_endings", "# A\rB\r\rC\r- x\r- y\r"),
        ("crlf_trailing_spaces_break", "a  \r\nb\r\n"),
        ("bom", "\u{feff}# Heading after a BOM\n"),
        ("bom_front_matter", "\u{feff}---\na: 1\n---\nText"),
        ("nul_byte", "a\0b\n\n`\0`\n"),
        ("only_hashes", "#\n##\n###"),
        ("only_markers", "-\n*\n+\n1.\n>\n"),
        ("unterminated_fence", "```rust\nfn f() {"),
        ("unterminated_math", "$$\nx^2"),
        ("unterminated_front_matter", "---\ntitle: x\n"),
        (
            "lone_specials",
            "*\n\n_\n\n`\n\n$\n\n[\n\n]\n\n|\n\n<\n\n&\n\n\\",
        ),
    ] {
        let doc = invariants::parse_and_check(src);
        insta::assert_snapshot!(name, dump::document(&doc), src);
    }
}
