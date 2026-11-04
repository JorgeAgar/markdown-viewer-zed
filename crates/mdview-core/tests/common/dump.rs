//! A compact, human-readable text dump of the parser output, used for the
//! snapshot tests. Every field of the model shows up, so any change in what
//! `parse` produces changes a snapshot.
//!
//! Format (two spaces of indentation per nesting level):
//!
//! ```text
//! heading h2 #slug
//!   "Some ⟦B|bold⟧ text with a ⟦L0|link⟧"
//!   link 0: https://example.com
//! code rust
//!   |⟦kw|fn⟧ ⟦fn|main⟧() {}
//! ```
//!
//! Inside a `"…"` text line, `⟦STYLE|text⟧` marks a span: `B` bold, `I`
//! italic, `S` strikethrough, `C` code, `L` the link style, followed by the
//! link index when the span is clickable (`L0`); unstyled spans show as plain
//! text. Newlines show as `⏎`, tabs as `⇥`, carriage returns as `␍`, and
//! invisible or combining characters as `⟨U+XXXX⟩`.
//!
//! Code, math and diagram sources are printed line by line after `|`, with
//! highlight tokens as `⟦tok|text⟧` and trailing spaces as `␠`.

#![allow(dead_code)]

use std::fmt::Write;

use mdview_core::{
    highlight::{Highlight, Tok},
    markdown::{Align, BOLD, Block, CODE, Document, ITALIC, LINK, RichText, STRIKE},
};

pub fn document(doc: &Document) -> String {
    let mut out = String::new();
    if doc.blocks.is_empty() {
        out.push_str("(no blocks)\n");
    }
    for b in &doc.blocks {
        block(&mut out, b, 0);
    }
    out
}

fn line(out: &mut String, depth: usize, s: &str) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    out.push_str(s);
    out.push('\n');
}

fn block(out: &mut String, b: &Block, d: usize) {
    match b {
        Block::Heading { level, text, slug } => {
            line(out, d, &format!("heading h{level} #{slug}"));
            rich(out, text, d + 1);
        }
        Block::Paragraph(text) => {
            line(out, d, "paragraph");
            rich(out, text, d + 1);
        }
        Block::Images(images) => {
            line(out, d, "images");
            for i in images {
                line(
                    out,
                    d + 1,
                    &format!("image {} alt=\"{}\"", i.url, escape(&i.alt)),
                );
            }
        }
        Block::Code {
            lang,
            code,
            highlights,
        } => {
            line(
                out,
                d,
                &format!("code {}", if lang.is_empty() { "(no lang)" } else { lang }),
            );
            source(out, &tokens(code, highlights), d + 1);
        }
        Block::Quote { callout, blocks } => {
            match callout {
                Some(c) => line(out, d, &format!("quote [!{c:?}]")),
                None => line(out, d, "quote"),
            }
            for b in blocks {
                block(out, b, d + 1);
            }
        }
        Block::List { start, items } => {
            match start {
                Some(n) => line(out, d, &format!("list ordered start={n}")),
                None => line(out, d, "list bullet"),
            }
            for item in items {
                match item.checked {
                    Some(true) => line(out, d + 1, "item [x]"),
                    Some(false) => line(out, d + 1, "item [ ]"),
                    None => line(out, d + 1, "item"),
                }
                for b in &item.blocks {
                    block(out, b, d + 2);
                }
            }
        }
        Block::Table {
            aligns,
            header,
            rows,
        } => {
            let aligns: Vec<&str> = aligns
                .iter()
                .map(|a| match a {
                    Align::None => "none",
                    Align::Left => "left",
                    Align::Center => "center",
                    Align::Right => "right",
                })
                .collect();
            line(out, d, &format!("table aligns=[{}]", aligns.join(", ")));
            line(out, d + 1, "header");
            for c in header {
                rich(out, c, d + 2);
            }
            for (i, r) in rows.iter().enumerate() {
                line(out, d + 1, &format!("row {i}"));
                for c in r {
                    rich(out, c, d + 2);
                }
            }
        }
        Block::Footnote { label, blocks } => {
            line(out, d, &format!("footnote [^{label}]"));
            for b in blocks {
                block(out, b, d + 1);
            }
        }
        Block::Math(tex) => {
            line(out, d, "math");
            source(out, tex, d + 1);
        }
        Block::Mermaid(src) => {
            line(out, d, "mermaid");
            source(out, src, d + 1);
        }
        Block::Rule => line(out, d, "rule"),
    }
}

/// One `"…"` line with the styled text, then one line per link.
pub fn rich(out: &mut String, rt: &RichText, d: usize) {
    line(out, d, &format!("\"{}\"", styled(rt)));
    for (i, l) in rt.links.iter().enumerate() {
        line(out, d, &format!("link {i}: {l}"));
    }
}

/// The text with `⟦STYLE|…⟧` around each styled span.
pub fn styled(rt: &RichText) -> String {
    let mut s = String::new();
    let mut pos = 0;
    for span in &rt.spans {
        s.push_str(&escape(&rt.text[pos..span.range.start]));
        let mut tag = String::new();
        for (bit, c) in [(BOLD, 'B'), (ITALIC, 'I'), (STRIKE, 'S'), (CODE, 'C')] {
            if span.style & bit != 0 {
                tag.push(c);
            }
        }
        if span.style & LINK != 0 {
            tag.push('L');
        }
        if let Some(l) = span.link {
            write!(tag, "{l}").unwrap();
        }
        let text = escape(&rt.text[span.range.clone()]);
        if tag.is_empty() {
            s.push_str(&text);
        } else {
            write!(s, "⟦{tag}|{text}⟧").unwrap();
        }
        pos = span.range.end;
    }
    s.push_str(&escape(&rt.text[pos..]));
    s
}

/// Code with `⟦tok|…⟧` around each highlight.
pub fn tokens(code: &str, hs: &[Highlight]) -> String {
    let mut s = String::new();
    let mut pos = 0;
    for h in hs {
        s.push_str(&code[pos..h.range.start]);
        write!(s, "⟦{}|{}⟧", tok_name(h.tok), &code[h.range.clone()]).unwrap();
        pos = h.range.end;
    }
    s.push_str(&code[pos..]);
    s
}

pub fn tok_name(t: Tok) -> &'static str {
    match t {
        Tok::Keyword => "kw",
        Tok::Type => "ty",
        Tok::Function => "fn",
        Tok::String => "str",
        Tok::Number => "num",
        Tok::Comment => "com",
        Tok::Constant => "const",
        Tok::Attr => "attr",
        Tok::Variable => "var",
        Tok::Tag => "tag",
        Tok::Property => "prop",
        Tok::Inserted => "ins",
        Tok::Deleted => "del",
        Tok::Meta => "meta",
    }
}

/// Multi-line source after `|`, one line per source line.
pub fn source(out: &mut String, src: &str, d: usize) {
    for l in src.split('\n') {
        let trimmed = l.trim_end_matches(' ');
        let mut s = String::from("|");
        for c in trimmed.chars() {
            push_char(&mut s, c, false);
        }
        for _ in trimmed.len()..l.len() {
            s.push('␠');
        }
        line(out, d, &s);
    }
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_char(&mut out, c, true);
    }
    out
}

fn push_char(out: &mut String, c: char, newline: bool) {
    match c {
        '\n' if newline => out.push('⏎'),
        '\t' => out.push('⇥'),
        '\r' => out.push('␍'),
        c if invisible(c) => write!(out, "⟨U+{:04X}⟩", c as u32).unwrap(),
        c => out.push(c),
    }
}

/// Characters a reviewer couldn't see (or tell apart) in a snapshot.
fn invisible(c: char) -> bool {
    c.is_control()
        || (c.is_whitespace() && c != ' ')
        || matches!(c as u32,
            0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x180E
            | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
            | 0xFE00..=0xFE0F | 0xFEFF
            // Combining marks.
            | 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F)
}
