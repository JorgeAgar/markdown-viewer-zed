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
    Alignment, BlockQuoteKind, CodeBlockKind, CowStr, Event, HeadingLevel, LinkType, Options,
    Parser, Tag, TagEnd,
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
    /// A ```` ```mermaid ```` block: diagram source, rendered as an image by
    /// `graphics.rs`.
    Mermaid(Arc<str>),
    Rule,
}

pub struct Document {
    pub blocks: Vec<Block>,
}

pub fn parse(src: &str) -> Document {
    // Editors on Windows like to start UTF-8 files with a byte order mark;
    // left in, it would turn a first `# Heading` into a paragraph.
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    // Front matter isn't shown. pulldown-cmark's metadata-block options
    // would also swallow a `---` line, text and a `---` line anywhere in the
    // document (really a rule and a setext heading), so cut it off here and
    // parse the rest without them.
    let src = &src[front_matter_len(src)..];
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_MATH;
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
                if lang.eq_ignore_ascii_case("mermaid") {
                    out.push(Block::Mermaid(code.into()));
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
            Tag::HtmlBlock => self.skip_to_end(),
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
        // Images nested inside the open image's description.
        let mut nested_images = 0u32;

        fn bit(idx: usize) -> u8 {
            1 << idx
        }

        while let Some(ev) = self.peek() {
            if !is_inline(ev)
                || (split_display && image.is_none() && matches!(ev, Event::DisplayMath(_)))
            {
                break;
            }
            let ev = self.next().unwrap();
            // Inside an image description everything is alt text: keep the
            // text of code, math, links and nested images, drop the markup.
            if let Some(img) = image.as_mut() {
                match ev {
                    Event::Text(t) | Event::Code(t) => img.alt.push_str(&t),
                    Event::InlineMath(t) | Event::DisplayMath(t) => {
                        img.alt.push_str(&latex::to_unicode(&t).text)
                    }
                    Event::SoftBreak | Event::HardBreak => img.alt.push(' '),
                    Event::Start(Tag::Image { .. }) => nested_images += 1,
                    Event::End(TagEnd::Image) if nested_images > 0 => nested_images -= 1,
                    Event::End(TagEnd::Image) => {
                        let img = image.take().unwrap();
                        // Keep a textual fallback in the flow; the paragraph
                        // becomes a pure image block if that's all there is.
                        let label = if img.alt.is_empty() {
                            "image"
                        } else {
                            &img.alt
                        };
                        let label = format!("[{label}]");
                        if link.is_none() {
                            rich.links.push(img.url.clone());
                            let l = Some(rich.links.len() as u32 - 1);
                            rich.push(&label, style | LINK, l);
                        } else {
                            rich.push(&label, style, link);
                        }
                        images.push(img);
                    }
                    _ => {}
                }
                continue;
            }
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
                Event::Text(t) => rich.push(&t, style, link),
                Event::Code(t) => rich.push(&t, style | CODE, link),
                Event::InlineMath(t) | Event::DisplayMath(t) => rich.push_math(&t, style, link),
                Event::InlineHtml(h) => {
                    if is_br(&h) {
                        rich.push("\n", style, link);
                    }
                }
                Event::SoftBreak => rich.push(" ", style, link),
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
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    ..
                }) => {
                    // `<me@example.com>` comes without its scheme; without
                    // one, `render::open_link` would look for a local file.
                    rich.links.push(if link_type == LinkType::Email {
                        format!("mailto:{dest_url}")
                    } else {
                        dest_url.to_string()
                    });
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
                _ => {}
            }
        }
        (rich, images)
    }
}

/// Length of the YAML (`---`) or TOML (`+++`) front matter at the start of
/// `src`, including its closing line; 0 if there is none. Same rules as
/// pulldown-cmark's metadata blocks: an opening line of exactly three
/// delimiters (plus whitespace), a first line that is neither blank nor the
/// closing line, and a closing line of exactly three delimiters (or `...`
/// for YAML) followed only by spaces.
fn front_matter_len(src: &str) -> usize {
    let delim = match src.as_bytes().first() {
        Some(b'-') => "---",
        Some(b'+') => "+++",
        _ => return 0,
    };
    let mut lines = src.split_inclusive('\n');
    let Some(rest) = lines.next().and_then(|l| l.strip_prefix(delim)) else {
        return 0;
    };
    if !rest.bytes().all(|b| b.is_ascii_whitespace()) {
        return 0;
    }
    // Length of `line` up to the end of its line ending if it closes the
    // block. (Lines are split at `\n` only, like pulldown-cmark does, but a
    // lone `\r` also ends a closing line.)
    let closing = |line: &str| {
        let rest = line
            .strip_prefix(delim)
            .or_else(|| (delim == "---").then(|| line.strip_prefix("...")).flatten())?
            .trim_start_matches(' ');
        let eol = if rest.starts_with("\r\n") {
            2
        } else if rest.starts_with(['\r', '\n']) {
            1
        } else if rest.is_empty() {
            0
        } else {
            return None;
        };
        Some(line.len() - rest.len() + eol)
    };
    let mut len = delim.len() + rest.len();
    for (i, line) in lines.enumerate() {
        let close = closing(line);
        if i == 0
            && (close.is_some()
                || line
                    .trim_start_matches([' ', '\t'])
                    .starts_with(['\n', '\r']))
        {
            return 0;
        }
        if let Some(n) = close {
            return len + n;
        }
        len += line.len();
    }
    0
}

/// `<br>`, `<br/>`, `<br />`, `<BR >`...
fn is_br(html: &str) -> bool {
    html.strip_prefix('<')
        .and_then(|h| h.strip_suffix('>'))
        .map(|h| h.trim_end().strip_suffix('/').unwrap_or(h).trim_end())
        .is_some_and(|h| h.eq_ignore_ascii_case("br"))
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
            "# Title\n\nHello **bold** and `code` [link](https://x.y).\n\n- [x] done\n- [ ] todo\n\n> [!NOTE]\n> hi\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\n![alt](img.png)\n\n```mermaid\nflowchart LR; A-->B\n```\n",
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
                Block::Mermaid(_) => "mermaid",
                Block::Rule => "hr",
            })
            .collect();
        assert_eq!(
            kinds,
            ["h", "p", "list", "quote", "table", "code", "img", "mermaid"]
        );
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

    #[test]
    fn mathit_in_inline_math_does_not_panic() {
        let doc = parse("A $\\mathit{xy}$ b");
        let Block::Paragraph(p) = &doc.blocks[0] else {
            panic!("expected a paragraph")
        };
        assert_eq!(p.text, "A xy b");
        let italic: Vec<&str> = p
            .spans
            .iter()
            .filter(|s| s.style & ITALIC != 0)
            .map(|s| &p.text[s.range.clone()])
            .collect();
        assert_eq!(italic, ["xy"]);
    }

    #[test]
    fn image_description_is_alt_text() {
        // Code, math, links, line breaks and nested images inside an image's
        // description used to leak into the paragraph (or replace the outer
        // image); they are all part of the alt text.
        let doc = parse("![a `b` *c* [l](u) ![n](m) $x$\nd](img.png)\n\n![p\n$$q$$ r](s.png)");
        let alts: Vec<(&str, &str)> = doc
            .blocks
            .iter()
            .flat_map(|b| match b {
                Block::Images(imgs) => imgs.iter().map(|i| (i.url.as_str(), i.alt.as_str())),
                other => panic!("expected only images, got {other:?}"),
            })
            .collect();
        assert_eq!(alts, [("img.png", "a b c l n x d"), ("s.png", "p q r")]);

        let doc = parse("see ![a `b`](i.png) [![c](j.png) d](u)");
        let Block::Paragraph(p) = &doc.blocks[0] else {
            panic!("expected a paragraph")
        };
        assert_eq!(p.text, "see [a b] [c] d");
        assert_eq!(p.links, ["i.png", "u"]);
    }

    #[test]
    fn email_autolinks_get_a_mailto_scheme() {
        let doc = parse("<me@example.com> <mailto:you@example.com> <https://x.y> [m](me@x.y)");
        let Block::Paragraph(p) = &doc.blocks[0] else {
            panic!("expected a paragraph")
        };
        assert_eq!(
            p.text,
            "me@example.com mailto:you@example.com https://x.y m"
        );
        assert_eq!(
            p.links,
            [
                "mailto:me@example.com",
                "mailto:you@example.com",
                "https://x.y",
                "me@x.y"
            ]
        );
    }

    #[test]
    fn front_matter_only_at_the_start() {
        fn texts(src: &str) -> Vec<String> {
            parse(src)
                .blocks
                .iter()
                .map(|b| match b {
                    Block::Heading { level, text, .. } => format!("h{level}:{}", text.text),
                    Block::Paragraph(p) => format!("p:{}", p.text),
                    Block::Rule => "hr".into(),
                    other => format!("{other:?}"),
                })
                .collect()
        }
        // Front matter is hidden...
        assert_eq!(texts("---\ntitle: x\n---\nText"), ["p:Text"]);
        assert_eq!(texts("---\r\na: 1\r\n...\r\nText"), ["p:Text"]);
        assert_eq!(texts("+++ \nb = 2\n+++  \n# H"), ["h1:H"]);
        assert_eq!(texts("---\na\n\nb\n---\n"), Vec::<String>::new());
        // ...but only at the very start: later it's a rule and a heading.
        assert_eq!(
            texts("Intro\n\n---\ntitle: x\n---\n\nAfter"),
            ["p:Intro", "hr", "h2:title: x", "p:After"]
        );
        assert_eq!(texts("# H\n\n+++\nb\n+++\n"), ["h1:H", "p:+++ b +++"]);
        // Not front matter: blank or closing first line, wrong delimiters,
        // indentation, never closed.
        assert_eq!(texts("---\n\nx\n---\n"), ["hr", "h2:x"]);
        assert_eq!(texts("---\n---\n"), ["hr", "hr"]);
        assert_eq!(texts("----\nx\n----\n"), ["hr", "h2:x"]);
        assert_eq!(texts("--- x\ny\n---\n"), ["h2:--- x y"]);
        assert_eq!(texts("---\nx\n ---\n"), ["hr", "h2:x"]);
        assert_eq!(texts("---\nx\n----\n"), ["hr", "h2:x"]);
        assert_eq!(texts("+++\nx\n...\n"), ["p:+++ x ..."]);
        assert_eq!(texts("---\nx\n"), ["hr", "p:x"]);
        assert_eq!(texts("---"), ["hr"]);
        assert_eq!(texts("--"), ["p:--"]);
        assert_eq!(texts(""), Vec::<String>::new());
    }

    proptest::proptest! {
        /// `front_matter_len` agrees with pulldown-cmark's own metadata-block
        /// scanner on what the document starts with.
        #[test]
        fn front_matter_len_matches_pulldown(
            parts in proptest::collection::vec(
                proptest::sample::select(&[
                    "---", "+++", "...", "-", "+", ".", "\n", "\r\n", "\r", " ", "\t",
                    "x", "a: 1", "#",
                ][..]),
                0..16,
            )
        ) {
            let src = parts.concat();
            let opts = Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
                | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
            let pulldown = match Parser::new_ext(&src, opts).into_offset_iter().next() {
                // pulldown-cmark also accepts it after blank lines; GitHub
                // (and Jekyll) only on the first line, like us.
                Some((Event::Start(Tag::MetadataBlock(_)), r)) if r.start == 0 => Some(r.end),
                _ => None,
            };
            let len = front_matter_len(&src);
            match pulldown {
                // pulldown's range stops before the closing line's ending.
                Some(end) => proptest::prop_assert!(
                    len >= end && matches!(&src[end..len], "" | "\n" | "\r" | "\r\n"),
                    "pulldown: ..{end}, ours: ..{len}"
                ),
                None => proptest::prop_assert_eq!(len, 0),
            }
        }
    }

    #[test]
    fn leading_byte_order_mark_is_ignored() {
        let doc = parse("\u{feff}# Title\n\nText \u{feff} kept.");
        let Block::Heading { text, slug, .. } = &doc.blocks[0] else {
            panic!("expected a heading, got {:?}", doc.blocks[0])
        };
        assert_eq!((text.text.as_str(), slug.as_str()), ("Title", "title"));
        let Block::Paragraph(p) = &doc.blocks[1] else {
            panic!("expected a paragraph")
        };
        // Only a leading BOM is a BOM.
        assert_eq!(p.text, "Text \u{feff} kept.");
    }

    #[test]
    fn br_tags_are_line_breaks() {
        let doc = parse("a<br>b<br/>c<br />d<BR >e<Br/ >f<br\n/>g<bR\t/>h<brx>i<br x>j</br>k");
        let Block::Paragraph(p) = &doc.blocks[0] else {
            panic!("expected a paragraph")
        };
        // `<Br/ >` isn't an HTML tag for CommonMark, so it stays text.
        assert_eq!(p.text, "a\nb\nc\nd\ne<Br/ >f\ng\nhijk");
    }
}
