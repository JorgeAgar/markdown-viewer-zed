//! Markdown -> lightweight block model.
//!
//! Parsing happens once per file load (off the UI thread for the initial
//! files). Rendering then only walks this pre-digested tree, so a frame never
//! touches the raw Markdown again.

use std::{ops::Range, sync::Arc};

use crate::{
    highlight::{Highlight, highlight},
    latex,
};

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag,
    TagEnd,
};

/// Inline style bits.
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 1 << 1;
pub const STRIKE: u8 = 1 << 2;
pub const CODE: u8 = 1 << 3;
pub const LINK: u8 = 1 << 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: u8,
    /// Index into `RichText::links`.
    pub link: Option<u32>,
}

/// A run of styled text (paragraph, heading, table cell...).
#[derive(Clone, Debug, Default)]
pub struct RichText {
    pub text: String,
    pub spans: Vec<Span>,
    pub links: Vec<String>,
}

impl RichText {
    fn push(&mut self, s: &str, style: u8, link: Option<u32>) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        let end = self.text.len();
        if let Some(last) = self.spans.last_mut()
            && last.style == style
            && last.link == link
            && last.range.end == start
        {
            last.range.end = end;
            return;
        }
        self.spans.push(Span {
            range: start..end,
            style,
            link,
        });
    }

    /// Byte ranges of each link, in the same order as `links`... but one range
    /// per span (a link can be split by styles), paired with the link index.
    pub fn link_ranges(&self) -> (Vec<Range<usize>>, Vec<u32>) {
        let mut ranges = Vec::new();
        let mut ids = Vec::new();
        for span in &self.spans {
            if let Some(l) = span.link {
                ranges.push(span.range.clone());
                ids.push(l);
            }
        }
        (ranges, ids)
    }

    /// Appends inline math, converted to Unicode text (see `latex.rs`).
    fn push_math(&mut self, tex: &str, style: u8, link: Option<u32>) {
        let m = latex::to_unicode(tex);
        let mut pos = 0;
        for r in &m.italic {
            self.push(&m.text[pos..r.start], style, link);
            self.push(&m.text[r.clone()], style | ITALIC, link);
            pos = r.end;
        }
        self.push(&m.text[pos..], style, link);
    }

    fn trim_start(&mut self) {
        let cut = self.text.len() - self.text.trim_start().len();
        if cut == 0 {
            return;
        }
        self.text.drain(..cut);
        self.spans.retain_mut(|s| {
            s.range.start = s.range.start.saturating_sub(cut);
            s.range.end = s.range.end.saturating_sub(cut);
            s.range.start < s.range.end
        });
    }

    fn trim_end(&mut self) {
        let trimmed = self.text.trim_end().len();
        self.text.truncate(trimmed);
        self.spans.retain_mut(|s| {
            s.range.end = s.range.end.min(trimmed);
            s.range.start < s.range.end
        });
    }
}

#[derive(Clone, Debug)]
pub struct Image {
    pub url: String,
    pub alt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callout {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Debug)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug)]
pub enum Block {
    Heading {
        level: u8,
        text: RichText,
        slug: String,
    },
    Paragraph(RichText),
    Images(Vec<Image>),
    Code {
        lang: String,
        code: String,
        /// Syntax highlighting, computed once here (see `highlight.rs`).
        highlights: Vec<Highlight>,
    },
    Quote {
        callout: Option<Callout>,
        blocks: Vec<Block>,
    },
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Table {
        aligns: Vec<Align>,
        header: Vec<RichText>,
        rows: Vec<Vec<RichText>>,
    },
    Footnote {
        label: String,
        blocks: Vec<Block>,
    },
    /// Display math (`$$…$$` or a ```` ```math ```` block): LaTeX source,
    /// typeset as an image by `graphics.rs`.
    Math(Arc<str>),
    Rule,
}

pub struct Document {
    pub blocks: Vec<Block>,
}

pub fn parse(src: &str) -> Document {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_MATH
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    let events: Vec<Event> = Parser::new_ext(src, opts).collect();
    let mut p = P { ev: events, i: 0 };
    let mut blocks = Vec::new();
    p.blocks(&mut blocks);
    Document { blocks }
}

struct P<'a> {
    ev: Vec<Event<'a>>,
    i: usize,
}

fn is_inline(ev: &Event) -> bool {
    match ev {
        Event::Text(_)
        | Event::Code(_)
        | Event::InlineHtml(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::FootnoteReference(_)
        | Event::TaskListMarker(_)
        | Event::InlineMath(_)
        | Event::DisplayMath(_) => true,
        Event::Start(t) => matches!(
            t,
            Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Link { .. }
                | Tag::Image { .. }
                | Tag::Superscript
                | Tag::Subscript
        ),
        Event::End(t) => matches!(
            t,
            TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Link
                | TagEnd::Image
                | TagEnd::Superscript
                | TagEnd::Subscript
        ),
        _ => false,
    }
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<&Event<'a>> {
        self.ev.get(self.i)
    }

    fn next(&mut self) -> Option<Event<'a>> {
        let e = self.ev.get_mut(self.i)?;
        self.i += 1;
        Some(std::mem::replace(e, Event::SoftBreak))
    }

    /// Parse blocks until the enclosing container's End event (consumed) or EOF.
    fn blocks(&mut self, out: &mut Vec<Block>) {
        while let Some(ev) = self.peek() {
            if is_inline(ev) {
                // Tight list items emit inline content without a Paragraph.
                self.paragraph(out);
                continue;
            }
            match self.next().unwrap() {
                Event::End(_) => return,
                Event::Start(tag) => self.block(tag, out),
                Event::Rule => out.push(Block::Rule),
                Event::Html(_) => {}
                _ => {}
            }
        }
    }

    fn block(&mut self, tag: Tag<'a>, out: &mut Vec<Block>) {
        match tag {
            Tag::Paragraph => {
                self.paragraph(out);
                self.next(); // End(Paragraph)
            }
            Tag::Heading { level, .. } => {
                let (mut text, _) = self.inlines(false);
                self.next();
                text.trim_end();
                let slug = slugify(&text.text);
                out.push(Block::Heading {
                    level: heading_level(level),
                    text,
                    slug,
                });
            }
            Tag::BlockQuote(kind) => {
                let mut blocks = Vec::new();
                self.blocks(&mut blocks);
                let callout = kind.map(|k| match k {
                    BlockQuoteKind::Note => Callout::Note,
                    BlockQuoteKind::Tip => Callout::Tip,
                    BlockQuoteKind::Important => Callout::Important,
                    BlockQuoteKind::Warning => Callout::Warning,
                    BlockQuoteKind::Caution => Callout::Caution,
                });
                out.push(Block::Quote { callout, blocks });
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => {
                        l.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                while let Some(ev) = self.next() {
                    match ev {
                        Event::Text(t) => code.push_str(&t),
                        Event::End(_) => break,
                        _ => {}
                    }
                }
                if code.ends_with('\n') {
                    code.pop();
                }
                if lang.eq_ignore_ascii_case("math") {
                    out.push(Block::Math(code.into()));
                    return;
                }
                let highlights = highlight(&lang, &code);
                out.push(Block::Code {
                    lang,
                    code,
                    highlights,
                });
            }
            Tag::List(start) => {
                let mut items = Vec::new();
                while let Some(ev) = self.next() {
                    match ev {
                        Event::Start(Tag::Item) => {
                            let mut checked = None;
                            // Tight: Item, TaskListMarker. Loose: Item, Paragraph, TaskListMarker.
                            if let Some(Event::TaskListMarker(c)) = self.peek() {
                                checked = Some(*c);
                                self.i += 1;
                            } else if matches!(self.peek(), Some(Event::Start(Tag::Paragraph)))
                                && let Some(Event::TaskListMarker(c)) = self.ev.get(self.i + 1)
                            {
                                checked = Some(*c);
                                self.ev.remove(self.i + 1);
                            }
                            let mut blocks = Vec::new();
                            self.blocks(&mut blocks);
                            items.push(ListItem { checked, blocks });
                        }
                        _ => break, // End(List)
                    }
                }
                out.push(Block::List { start, items });
            }
            Tag::Table(aligns) => {
                let aligns = aligns
                    .into_iter()
                    .map(|a| match a {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect();
                let mut header = Vec::new();
                let mut rows = Vec::new();
                while let Some(ev) = self.next() {
                    match ev {
                        Event::Start(Tag::TableHead) => header = self.cells(),
                        Event::Start(Tag::TableRow) => rows.push(self.cells()),
                        Event::End(TagEnd::Table) => break,
                        _ => {}
                    }
                }
                out.push(Block::Table {
                    aligns,
                    header,
                    rows,
                });
            }
            Tag::FootnoteDefinition(label) => {
                let mut blocks = Vec::new();
                self.blocks(&mut blocks);
                out.push(Block::Footnote {
                    label: label.to_string(),
                    blocks,
                });
            }
            Tag::HtmlBlock | Tag::MetadataBlock(_) => self.skip_to_end(),
            // Anything else we don't specifically render: keep its children.
            _ => self.blocks(out),
        }
    }

    fn skip_to_end(&mut self) {
        let mut depth = 0;
        while let Some(ev) = self.next() {
            match ev {
                Event::Start(_) => depth += 1,
                Event::End(_) if depth == 0 => return,
                Event::End(_) => depth -= 1,
                _ => {}
            }
        }
    }

    fn cells(&mut self) -> Vec<RichText> {
        let mut cells = Vec::new();
        while let Some(ev) = self.next() {
            match ev {
                Event::Start(Tag::TableCell) => {
                    let (rich, _) = self.inlines(false);
                    self.next();
                    cells.push(rich);
                }
                _ => break, // End(TableHead | TableRow)
            }
        }
        cells
    }

    /// A paragraph's inline content. Display math (`$$…$$`) splits it: the
    /// text before and after become paragraphs and the math its own block.
    fn paragraph(&mut self, out: &mut Vec<Block>) {
        loop {
            let (rich, images) = self.inlines(true);
            push_paragraph(out, rich, images);
            match self.peek() {
                Some(Event::DisplayMath(_)) => {
                    if let Some(Event::DisplayMath(tex)) = self.next() {
                        out.push(Block::Math(tex.trim().into()));
                    }
                }
                Some(ev) if is_inline(ev) => {}
                _ => return,
            }
        }
    }

    /// Consume inline events. Stops (without consuming) at the first
    /// non-inline event, or at display math when `split_display` is set.
    fn inlines(&mut self, split_display: bool) -> (RichText, Vec<Image>) {
        let mut rich = RichText::default();
        let mut images = Vec::new();
        let mut style = 0u8;
        let mut counts = [0u8; 5];
        let mut link: Option<u32> = None;
        let mut image: Option<Image> = None;

        fn bit(idx: usize) -> u8 {
            1 << idx
        }

        while let Some(ev) = self.peek() {
            if !is_inline(ev) || (split_display && matches!(ev, Event::DisplayMath(_))) {
                break;
            }
            let ev = self.next().unwrap();
            let mut toggle = |idx: usize, on: bool, style: &mut u8| {
                if on {
                    counts[idx] += 1;
                } else {
                    counts[idx] = counts[idx].saturating_sub(1);
                }
                if counts[idx] > 0 {
                    *style |= bit(idx);
                } else {
                    *style &= !bit(idx);
                }
            };
            match ev {
                Event::Text(t) => {
                    if let Some(img) = image.as_mut() {
                        img.alt.push_str(&t);
                    } else {
                        rich.push(&t, style, link);
                    }
                }
                Event::Code(t) => rich.push(&t, style | CODE, link),
                Event::InlineMath(t) | Event::DisplayMath(t) => rich.push_math(&t, style, link),
                Event::InlineHtml(h) => {
                    if h.eq_ignore_ascii_case("<br>") || h.eq_ignore_ascii_case("<br/>") {
                        rich.push("\n", style, link);
                    }
                }
                Event::SoftBreak => {
                    if image.is_none() {
                        rich.push(" ", style, link)
                    }
                }
                Event::HardBreak => rich.push("\n", style, link),
                Event::FootnoteReference(l) => {
                    rich.push(&format!("[{l}]"), style | LINK, link);
                }
                Event::TaskListMarker(c) => {
                    rich.push(if c { "☑ " } else { "☐ " }, style, link);
                }
                Event::Start(Tag::Strong) => toggle(0, true, &mut style),
                Event::End(TagEnd::Strong) => toggle(0, false, &mut style),
                Event::Start(Tag::Emphasis) => toggle(1, true, &mut style),
                Event::End(TagEnd::Emphasis) => toggle(1, false, &mut style),
                Event::Start(Tag::Strikethrough) => toggle(2, true, &mut style),
                Event::End(TagEnd::Strikethrough) => toggle(2, false, &mut style),
                Event::Start(Tag::Link { dest_url, .. }) => {
                    rich.links.push(dest_url.to_string());
                    link = Some(rich.links.len() as u32 - 1);
                    style |= LINK;
                }
                Event::End(TagEnd::Link) => {
                    link = None;
                    style &= !LINK;
                }
                Event::Start(Tag::Image { dest_url, .. }) => {
                    image = Some(Image {
                        url: cow(dest_url),
                        alt: String::new(),
                    });
                }
                Event::End(TagEnd::Image) => {
                    if let Some(img) = image.take() {
                        // Keep a textual fallback in the flow; the paragraph
                        // becomes a pure image block if that's all there is.
                        let label = if img.alt.is_empty() {
                            "image".to_string()
                        } else {
                            img.alt.clone()
                        };
                        if link.is_none() {
                            rich.links.push(img.url.clone());
                            let l = Some(rich.links.len() as u32 - 1);
                            rich.push(&format!("[{label}]"), style | LINK, l);
                        } else {
                            rich.push(&format!("[{label}]"), style, link);
                        }
                        images.push(img);
                    }
                }
                _ => {}
            }
        }
        (rich, images)
    }
}

fn cow(s: CowStr) -> String {
    s.to_string()
}

fn push_paragraph(out: &mut Vec<Block>, mut rich: RichText, images: Vec<Image>) {
    if !images.is_empty() {
        // Paragraph made only of images (and whitespace) => image block.
        let only_images = rich
            .spans
            .iter()
            .all(|s| s.style & LINK != 0 || rich.text[s.range.clone()].trim().is_empty());
        if only_images {
            out.push(Block::Images(images));
            return;
        }
    }
    rich.trim_start();
    rich.trim_end();
    if !rich.text.is_empty() {
        out.push(Block::Paragraph(rich));
    }
}

fn heading_level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// GitHub-style anchor slug.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            out.extend(c.to_lowercase());
        } else if c == ' ' {
            out.push('-');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_document() {
        let doc = parse(
            "# Title\n\nHello **bold** and `code` [link](https://x.y).\n\n- [x] done\n- [ ] todo\n\n> [!NOTE]\n> hi\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\n![alt](img.png)\n",
        );
        let kinds: Vec<&str> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Heading { .. } => "h",
                Block::Paragraph(_) => "p",
                Block::Images(_) => "img",
                Block::Code { .. } => "code",
                Block::Quote { .. } => "quote",
                Block::List { .. } => "list",
                Block::Table { .. } => "table",
                Block::Footnote { .. } => "fn",
                Block::Math(_) => "math",
                Block::Rule => "hr",
            })
            .collect();
        assert_eq!(kinds, ["h", "p", "list", "quote", "table", "code", "img"]);
        if let Block::List { items, .. } = &doc.blocks[2] {
            assert_eq!(items[0].checked, Some(true));
            assert_eq!(items[1].checked, Some(false));
        }
        if let Block::Quote { callout, .. } = &doc.blocks[3] {
            assert_eq!(*callout, Some(Callout::Note));
        }
        if let Block::Paragraph(p) = &doc.blocks[1] {
            assert_eq!(p.text, "Hello bold and code link.");
            assert_eq!(p.links, ["https://x.y"]);
        }
    }

    #[test]
    fn parses_math() {
        let doc = parse(
            "Energy $E = mc^2$ here.\n\n$$\n\\int_0^1 x\\,dx\n$$\n\nBefore\n$$a^2$$\nafter\n\n```math\n\\frac{1}{2}\n```\n",
        );
        let kinds: Vec<String> = doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => format!("p:{}", p.text),
                Block::Math(t) => format!("m:{t}"),
                _ => "?".into(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "p:Energy E = mc² here.",
                "m:\\int_0^1 x\\,dx",
                "p:Before",
                "m:a^2",
                "p:after",
                "m:\\frac{1}{2}",
            ]
        );
        if let Block::Paragraph(p) = &doc.blocks[0] {
            let italic: Vec<&str> = p
                .spans
                .iter()
                .filter(|s| s.style & ITALIC != 0)
                .map(|s| &p.text[s.range.clone()])
                .collect();
            assert_eq!(italic, ["E", "mc"]);
        }
    }
}
