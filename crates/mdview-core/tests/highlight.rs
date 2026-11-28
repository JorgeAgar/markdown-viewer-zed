//! Syntax highlighter tests: a token snapshot of a realistic sample for every
//! lexer (`tests/code/<lang>.txt`, named after a language from
//! `common/langs.rs`), plus checks that the language list, the aliases and
//! the samples stay in sync with `src/highlight.rs`.

mod common;

use std::{collections::BTreeSet, path::Path};

use common::{
    dump,
    invariants::{check_highlights, highlight_and_check},
    langs::{LANG_GROUPS, all_langs},
};
use mdview_core::highlight::{Highlight, Tok, highlight};

fn sample(lang: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/code/{lang}.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `(text, token)` pairs, for exact assertions.
fn toks(lang: &str, code: &str) -> Vec<(String, Tok)> {
    highlight_and_check(lang, code)
        .into_iter()
        .map(|h| (code[h.range].to_string(), h.tok))
        .collect()
}

fn t(s: &str, tok: Tok) -> (String, Tok) {
    (s.to_string(), tok)
}

#[test]
fn samples() {
    insta::glob!("code/*.txt", |path| {
        let lang = path.file_stem().unwrap().to_str().unwrap();
        assert!(
            all_langs().any(|l| l == lang),
            "tests/code/{lang}.txt: not a language in common/langs.rs"
        );
        let code = std::fs::read_to_string(path).unwrap();
        let hs = highlight_and_check(lang, &code);
        let mut out = String::new();
        dump::source(&mut out, &dump::tokens(&code, &hs), 0);
        insta::assert_snapshot!(out);
    });
}

/// Every name the `match`es in `src/highlight.rs` accept is in
/// `common/langs.rs` and vice versa, so a new language can't go untested.
#[test]
fn language_list_matches_the_source() {
    let src = include_str!("../src/highlight.rs");
    let section = |from: &str, to: &str| {
        let start = src
            .find(from)
            .unwrap_or_else(|| panic!("{from:?} not found"));
        let end = start + src[start..].find(to).unwrap();
        &src[start..end]
    };
    let mut in_source = BTreeSet::new();
    for part in [
        section("pub fn highlight(", "_ => {"),
        section("    Some(match lang {", "_ => return None"),
    ] {
        // Every string literal in the match arms.
        let mut rest = part;
        while let Some(i) = rest.find('"') {
            let end = rest[i + 1..].find('"').unwrap();
            in_source.insert(rest[i + 1..i + 1 + end].to_string());
            rest = &rest[i + end + 2..];
        }
    }
    let listed: BTreeSet<String> = all_langs().map(String::from).collect();
    assert_eq!(
        listed.len(),
        all_langs().count(),
        "duplicate names in langs.rs"
    );
    assert_eq!(listed, in_source);
}

#[test]
fn every_lexer_has_a_sample() {
    for group in LANG_GROUPS {
        let code = sample(group[0]);
        assert!(
            !highlight_and_check(group[0], &code).is_empty(),
            "{}: sample has no highlights",
            group[0]
        );
    }
}

/// Aliases (and any capitalization) behave exactly like the canonical name.
#[test]
fn aliases_match_their_canonical_language() {
    for group in LANG_GROUPS {
        let code = sample(group[0]);
        let expected = highlight(group[0], &code);
        for lang in group.iter().copied() {
            for name in [lang.to_string(), lang.to_uppercase(), capitalize(lang)] {
                assert_eq!(
                    highlight(&name, &code),
                    expected,
                    "{name:?} differs from {:?}",
                    group[0]
                );
            }
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Lexers differ: each canonical language highlights its own sample
/// differently from at least one other lexer (catches an alias pointing at
/// the wrong `Lang`).
#[test]
fn lexers_are_distinct() {
    for (i, a) in LANG_GROUPS.iter().enumerate() {
        for b in &LANG_GROUPS[i + 1..] {
            let code = sample(a[0]);
            let ha = highlight(a[0], &code);
            let hb = highlight(b[0], &code);
            let code_b = sample(b[0]);
            let same_b = highlight(a[0], &code_b) == highlight(b[0], &code_b);
            assert!(
                ha != hb || !same_b,
                "{} and {} highlight both samples identically",
                a[0],
                b[0]
            );
        }
    }
}

#[test]
fn unknown_and_empty() {
    for lang in ["", "text", "plain", "brainfuck", "rust2", " rust", "md"] {
        assert_eq!(highlight(lang, "fn main() { 1 } // x"), [], "{lang:?}");
    }
    for lang in all_langs() {
        assert_eq!(highlight(lang, ""), [], "{lang}");
        for code in ["\n", " ", "\n\n\t \r\n", "é", "日本語", "\u{200b}"] {
            highlight_and_check(lang, code);
        }
    }
}

/// Inputs that end in the middle of a construct must not run past the end
/// of the code or split a character.
#[test]
fn truncated_constructs() {
    let cases = [
        "\"",
        "'",
        "`",
        "\"\\",
        "'\\",
        "/*",
        "/* é",
        "--[[",
        "{-",
        "<!--",
        "<",
        "<a",
        "<a b=\"",
        "<a b='",
        "&",
        "&amp",
        "#",
        "#[",
        "#![",
        "#!",
        "r#\"",
        "br\"",
        "r#",
        "'a",
        "'\\",
        "'é",
        "$",
        "${",
        "$(",
        "$((",
        "@",
        "0x",
        "1e",
        "1e+",
        "1.",
        ".5",
        "\\",
        "a\\",
        "\"\"\"",
        "'''",
        "\"\"\"é",
        "[",
        "[a",
        "- ",
        "key:",
        "key =",
        "&a",
        "*a",
        "---",
        "...",
        "x = \"é",
        "<p a=\"é",
        "#!/é",
        "é\\",
        "\\é",
        "'é'",
        "r\"é",
        "-é",
        "@é",
        "$é",
    ];
    for lang in all_langs() {
        for code in cases {
            highlight_and_check(lang, code);
            // And with a multi-byte character right after.
            highlight_and_check(lang, &format!("{code}é"));
            highlight_and_check(lang, &format!("{code}\n日"));
        }
    }
}

#[test]
fn rust_details() {
    let code = "let r = 0..10; let f = 1.5; x.0.1; 1e10 1e-3 0x1e+5 2E+3 a!= b println!(\"\") Vec::<u8> 'a' '\\n' 'ab 'é' r\"raw\" r##\"a\"#b\"## br\"b\" #[a[b]] #![x] self Some None";
    let got = toks("rust", code);
    // `0..10` is a range, `0x1e+5` hex plus 5. (`x.0.1`, a tuple field
    // access, comes out as the number `0.1`: known and harmless.)
    assert_eq!(
        got[..11],
        [
            t("let", Tok::Keyword),
            t("0", Tok::Number),
            t("10", Tok::Number),
            t("let", Tok::Keyword),
            t("1.5", Tok::Number),
            t("0.1", Tok::Number),
            t("1e10", Tok::Number),
            t("1e-3", Tok::Number),
            t("0x1e", Tok::Number),
            t("5", Tok::Number),
            t("2E+3", Tok::Number),
        ],
        "{got:#?}"
    );
    for want in [
        t("println!", Tok::Attr),
        t("\"\"", Tok::String),
        // The turbofish makes `Vec` a function name, like `collect::<T>`.
        t("Vec", Tok::Function),
        t("u8", Tok::Type),
        t("'a'", Tok::String),
        t("'\\n'", Tok::String),
        t("'ab", Tok::Attr),
        t("'é'", Tok::String),
        t("r\"raw\"", Tok::String),
        t("r##\"a\"#b\"##", Tok::String),
        t("br\"b\"", Tok::String),
        t("#[a[b]]", Tok::Attr),
        t("#![x]", Tok::Attr),
        t("self", Tok::Constant),
        t("Some", Tok::Constant),
        t("None", Tok::Constant),
    ] {
        assert!(got.contains(&want), "missing {want:?} in {got:#?}");
    }
    // `a!=` is a comparison, not a macro.
    assert!(!got.iter().any(|(s, _)| s.starts_with("a!")), "{got:#?}");
}

/// An attribute is `#[...]` or `#![...]` and ends at the `]` that balances
/// its first `[` (#43). Brackets in strings, character literals and
/// comments don't count; `#!` without `[` is only a shebang at the start.
#[test]
fn rust_attributes() {
    for attr in [
        "#[doc = \"[\"]",
        "#[doc = \"]\"]",
        "#[doc = \"\\\"]\"]",
        "#[doc = \"\\\\\"]",
        "#[doc = \"a\n]b\"]",
        "#[doc = r\"]\"]",
        "#[doc = r#\"[\"#]",
        "#[doc = r#\"]\"]\"#]",
        "#[x = br\"]\"]",
        "#[x = b\"[\"]",
        "#[x = '[']",
        "#[x = ']']",
        "#[x = '\\'']",
        "#[x('a, ']')]",
        "#[derive(\n    Debug, // ]\n)]",
        "#[cfg(/* ] */ unix)]",
        "#[cfg(/* /* ] */ ] */ unix)]",
        "#[x(a / b)]",
        // A `/` or `*` after a comment or operator doesn't start a comment.
        "#[x(/* a *//b)]",
        "#[x(a */ b)]",
        "#[a[b][c]]",
        "#![doc = \"[\"]",
    ] {
        assert_eq!(
            toks("rust", &format!("{attr}\nfn main() {{}}")),
            [
                t(attr, Tok::Attr),
                t("fn", Tok::Keyword),
                t("main", Tok::Function)
            ],
            "{attr}"
        );
        // Not at the start of the code, and right after another token.
        assert_eq!(
            toks("rust", &format!("x;{attr} fn")),
            [t(attr, Tok::Attr), t("fn", Tok::Keyword)],
            "{attr}"
        );
    }
    // `#!` without `[` isn't an attribute (it used to take the `]` as
    // depth -1 and swallow the rest of the code).
    for code in [
        "let x = [a#!];\nfn main() {}",
        "let x = [a#!]; fn main() {}",
    ] {
        assert_eq!(
            toks("rust", code),
            [
                t("let", Tok::Keyword),
                t("fn", Tok::Keyword),
                t("main", Tok::Function)
            ],
            "{code:?}"
        );
    }
    assert_eq!(toks("rust", "x #!] fn"), [t("fn", Tok::Keyword)]);
    // A shebang line, only at the very start.
    assert_eq!(
        toks("rust", "#!/usr/bin/env -S cargo -Zscript [x]\nfn main() {}"),
        [
            t("#!/usr/bin/env -S cargo -Zscript [x]", Tok::Attr),
            t("fn", Tok::Keyword),
            t("main", Tok::Function),
        ]
    );
    assert_eq!(
        toks("rust", "#!/bin/x\nlet a = b #!c;"),
        [t("#!/bin/x", Tok::Attr), t("let", Tok::Keyword)]
    );
    // An inner attribute at the start isn't a shebang.
    assert_eq!(
        toks("rust", "#![no_std] fn"),
        [t("#![no_std]", Tok::Attr), t("fn", Tok::Keyword)]
    );
    // Unbalanced: the rest of the code.
    assert_eq!(
        toks("rust", "#[a(\"]\"\nfn x() {}"),
        [t("#[a(\"]\"\nfn x() {}", Tok::Attr)]
    );
    // `r"` only starts a raw string after a non-identifier: in `ar"\")]`
    // the `\"` is an escaped quote, so the string (and attribute) is open.
    assert_eq!(
        toks("rust", "#[x(r\"\\\")]\nfn"),
        [t("#[x(r\"\\\")]", Tok::Attr), t("fn", Tok::Keyword)]
    );
    assert_eq!(
        toks("rust", "#[x(ar\"\\\")]\nfn"),
        [t("#[x(ar\"\\\")]\nfn", Tok::Attr)]
    );
}

/// A character literal is one character (or an escape) and a closing quote;
/// anything else after `'` is a lifetime or label, including non-ASCII
/// ones (#26: `'é>(s: &'` used to be one character literal).
#[test]
fn rust_unicode_lifetimes_and_char_literals() {
    assert_eq!(
        toks("rust", "fn f<'é>(s: &'é str) {}\nfn next() {}"),
        [
            t("fn", Tok::Keyword),
            t("f", Tok::Function),
            t("'é", Tok::Attr),
            t("'é", Tok::Attr),
            t("str", Tok::Type),
            t("fn", Tok::Keyword),
            t("next", Tok::Function),
        ]
    );
    for lit in [
        "'a'",
        "'é'",
        "'日'",
        "'😀'",
        "'\\n'",
        "'\\''",
        "'\\\\'",
        "'\\x7f'",
        "'\\u{1F600}'",
    ] {
        assert_eq!(
            toks("rust", &format!("x({lit}, 'b);")),
            [
                t("x", Tok::Function),
                t(lit, Tok::String),
                t("'b", Tok::Attr)
            ],
            "{lit}"
        );
    }
    for life in ["'a", "'é", "'éa", "'日本", "'static", "'_", "'outer"] {
        assert_eq!(
            toks("rust", &format!("&{life} T; 'c'")),
            [t(life, Tok::Attr), t("T", Tok::Type), t("'c'", Tok::String)],
            "{life}"
        );
    }
}

/// Block comments nest in some languages (#27: Rust used to end the
/// comment at the inner `*/`), and not in others.
#[test]
fn nested_block_comments() {
    assert_eq!(
        toks(
            "rust",
            "/* outer /* inner */ let hidden = 1; */\nfn main() {}"
        ),
        [
            t("/* outer /* inner */ let hidden = 1; */", Tok::Comment),
            t("fn", Tok::Keyword),
            t("main", Tok::Function),
        ]
    );
    let nesting = ["rust", "swift", "kotlin", "scala", "dart", "haskell"];
    let mut checked = BTreeSet::new();
    for group in LANG_GROUPS {
        let lang = group[0];
        let (open, close) = match lang {
            "haskell" => ("{-", "-}"),
            "lua" => ("--[[", "]]"),
            _ => ("/*", "*/"),
        };
        let code = format!("{open} a {open} b {close} 1 {close} 2");
        if highlight(lang, &code).first().map(|h| h.tok) != Some(Tok::Comment) {
            continue; // no block comments
        }
        checked.insert(lang);
        let want = if nesting.contains(&lang) {
            vec![
                t(
                    &format!("{open} a {open} b {close} 1 {close}"),
                    Tok::Comment,
                ),
                t("2", Tok::Number),
            ]
        } else {
            vec![
                t(&format!("{open} a {open} b {close}"), Tok::Comment),
                t("1", Tok::Number),
                t("2", Tok::Number),
            ]
        };
        assert_eq!(toks(lang, &code), want, "{lang}");
    }
    assert_eq!(
        checked.into_iter().collect::<Vec<_>>(),
        [
            "c",
            "cpp",
            "csharp",
            "css",
            "dart",
            "go",
            "haskell",
            "java",
            "javascript",
            "json",
            "kotlin",
            "lua",
            "nix",
            "php",
            "rust",
            "scala",
            "sql",
            "swift",
            "typescript"
        ]
    );
    for lang in nesting {
        let (open, close) = if lang == "haskell" {
            ("{-", "-}")
        } else {
            ("/*", "*/")
        };
        // Three levels, then an unclosed inner comment: the rest of the code.
        let deep = format!("{open}{open}{open} a {close}{close}{close} 1");
        let unclosed = format!("{open} a {open} b {close} 1");
        assert_eq!(
            toks(lang, &deep),
            [
                t(
                    &format!("{open}{open}{open} a {close}{close}{close}"),
                    Tok::Comment
                ),
                t("1", Tok::Number),
            ],
            "{lang}"
        );
        assert_eq!(
            toks(lang, &unclosed),
            [t(&unclosed, Tok::Comment)],
            "{lang}"
        );
    }
    // `/*/` opens a comment and doesn't close it; `*/*` closes it.
    assert_eq!(
        toks("rust", "/*/ a */* 1"),
        [t("/*/ a */", Tok::Comment), t("1", Tok::Number)]
    );
}

#[test]
fn shell_details() {
    assert_eq!(
        toks("sh", "$ ls -la\n% echo $HOME ${X:-y} $(pwd) $? $@ $# $1"),
        [
            t("$", Tok::Comment),
            t("ls", Tok::Function),
            t("-la", Tok::Attr),
            t("%", Tok::Comment),
            t("echo", Tok::Function),
            t("$HOME", Tok::Variable),
            t("${X:-y}", Tok::Variable),
            t("$(pwd)", Tok::Variable),
            t("$?", Tok::Variable),
            t("$@", Tok::Variable),
            t("$#", Tok::Variable),
            t("$1", Tok::Variable),
        ]
    );
    assert_eq!(
        toks(
            "bash",
            "A=1 B=2 make -j 4 | tee log; for x in a b; do echo; done"
        ),
        [
            t("A", Tok::Variable),
            t("B", Tok::Variable),
            t("make", Tok::Function),
            t("-j", Tok::Attr),
            t("4", Tok::Number),
            t("tee", Tok::Function),
            t("for", Tok::Keyword),
            t("x", Tok::Function),
            t("in", Tok::Keyword),
            t("do", Tok::Keyword),
            t("echo", Tok::Function),
            t("done", Tok::Keyword),
        ]
    );
    assert_eq!(
        toks("sh", "export A=1; local b; echo a#b # c\n'x' \"$y\""),
        [
            t("export", Tok::Keyword),
            t("local", Tok::Keyword),
            t("echo", Tok::Function),
            t("# c", Tok::Comment),
            t("'x'", Tok::String),
            t("\"$y\"", Tok::String),
        ]
    );
}

/// An unclosed `${` / `$(` stops before the newline (#45): the newline
/// still starts a new command, and the variable doesn't include it.
#[test]
fn unclosed_dollar_group_stops_at_the_newline() {
    for open in ["${", "$("] {
        assert_eq!(
            toks("sh", &format!("echo {open}FOO\nls -la\n$ pwd")),
            [
                t("echo", Tok::Function),
                t(&format!("{open}FOO"), Tok::Variable),
                t("ls", Tok::Function),
                t("-la", Tok::Attr),
                t("$", Tok::Comment),
                t("pwd", Tok::Function),
            ],
            "{open}"
        );
        // The generic lexer's `$vars` (PHP, Make, Dockerfile).
        assert_eq!(
            toks("php", &format!("{open}a\n$b")),
            [
                t(&format!("{open}a"), Tok::Variable),
                t("$b", Tok::Variable)
            ],
            "{open}"
        );
        assert_eq!(
            toks("make", &format!("{open}A\n#x")),
            [t(&format!("{open}A"), Tok::Variable), t("#x", Tok::Comment)],
            "{open}"
        );
    }
    // Closed groups still include the closing bracket.
    assert_eq!(
        toks("sh", "a ${B}c $(d)\ne"),
        [
            t("a", Tok::Function),
            t("${B}", Tok::Variable),
            t("$(d)", Tok::Variable),
            t("e", Tok::Function),
        ]
    );
    assert_eq!(toks("php", "${b}c"), [t("${b}", Tok::Variable)]);
}

#[test]
fn diff_details() {
    assert_eq!(
        toks(
            "diff",
            "diff --git a b\nindex 1..2\n--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n z\n<l\n>r\n\\ No newline"
        ),
        [
            t("diff --git a b", Tok::Meta),
            t("index 1..2", Tok::Meta),
            t("--- a", Tok::Meta),
            t("+++ b", Tok::Meta),
            t("@@ -1 +1 @@", Tok::Meta),
            t("-x", Tok::Deleted),
            t("+y", Tok::Inserted),
            t("<l", Tok::Deleted),
            t(">r", Tok::Inserted),
        ]
    );
    // A line's highlight never includes its `\n` (a CRLF's `\r` stays in;
    // the parser hands code blocks over with `\n` line endings anyway).
    let hs = highlight("diff", "+a\r\n+b\n");
    assert_eq!(
        hs,
        [
            Highlight {
                range: 0..3,
                tok: Tok::Inserted
            },
            Highlight {
                range: 4..6,
                tok: Tok::Inserted
            }
        ]
    );
}

#[test]
fn markup_details() {
    assert_eq!(
        toks(
            "html",
            "<a href=\"x\" b='y' c>t &amp; &#35; &x</a><!-- c --><br/>"
        ),
        [
            t("a", Tok::Tag),
            t("href", Tok::Property),
            t("\"x\"", Tok::String),
            t("b", Tok::Property),
            t("'y'", Tok::String),
            t("c", Tok::Property),
            t("&amp;", Tok::Constant),
            t("&#35;", Tok::Constant),
            t("a", Tok::Tag),
            t("<!-- c -->", Tok::Comment),
            t("br", Tok::Tag),
        ]
    );
    // Malformed empty entities still let the following tag be highlighted.
    assert_eq!(
        toks("html", "&;<br/>"),
        [t("&;", Tok::Constant), t("br", Tok::Tag)]
    );
    // A value cut off at EOF still belongs to the attribute string.
    assert_eq!(
        toks("html", "<p title=\"unfinished"),
        [
            t("p", Tok::Tag),
            t("title", Tok::Property),
            t("\"unfinished", Tok::String)
        ]
    );
    // Multi-line attribute values, and `<` that doesn't start a tag.
    assert_eq!(
        toks("xml", "<p t=\"a\nb\">1 < 2 <3</p>"),
        [
            t("p", Tok::Tag),
            t("t", Tok::Property),
            t("\"a\nb\"", Tok::String),
            t("p", Tok::Tag),
        ]
    );
    // A value that starts with a newline (#42) used to end at its opening
    // quote, which inverted the quotes for the rest of the tag.
    for q in ['"', '\''] {
        assert_eq!(
            toks(
                "html",
                &format!("<p title={q}\nhello\n{q} a={q}x{q}>t</p>\n<b c={q}{q}>")
            ),
            [
                t("p", Tok::Tag),
                t("title", Tok::Property),
                t(&format!("{q}\nhello\n{q}"), Tok::String),
                t("a", Tok::Property),
                t(&format!("{q}x{q}"), Tok::String),
                t("p", Tok::Tag),
                t("b", Tok::Tag),
                t("c", Tok::Property),
                t(&format!("{q}{q}"), Tok::String),
            ],
            "{q}"
        );
    }
    // Values have no escapes; the other quote doesn't close them.
    assert_eq!(
        toks("xml", "<a b=\"x\\\" c='\" d\"'>"),
        [
            t("a", Tok::Tag),
            t("b", Tok::Property),
            t("\"x\\\"", Tok::String),
            t("c", Tok::Property),
            t("'\" d\"'", Tok::String),
        ]
    );
    // Unclosed after a newline: the rest of the code.
    assert_eq!(
        toks("html", "<a b=\"\nc>d"),
        [
            t("a", Tok::Tag),
            t("b", Tok::Property),
            t("\"\nc>d", Tok::String)
        ]
    );
}

#[test]
fn config_details() {
    assert_eq!(
        toks(
            "yaml",
            "a: 1\n- b: -2.5\n  c: \"s\" # x\nd: &anc\ne: *anc\nf: yes\ng:\n---\n..."
        ),
        [
            t("a", Tok::Property),
            t("1", Tok::Number),
            t("b", Tok::Property),
            t("-2.5", Tok::Number),
            t("c", Tok::Property),
            t("\"s\"", Tok::String),
            t("# x", Tok::Comment),
            t("d", Tok::Property),
            t("&anc", Tok::Variable),
            t("e", Tok::Property),
            t("*anc", Tok::Variable),
            t("f", Tok::Property),
            t("yes", Tok::Constant),
            t("g", Tok::Property),
            t("---", Tok::Meta),
            t("...", Tok::Meta),
        ]
    );
    assert_eq!(
        toks(
            "toml",
            "[a.b] # s\nk = [1, true]\n\"q\" = 'v'\nurl: http://x\n; c\nx y = 1"
        ),
        [
            t("[a.b]", Tok::Type),
            t("# s", Tok::Comment),
            t("k", Tok::Property),
            t("1", Tok::Number),
            t("true", Tok::Constant),
            t("\"q\"", Tok::Property),
            t("'v'", Tok::String),
            t("url", Tok::Property),
            t("; c", Tok::Comment),
            // `x y = 1` isn't a key (keys have no spaces): just a value.
            t("1", Tok::Number),
        ]
    );
}

/// A section header ends at its own `]`, not the last `]` on the line
/// (#44): a trailing comment may contain brackets, and quoted keys may too.
#[test]
fn config_section_header_ends_at_its_bracket() {
    for (code, header, comment) in [
        ("[package] # see [section]", "[package]", "# see [section]"),
        ("[[bin]] # [[x]]", "[[bin]]", "# [[x]]"),
        ("[a.\"b]c\"] # ]", "[a.\"b]c\"]", "# ]"),
        ("[a.'b]c'] # ]", "[a.'b]c']", "# ]"),
        // Basic strings have escapes, literal strings don't.
        ("[a.\"b\\\"]\"] # ]", "[a.\"b\\\"]\"]", "# ]"),
        ("[a.'b\\'] # ]", "[a.'b\\']", "# ]"),
    ] {
        for lang in ["toml", "ini"] {
            assert_eq!(
                toks(lang, &format!("  {code}\nk = 1")),
                [
                    t(header, Tok::Type),
                    t(comment, Tok::Comment),
                    t("k", Tok::Property),
                    t("1", Tok::Number),
                ],
                "{lang}: {code}"
            );
        }
    }
    // Unclosed headers (or quotes) run to the end of the line.
    for code in ["[a # b", "[[a] # b", "[a.\"b] # c", "[a.\"b\\\"] # c"] {
        assert_eq!(
            toks("toml", &format!("{code}\nk")),
            [t(code, Tok::Type)],
            "{code}"
        );
    }
    // Text after the header is still a value.
    assert_eq!(
        toks("ini", "[a] 1 [b]"),
        [t("[a]", Tok::Type), t("1", Tok::Number)]
    );
}

/// A backslash escapes the next character in a triple-quoted string (#28),
/// so `\"""` doesn't close it, while `\\"""` does. Python raw strings
/// (`r"""`) follow the same rule. Kotlin and Scala `"""` strings are raw:
/// a backslash is just a character.
#[test]
fn triple_quote_escapes() {
    for q in ["\"", "'"] {
        let qqq = q.repeat(3);
        assert_eq!(
            toks("python", &format!("x = {qqq}a\\{qqq}b{qqq}\nprint(x)")),
            [
                t(&format!("{qqq}a\\{qqq}b{qqq}"), Tok::String),
                t("print", Tok::Keyword),
            ],
            "{qqq}"
        );
        assert_eq!(
            toks("python", &format!("x = {qqq}a\\\\{qqq} if\n{qqq}")),
            [
                t(&format!("{qqq}a\\\\{qqq}"), Tok::String),
                t("if", Tok::Keyword),
                t(&qqq, Tok::String),
            ],
            "{qqq}"
        );
        // An escaped quote right before the closing ones.
        assert_eq!(
            toks("python", &format!("r{qqq}a\\{q}{qqq} or")),
            [
                t(&format!("{qqq}a\\{q}{qqq}"), Tok::String),
                t("or", Tok::Keyword),
            ],
            "{qqq}"
        );
        // A backslash at the very end.
        assert_eq!(
            toks("python", &format!("{qqq}a\\")),
            [t(&format!("{qqq}a\\"), Tok::String)]
        );
    }
    for (lang, kw) in [("swift", "let"), ("dart", "var"), ("elixir", "end")] {
        assert_eq!(
            toks(lang, &format!("\"\"\"a\\\"\"\"b\"\"\"\n{kw}")),
            [
                t("\"\"\"a\\\"\"\"b\"\"\"", Tok::String),
                t(kw, Tok::Keyword)
            ],
            "{lang}"
        );
    }
    for lang in ["kotlin", "scala"] {
        assert_eq!(
            toks(lang, "val p = \"\"\"C:\\\"\"\"\nval q = 1"),
            [
                t("val", Tok::Keyword),
                t("\"\"\"C:\\\"\"\"", Tok::String),
                t("val", Tok::Keyword),
                t("1", Tok::Number),
            ],
            "{lang}"
        );
    }
}

#[test]
fn generic_details() {
    // SQL keywords are case-insensitive; other languages' aren't.
    assert_eq!(
        toks("sql", "Select x FROM t"),
        [t("Select", Tok::Keyword), t("FROM", Tok::Keyword)]
    );
    assert_eq!(toks("python", "If x"), [t("If", Tok::Type)]);
    // Python `#` comments need a space (or line start) before them.
    assert_eq!(
        toks("python", "a#b\n# c\nx = 1 # d"),
        [
            t("# c", Tok::Comment),
            t("1", Tok::Number),
            t("# d", Tok::Comment)
        ]
    );
    // C preprocessor lines, only at line start.
    assert_eq!(
        toks("c", "#include <a.h>\n  #define MAX 1\nx # y"),
        [
            t("#include", Tok::Attr),
            t("#define", Tok::Attr),
            t("MAX", Tok::Constant),
            t("1", Tok::Number),
        ]
    );
    // Reserved PHP literals keep their class beside parentheses, even in
    // incomplete code (they must not be mistaken for function names).
    assert_eq!(
        toks("php", "TRUE(FALSE) NULL()"),
        [
            t("TRUE", Tok::Constant),
            t("FALSE", Tok::Constant),
            t("NULL", Tok::Constant)
        ]
    );
    // Decorators, `$vars`, CSS properties and at-rules.
    assert_eq!(
        toks("java", "@Override @a.b-c @ x"),
        [t("@Override", Tok::Attr), t("@a.b-c", Tok::Attr)]
    );
    assert_eq!(
        toks("php", "$a ${b} $(c) $"),
        [
            t("$a", Tok::Variable),
            t("${b}", Tok::Variable),
            t("$(c)", Tok::Variable),
        ]
    );
    assert_eq!(
        toks("css", "@media x { a { -webkit-x: calc(1px) #fff; } }"),
        [
            t("@media", Tok::Keyword),
            t("x", Tok::Tag),
            // Selectors nested in a block aren't tags (the lexer only tracks
            // brace depth).
            t("-webkit-x", Tok::Property),
            t("calc", Tok::Function),
            t("1px", Tok::Number),
            t("#fff", Tok::Number),
        ]
    );
    // Function names after `fn`-like keywords; types if capitalized.
    assert_eq!(
        toks("python", "def f(): class C: pass"),
        [
            t("def", Tok::Keyword),
            t("f", Tok::Function),
            t("class", Tok::Keyword),
            t("C", Tok::Type),
            t("pass", Tok::Keyword),
        ]
    );
    // JSON keys vs values.
    assert_eq!(
        toks("json", "{\"k\" : \"v\", \"n\":1}"),
        [
            t("\"k\"", Tok::Property),
            t("\"v\"", Tok::String),
            t("\"n\"", Tok::Property),
            t("1", Tok::Number),
        ]
    );
    // Lua block comments win over line comments; Haskell's `{- -}`.
    assert_eq!(
        toks("lua", "--[[ a\nb ]] x -- c"),
        [t("--[[ a\nb ]]", Tok::Comment), t("-- c", Tok::Comment)]
    );
    assert_eq!(toks("hs", "{- a -} b"), [t("{- a -}", Tok::Comment)]);
    // Unterminated strings stop at the end of the line, except backticks.
    assert_eq!(
        toks("js", "\"a\nb `c\nd` e"),
        [t("\"a", Tok::String), t("`c\nd`", Tok::String)]
    );
    // Triple quotes.
    assert_eq!(
        toks("kotlin", "\"\"\"a\n\"b\"\n\"\"\" x"),
        [t("\"\"\"a\n\"b\"\n\"\"\"", Tok::String)]
    );
    // ALL_CAPS constants need two characters and a letter.
    assert_eq!(
        toks("go", "A AB A1 _B 1A __"),
        [
            t("AB", Tok::Constant),
            t("A1", Tok::Constant),
            t("_B", Tok::Constant),
            t("1A", Tok::Number)
        ]
    );
}

/// Highlights stored in a parsed code block are the ones `highlight`
/// computes, for every language (the parser passes the info string's first
/// word through).
#[test]
fn parser_uses_the_info_string() {
    for lang in all_langs() {
        let code = sample(LANG_GROUPS.iter().find(|g| g.contains(&lang)).unwrap()[0]);
        let fence = "`".repeat(8);
        let md = format!("{fence}{lang} extra words\n{code}{fence}\n");
        let doc = common::invariants::parse_and_check(&md);
        match &doc.blocks[..] {
            [
                mdview_core::markdown::Block::Code {
                    lang: l,
                    code: c,
                    highlights,
                },
            ] => {
                assert_eq!(l, lang);
                assert_eq!(c, code.strip_suffix('\n').unwrap_or(&code));
                check_highlights(c, highlights, lang);
                assert_eq!(*highlights, highlight(lang, c));
            }
            other => panic!("{lang}: {other:?}"),
        }
    }
}

/// One line per construct any lexer knows. Each language's result shows
/// which of them it supports (comment styles, quotes, decorators,
/// variables, constants, function keywords...), so changing a language's
/// configuration changes this snapshot.
const PROBE: &str = r##"/* block */ x
(* ml *) x
{- hs -} x
--[[ lua ]] x
<!-- html --> x
// line
# hash
-- dash
; semi
"dq" x
'sq' x
`bt` x
"""tdq
""" x
'''tsq''' x
@attr @a.b-c x
$var ${braced} $(paren) $
#include <x>
  #define X
#[attr] #![inner] r"raw" r#"h"# br"b" 'a 'c' '\n' name!() a != b Vec::<u8>
true false null nil None NULL nullptr undefined NaN Infinity this self Self super cls it iota
True False Nothing Just TRUE FALSE Some Ok Err
fn f def g func h function i fun j defp k class C
Foo FOO_BAR X1 fooBar(1) x
0x1F 1.5e-3 .5 1_000 10n 1e+5 0..10 x.5
SELECT Select select from Where int VARCHAR
a { color: red; -webkit-x: 1; } #fff .b:hover x
{"key": "value", "n": 1}
if else for while return import let const var
int char void bool string number String u8 i32 float str
"##;

#[test]
fn probe_every_language() {
    let mut out = String::new();
    for group in LANG_GROUPS {
        let lang = group[0];
        let hs = highlight_and_check(lang, PROBE);
        out.push_str(&format!("== {lang}\n"));
        dump::source(&mut out, &dump::tokens(PROBE, &hs), 0);
    }
    insta::assert_snapshot!(out);
}
