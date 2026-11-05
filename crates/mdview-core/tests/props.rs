//! Property tests. The generators build text out of syntax fragments
//! (emphasis markers, brackets, fences, `$`, pipes, list markers, line
//! endings, tabs, multi-byte and invisible characters...) rather than only
//! random Unicode, so they reach the interesting parser states.
//!
//! Failing inputs are minimized and saved under `proptest-regressions/`;
//! commit those files so the case is replayed forever. For a deeper run:
//! `PROPTEST_CASES=20000 cargo test -p mdview-core --test props`.

mod common;

use common::{
    dump,
    invariants::{
        check_math, check_slug, highlight_and_check, parse_and_check, to_unicode_and_check,
    },
    langs::all_langs,
};
use mdview_core::{
    latex,
    markdown::{BOLD, Block, CODE, ITALIC, LINK, STRIKE, parse, slugify},
};
use proptest::{prelude::*, sample::select};

/// Bits of Markdown syntax, in contexts where they mean something.
const MD_TOKENS: &[&str] = &[
    "*",
    "**",
    "***",
    "_",
    "__",
    "~",
    "~~",
    "`",
    "``",
    "```",
    "````",
    "~~~",
    "$",
    "$$",
    "\\",
    "[",
    "]",
    "](",
    ")",
    "(",
    "![",
    "[^1]",
    "[^1]: ",
    "[^a b]",
    "]: ",
    "<",
    ">",
    "</",
    "/>",
    "<br>",
    "<br />",
    "<div>",
    "</div>",
    "<!--",
    "-->",
    "<a href=\"x\">",
    "&amp;",
    "&#35;",
    "&",
    ";",
    "|",
    "|---|",
    "|:-:|",
    "| a |",
    ":",
    "-",
    "+",
    "=",
    "#",
    "## ",
    "> ",
    ">",
    "- ",
    "* ",
    "+ ",
    "1. ",
    "2) ",
    "- [ ] ",
    "- [x] ",
    "> [!NOTE]\n",
    "> [!WARNING]\n",
    "---",
    "---\n",
    "+++\n",
    "***\n",
    "===",
    "\n",
    "\n",
    "\n\n",
    "\r\n",
    "\r",
    " ",
    " ",
    "  ",
    "    ",
    "\t",
    "  \n",
    "\\\n",
    "```math\n",
    "```mermaid\n",
    "```rust\n",
    "```sh\n",
    "\\frac{",
    "\\mathit{",
    "\\text{",
    "\\begin{pmatrix}",
    "\\end{pmatrix}",
    "{",
    "}",
    "^",
    "\\alpha",
    "http://x.y",
    "<https://x.y>",
    "<a@b.c>",
    "www.x.y",
    "'",
    "\"",
    "!",
    "?",
    ".",
    "é",
    "日本",
    "👍🏽",
    "👩\u{200d}💻",
    "\u{301}",
    "\u{200b}",
    "\u{202e}",
    "\u{feff}",
    "\u{a0}",
    "İ",
    "ǅ",
    "ß",
    "\u{0}",
];

fn word() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9]{1,6}"
}

fn markdown_ish() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            6 => select(MD_TOKENS).prop_map(String::from),
            3 => word(),
            1 => any::<char>().prop_map(String::from),
        ],
        0..80,
    )
    .prop_map(|parts| parts.concat())
}

const TEX_TOKENS: &[&str] = &[
    "\\",
    "{",
    "}",
    "^",
    "_",
    "&",
    "%",
    "'",
    "~",
    "[",
    "]",
    "(",
    ")",
    "-",
    "+",
    "=",
    "<",
    ">",
    ",",
    ";",
    ":",
    " ",
    "\n",
    "x",
    "y",
    "1",
    "2",
    "é",
    "日",
    "\\alpha",
    "\\frac",
    "\\sqrt",
    "\\sqrt[",
    "\\text{",
    "\\textit{",
    "\\mathit{",
    "\\mathrm{",
    "\\mathbb{",
    "\\boldsymbol{",
    "\\operatorname{",
    "\\hat",
    "\\overline",
    "\\not",
    "\\sin",
    "\\left(",
    "\\right)",
    "\\left.",
    "\\begin{pmatrix}",
    "\\end{pmatrix}",
    "\\begin{cases}",
    "\\end{cases}",
    "\\begin{array}{cc}",
    "\\begin",
    "\\end",
    "\\\\",
    "\\,",
    "\\quad",
    "\\overset",
    "\\binom",
    "\\mathbin{",
    "\\mathrel{",
    "\\tag{",
    "\\label{",
    "\\color{",
    "\\textcolor{",
    "\\foo",
    "\\{",
    "\\}",
    "\\sum",
    "\\int",
    "\\cdot",
    "\\le",
    "\\in",
    "\\vec",
    "*",
    "\u{301}",
    "\u{2003}",
];

fn tex_ish() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            8 => select(TEX_TOKENS).prop_map(String::from),
            1 => any::<char>().prop_map(String::from),
        ],
        0..40,
    )
    .prop_map(|parts| parts.concat())
}

const CODE_TOKENS: &[&str] = &[
    "\"", "'", "`", "\"\"\"", "'''", "\\", "\\\"", "//", "/*", "*/", "#", "#[", "#![", "--",
    "--[[", "]]", "{-", "-}", "<!--", "-->", "<", ">", "</", "/>", "&", "&amp;", ";", "@", "$",
    "${", "$(", "(", ")", "{", "}", "[", "]", ":", "=", ",", ".", "..", "-", "+", "!", "?", "|",
    "r\"", "r#\"", "\"#", "br\"", "'a", "0x1F", "1e-3", "1.5", "42", "_", "fn", "def", "let", "if",
    "true", "NULL", "Foo", "FOO_BAR", "x", "name", "- ", "key: ", "k = ", "[sec]", "---", "...",
    "+++", "@@", "diff ", "$ ", "% ", "\n", "\r\n", "\r", " ", "\t", "é", "日本", "👍🏽", "\u{301}",
    "\u{0}",
];

fn code_ish() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            8 => select(CODE_TOKENS).prop_map(String::from),
            1 => any::<char>().prop_map(String::from),
        ],
        0..60,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(ProptestConfig {
        // Keep `cargo test` fast; raise PROPTEST_CASES for deeper runs.
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(512),
        ..ProptestConfig::default()
    })]

    /// `parse` never panics, and its output satisfies every invariant.
    #[test]
    fn parse_markdown_ish(src in markdown_ish()) {
        parse_and_check(&src);
    }

    #[test]
    fn parse_arbitrary_unicode(src in any::<String>()) {
        parse_and_check(&src);
    }

    /// Several fragments as separate blocks, nested in quotes and lists.
    #[test]
    fn parse_nested(parts in prop::collection::vec(markdown_ish(), 1..5)) {
        let mut src = String::new();
        for (i, p) in parts.iter().enumerate() {
            let prefix = ["", "> ", "- ", "1. ", "> - ", "  - ", "| "][i % 7];
            for line in p.split('\n') {
                src.push_str(prefix);
                src.push_str(line);
                src.push('\n');
            }
            src.push('\n');
        }
        parse_and_check(&src);
    }

    /// Same input, same output (no hidden state).
    #[test]
    fn parse_is_deterministic(src in markdown_ish()) {
        prop_assert_eq!(
            dump::document(&parse(&src)),
            dump::document(&parse(&src))
        );
    }

    /// CRLF and LF line endings give the same document.
    #[test]
    fn crlf_is_like_lf(src in markdown_ish()) {
        // No code spans (pulldown-cmark quirk, see `crlf_inside_a_code_span`
        // below); `~~~` fences still make code blocks.
        let lf = src.replace("\r\n", "\n").replace('\r', "").replace('`', "~");
        let crlf = lf.replace('\n', "\r\n");
        prop_assert_eq!(dump::document(&parse(&crlf)), dump::document(&parse(&lf)));
    }

    /// Plain words are never lost or changed: they come back as one
    /// paragraph, joined by single spaces.
    #[test]
    fn plain_text_is_kept(
        words in prop::collection::vec("[a-zA-Z0-9éß日本äöü]{1,8}", 1..30),
        seps in prop::collection::vec(select(&[" ", "  ", "\n", " \n", "\t"][..]), 30),
    ) {
        let mut src = String::new();
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                src.push_str(seps[i]);
            }
            src.push_str(w);
        }
        let doc = parse_and_check(&src);
        let [Block::Paragraph(p)] = &doc.blocks[..] else {
            panic!("{src:?}: {:?}", doc.blocks);
        };
        // Spaces and tabs inside a line are kept as they are; a line break
        // (with any trailing space) becomes one space.
        let expected: String = words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let sep = match seps[i] {
                    "\n" | " \n" => " ",
                    other => other,
                };
                if i == 0 { w.clone() } else { format!("{sep}{w}") }
            })
            .collect();
        prop_assert_eq!(&p.text, &expected);
        prop_assert!(p.spans.iter().all(|s| s.style == 0 && s.link.is_none()));
    }

    /// Inline styles end up exactly on the right words: a model of what
    /// `*a* **b** `c` [d](u)...` should produce.
    #[test]
    fn inline_styles_match_a_model(
        pieces in prop::collection::vec((0..8usize, "[a-z0-9éß日]{1,6}", "[a-z]{1,4}"), 1..12)
    ) {
        let mut src = String::new();
        let mut text = String::new();
        // Expected (style, link) of every byte of `text`.
        let mut styles: Vec<(u8, Option<u32>)> = Vec::new();
        let mut links = Vec::new();
        for (i, (kind, w, url)) in pieces.iter().enumerate() {
            if i > 0 {
                src.push(' ');
                text.push(' ');
                styles.push((0, None));
            }
            let (md, style, link) = match kind {
                0 => (w.clone(), 0, None),
                1 => (format!("*{w}*"), ITALIC, None),
                2 => (format!("**{w}**"), BOLD, None),
                3 => (format!("***{w}***"), BOLD | ITALIC, None),
                4 => (format!("~~{w}~~"), STRIKE, None),
                5 => (format!("`{w}`"), CODE, None),
                6 => {
                    links.push(url.clone());
                    (format!("[{w}]({url})"), LINK, Some(links.len() as u32 - 1))
                }
                _ => {
                    links.push(url.clone());
                    (format!("**[{w}]({url})**"), BOLD | LINK, Some(links.len() as u32 - 1))
                }
            };
            src.push_str(&md);
            text.push_str(w);
            styles.extend(std::iter::repeat_n((style, link), w.len()));
        }
        let doc = parse_and_check(&src);
        let [Block::Paragraph(p)] = &doc.blocks[..] else {
            panic!("{src:?}: {:?}", doc.blocks);
        };
        prop_assert_eq!(&p.text, &text);
        prop_assert_eq!(&p.links, &links);
        let mut got = vec![(0u8, None); p.text.len()];
        for s in &p.spans {
            for b in &mut got[s.range.clone()] {
                *b = (s.style, s.link);
            }
        }
        prop_assert_eq!(got, styles, "{}", dump::styled(p));
    }

    /// Headings get `slugify(text)`, and slugs only contain allowed
    /// characters whatever the input.
    #[test]
    fn slugs(s in any::<String>(), md in markdown_ish()) {
        check_slug(&slugify(&s), "slugify");
        check_slug(&slugify(&md), "slugify");
        // ASCII-only slugs are stable when slugified again.
        let ascii: String = s.chars().filter(char::is_ascii).collect();
        let once = slugify(&ascii);
        prop_assert_eq!(slugify(&once), once.clone());
        prop_assert!(once.len() <= ascii.len());
        // And the heading built from the text has that slug.
        let line: String = md.chars().filter(|&c| c != '\n' && c != '\r').collect();
        parse_and_check(&format!("# {line}\n"));
    }

    /// Every language: highlights are sorted, non-overlapping, non-empty,
    /// in bounds and on char boundaries.
    #[test]
    fn highlight_every_language(code in code_ish()) {
        for lang in all_langs() {
            highlight_and_check(lang, &code);
        }
    }

    #[test]
    fn highlight_arbitrary_unicode(code in any::<String>()) {
        for lang in all_langs() {
            highlight_and_check(lang, &code);
        }
    }

    /// `to_unicode` never panics and its italic ranges are valid.
    #[test]
    fn latex_tex_ish(tex in tex_ish()) {
        to_unicode_and_check(&tex);
    }

    #[test]
    fn latex_arbitrary_unicode(tex in any::<String>()) {
        to_unicode_and_check(&tex);
    }

    /// Inline math inside styled text and links: the italic ranges are
    /// mapped onto the paragraph's spans correctly.
    #[test]
    fn inline_math_in_paragraphs(tex in tex_ish(), before in word(), after in word()) {
        let tex = tex.replace(['$', '\n', '\r', '`'], "");
        let m = latex::to_unicode(&tex);
        check_math(&tex, &m);
        for md in [
            format!("{before} ${tex}$ {after}"),
            format!("**{before} ${tex}$** {after}"),
            format!("[{before} ${tex}$]({after})"),
            format!("{before} $${tex}$$ {after}"),
            format!("- ${tex}$"),
            format!("| ${tex}$ |\n|---|\n| ${tex}$ |"),
        ] {
            parse_and_check(&md);
        }
        // When it's really inline math, its text and italics are in the
        // paragraph, at the right place.
        let doc = parse(&format!("{before} ${tex}$ {after}"));
        if let [Block::Paragraph(p)] = &doc.blocks[..]
            && p.text.contains(&m.text)
            && !m.text.is_empty()
            && !tex.trim().is_empty()
            && !tex.starts_with(' ')
            && !tex.ends_with(' ')
        {
            let start = before.len() + 1;
            if p.text.get(start..start + m.text.len()) == Some(m.text.as_str()) {
                for r in &m.italic {
                    let (s, e) = (start + r.start, start + r.end);
                    for span in &p.spans {
                        if span.range.start < e && s < span.range.end {
                            prop_assert!(span.style & ITALIC != 0, "{}", dump::styled(p));
                        }
                    }
                }
            }
        }
    }
}

/// Upstream pulldown-cmark quirk: a CRLF inside an inline code span becomes
/// two spaces (LF: one). Pinned here so a pulldown-cmark update that fixes
/// it shows up; then let `crlf_is_like_lf` generate backticks again.
#[test]
fn crlf_inside_a_code_span() {
    let text = |src: &str| match &parse(src).blocks[..] {
        [Block::Paragraph(p)] => p.text.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(text("`a\nb`"), "a b");
    assert_eq!(text("`a\r\nb`"), "a  b");
}
