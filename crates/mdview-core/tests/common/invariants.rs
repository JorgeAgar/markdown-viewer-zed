//! Structural invariants of the parser, highlighter and LaTeX output: the
//! things `render.rs` relies on (and slices strings with), so breaking one
//! means a wrong rendering or a panic in the viewer.
//!
//! Shared by the property tests, the spec-corpus test, the snapshot tests and
//! the fuzz target (`fuzz/fuzz_targets/parse.rs` includes this file with
//! `#[path]`), so it must only depend on `mdview_core` and `std`.

#![allow(dead_code)]

use std::ops::Range;

use mdview_core::{
    highlight::{Highlight, highlight},
    latex::{self, MathText},
    markdown::{self, Block, Document, LINK, RichText},
};

/// Parses `src` and checks every invariant of the result (including the
/// highlights of each code block and `to_unicode` on each inline formula
/// the parser could have seen).
pub fn parse_and_check(src: &str) -> Document {
    let doc = markdown::parse(src);
    check_document(&doc);
    doc
}

pub fn check_document(doc: &Document) {
    for (i, b) in doc.blocks.iter().enumerate() {
        check_block(b, &format!("blocks[{i}]"));
    }
}

fn check_block(b: &Block, at: &str) {
    match b {
        Block::Heading { level, text, slug } => {
            assert!((1..=6).contains(level), "{at}: heading level {level}");
            check_rich(text, at);
            assert_eq!(
                text.text.trim_end(),
                text.text,
                "{at}: heading text has trailing whitespace"
            );
            assert_eq!(
                *slug,
                markdown::slugify(&text.text),
                "{at}: slug isn't slugify(text)"
            );
            check_slug(slug, at);
        }
        Block::Paragraph(text) => {
            check_rich(text, at);
            assert!(!text.text.is_empty(), "{at}: empty paragraph");
            assert_eq!(
                text.text.trim(),
                text.text,
                "{at}: paragraph text isn't trimmed"
            );
        }
        Block::Images(images) => {
            assert!(!images.is_empty(), "{at}: empty image block");
        }
        Block::Code {
            lang,
            code,
            highlights,
        } => {
            assert!(
                !lang.chars().any(char::is_whitespace),
                "{at}: code lang {lang:?} has whitespace"
            );
            assert!(
                !lang.eq_ignore_ascii_case("math") && !lang.eq_ignore_ascii_case("mermaid"),
                "{at}: {lang} block should have its own block type"
            );
            check_highlights(code, highlights, &format!("{at} ({lang})"));
            assert_eq!(
                *highlights,
                highlight(lang, code),
                "{at}: stored highlights differ from highlight(lang, code)"
            );
        }
        Block::Quote { blocks, .. } | Block::Footnote { blocks, .. } => {
            for (i, b) in blocks.iter().enumerate() {
                check_block(b, &format!("{at}/{i}"));
            }
        }
        Block::List { items, .. } => {
            assert!(!items.is_empty(), "{at}: list without items");
            for (i, item) in items.iter().enumerate() {
                for (j, b) in item.blocks.iter().enumerate() {
                    check_block(b, &format!("{at}/item{i}/{j}"));
                }
            }
        }
        Block::Table {
            aligns,
            header,
            rows,
        } => {
            // GFM tables are rectangular: pulldown-cmark pads short rows and
            // drops extra cells, and `render.rs` lays cells out as equal
            // columns, indexing `aligns` by column.
            assert!(!aligns.is_empty(), "{at}: table without columns");
            assert_eq!(header.len(), aligns.len(), "{at}: header width");
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(row.len(), aligns.len(), "{at}: row {i} width");
            }
            for (i, cell) in header.iter().chain(rows.iter().flatten()).enumerate() {
                check_rich(cell, &format!("{at}/cell{i}"));
            }
        }
        Block::Math(_) | Block::Mermaid(_) | Block::Rule => {}
    }
}

/// Spans are what `render.rs` turns into text runs: they must be non-empty,
/// in bounds, on char boundaries, sorted and non-overlapping, and every link
/// index must point into `links`.
pub fn check_rich(rt: &RichText, at: &str) {
    let mut pos = 0;
    let mut prev: Option<&markdown::Span> = None;
    for (i, s) in rt.spans.iter().enumerate() {
        let r = &s.range;
        assert!(r.start < r.end, "{at}: span {i} {r:?} is empty");
        assert!(
            r.start >= pos,
            "{at}: span {i} {r:?} overlaps or is out of order (previous ended at {pos})"
        );
        assert!(
            r.end <= rt.text.len(),
            "{at}: span {i} {r:?} out of bounds ({})",
            rt.text.len()
        );
        assert!(
            rt.text.is_char_boundary(r.start) && rt.text.is_char_boundary(r.end),
            "{at}: span {i} {r:?} not on char boundaries in {:?}",
            rt.text
        );
        assert!(
            s.style & !0b1_1111 == 0,
            "{at}: span {i} has unknown style bits {:#b}",
            s.style
        );
        if let Some(l) = s.link {
            assert!(
                (l as usize) < rt.links.len(),
                "{at}: span {i} link {l} >= {}",
                rt.links.len()
            );
            assert!(
                s.style & LINK != 0,
                "{at}: span {i} has a link without the LINK style"
            );
        }
        // `RichText::push` merges touching runs with the same style.
        if let Some(p) = prev
            && p.range.end == r.start
        {
            assert!(
                p.style != s.style || p.link != s.link,
                "{at}: spans {} and {i} should have been merged",
                i - 1
            );
        }
        pos = r.end;
        prev = Some(s);
    }
    let (ranges, ids) = rt.link_ranges();
    let expected: Vec<(Range<usize>, u32)> = rt
        .spans
        .iter()
        .filter_map(|s| s.link.map(|l| (s.range.clone(), l)))
        .collect();
    let got: Vec<(Range<usize>, u32)> = ranges.into_iter().zip(ids).collect();
    assert_eq!(got, expected, "{at}: link_ranges() disagrees with spans");
}

/// GitHub-style slugs: lowercased alphanumerics, `-` and `_` (plus the
/// combining dot that lowercasing `İ` produces). Some uppercase letters,
/// like `𝕁`, have no lowercase form and stay.
pub fn check_slug(slug: &str, at: &str) {
    for c in slug.chars() {
        assert!(
            c == '-'
                || c == '_'
                || c == '\u{307}'
                || (c.is_alphanumeric() && c.to_lowercase().eq([c])),
            "{at}: slug {slug:?} contains {c:?}"
        );
    }
}

/// What `render.rs::code_text` relies on.
pub fn check_highlights(code: &str, hs: &[Highlight], at: &str) {
    let mut pos = 0;
    let mut prev: Option<&Highlight> = None;
    for (i, h) in hs.iter().enumerate() {
        let r = &h.range;
        assert!(r.start < r.end, "{at}: highlight {i} {r:?} is empty");
        assert!(
            r.start >= pos,
            "{at}: highlight {i} {r:?} overlaps or is out of order (previous ended at {pos})"
        );
        assert!(
            r.end <= code.len(),
            "{at}: highlight {i} {r:?} out of bounds ({})",
            code.len()
        );
        assert!(
            code.is_char_boundary(r.start) && code.is_char_boundary(r.end),
            "{at}: highlight {i} {r:?} not on char boundaries in {code:?}"
        );
        if let Some(p) = prev
            && p.range.end == r.start
        {
            assert!(
                p.tok != h.tok,
                "{at}: highlights {} and {i} should have been merged",
                i - 1
            );
        }
        pos = r.end;
        prev = Some(h);
    }
}

/// `push_math` slices the text with these ranges, in order.
pub fn check_math(tex: &str, m: &MathText) {
    assert_eq!(m.text.trim(), m.text, "{tex:?}: output isn't trimmed");
    let mut pos = 0;
    for (i, r) in m.italic.iter().enumerate() {
        assert!(r.start < r.end, "{tex:?}: italic {i} {r:?} is empty");
        assert!(
            r.start >= pos,
            "{tex:?}: italic {i} {r:?} overlaps or is out of order ({:?})",
            m.italic
        );
        assert!(
            r.end <= m.text.len(),
            "{tex:?}: italic {i} {r:?} out of bounds in {:?}",
            m.text
        );
        assert!(
            m.text.is_char_boundary(r.start) && m.text.is_char_boundary(r.end),
            "{tex:?}: italic {i} {r:?} not on char boundaries in {:?}",
            m.text
        );
        pos = r.end;
    }
}

pub fn to_unicode_and_check(tex: &str) -> MathText {
    let m = latex::to_unicode(tex);
    check_math(tex, &m);
    m
}

pub fn highlight_and_check(lang: &str, code: &str) -> Vec<Highlight> {
    let hs = highlight(lang, code);
    check_highlights(code, &hs, lang);
    hs
}
