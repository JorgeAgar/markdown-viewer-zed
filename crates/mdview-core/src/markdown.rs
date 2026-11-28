//! Markdown -> lightweight block model.
//!
//! Parsing happens once per file load (off the UI thread for the initial
//! files). Rendering then only walks this pre-digested tree, so a frame never
//! touches the raw Markdown again.

use std::{collections::HashMap, ops::Range, sync::Arc};

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

/// What clicking a span of a [`RichText`] does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// A link's destination as written (`https://…`, `other.md#x`, `#x`,
    /// an image's URL), routed by `links::classify_link` in the app.
    Url(String),
    /// A footnote reference (`[^label]`): jumps to the `Block::Footnote`
    /// with that label, matched as pulldown-cmark matches them (ignoring
    /// case). A variant of its own, so it can't be mistaken for a heading
    /// anchor or a URL, whatever the label says.
    Footnote(String),
}

/// A run of styled text (paragraph, heading, table cell...).
#[derive(Clone, Debug, Default)]
pub struct RichText {
    pub text: String,
    pub spans: Vec<Span>,
    pub links: Vec<Link>,
}

impl RichText {
    /// Adds `link` to `links` and returns its index for a span.
    fn add_link(&mut self, link: Link) -> Option<u32> {
        self.links.push(link);
        Some(self.links.len() as u32 - 1)
    }

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

    /// Drops the `links` no span points to (a link whose text was all
    /// trimmed away, or one carried past display math that ends right
    /// there) and renumbers the spans' links to match.
    fn drop_unused_links(&mut self) {
        let mut used = vec![false; self.links.len()];
        for s in &self.spans {
            if let Some(l) = s.link {
                used[l as usize] = true;
            }
        }
        if used.iter().all(|&u| u) {
            return;
        }
        // New index of each link: how many used links come before it.
        let renumber: Vec<u32> = used
            .iter()
            .scan(0, |n, &u| {
                let ix = *n;
                *n += u as u32;
                Some(ix)
            })
            .collect();
        for s in &mut self.spans {
            if let Some(l) = s.link.as_mut() {
                *l = renumber[*l as usize];
            }
        }
        let mut used = used.into_iter();
        self.links.retain(|_| used.next() == Some(true));
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
        /// Anchor id, unique within the document (see [`P::unique_slug`]).
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
    let mut p = P {
        ev: events,
        i: 0,
        slugs: HashMap::new(),
    };
    let mut blocks = Vec::new();
    p.blocks(&mut blocks);
    Document { blocks }
}

struct P<'a> {
    ev: Vec<Event<'a>>,
    i: usize,
    /// Every heading slug handed out so far (nested headings included),
    /// with how many times its base slug was repeated.
    slugs: HashMap<String, usize>,
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
                let (mut text, _) = self.inlines(false, &mut Open::default());
                self.next();
                text.trim_end();
                let slug = self.unique_slug(slugify(&text.text));
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
                                // Overwrite in place instead of Vec::remove so
                                // large loose task lists stay O(N).
                                self.ev[self.i + 1] = Event::Text(CowStr::Borrowed(""));
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

    /// `base`, or, if a heading earlier in the document (at any level or
    /// nesting depth) already has that slug, `base-1`, `base-2`... GitHub's
    /// rule (`github-slugger`): the counter is kept per base slug and skips
    /// suffixes that are taken already, so `# Setup 1` followed by two
    /// `# Setup` gives `setup-1`, `setup`, `setup-2`. Every slug in a
    /// document is unique.
    fn unique_slug(&mut self, base: String) -> String {
        let mut slug = base.clone();
        while self.slugs.contains_key(&slug) {
            let n = self.slugs.get_mut(&base).expect("base slug was seen");
            *n += 1;
            slug = format!("{base}-{n}");
        }
        self.slugs.insert(slug.clone(), 0);
        slug
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
                    let (rich, _) = self.inlines(false, &mut Open::default());
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
    /// Emphasis and links open across the formula carry on after it.
    fn paragraph(&mut self, out: &mut Vec<Block>) {
        let mut open = Open::default();
        loop {
            let (rich, images) = self.inlines(true, &mut open);
            push_paragraph(out, rich, images);
            match self.peek() {
                Some(Event::DisplayMath(_)) => {
                    if let Some(Event::DisplayMath(tex)) = self.next() {
                        // pulldown-cmark normalizes CRLF in code blocks but
                        // not here: make the source independent of the
                        // file's line endings.
                        let tex = tex.trim();
                        out.push(Block::Math(if tex.contains('\r') {
                            tex.replace("\r\n", "\n").replace('\r', "\n").into()
                        } else {
                            tex.into()
                        }));
                    }
                }
                Some(ev) if is_inline(ev) => {}
                _ => return,
            }
        }
    }

    /// Consume inline events. Stops (without consuming) at the first
    /// non-inline event, or at display math when `split_display` is set.
    /// Starts with the formatting in `open` and leaves what's still open at
    /// the end there, for the text after the display math.
    fn inlines(&mut self, split_display: bool, open: &mut Open) -> (RichText, Vec<Image>) {
        let mut rich = RichText::default();
        let mut images = Vec::new();
        let Open {
            mut style,
            mut counts,
            link: mut target,
        } = std::mem::take(open);
        // A link still open after display math links this text too, with an
        // entry in this `RichText`'s own `links`.
        let mut link: Option<u32> = target.clone().and_then(|t| rich.add_link(t));
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
                            let l = rich.add_link(Link::Url(img.url.clone()));
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
                    // Its own link even inside another link, so it always
                    // jumps to the footnote; the enclosing link carries on
                    // after it. (pulldown-cmark doesn't nest them today.)
                    let text = format!("[{l}]");
                    let l = rich.add_link(Link::Footnote(l.into_string()));
                    rich.push(&text, style | LINK, l);
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
                    let t = Link::Url(if link_type == LinkType::Email {
                        format!("mailto:{dest_url}")
                    } else {
                        dest_url.to_string()
                    });
                    link = rich.add_link(t.clone());
                    target = Some(t);
                    style |= LINK;
                }
                Event::End(TagEnd::Link) => {
                    link = None;
                    target = None;
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
        *open = Open {
            style,
            counts,
            link: target,
        };
        (rich, images)
    }
}

/// Inline formatting open at some point of a paragraph. Display math splits
/// a paragraph into several `RichText`s (issue #22); this carries over from
/// one to the next, so the text after a formula keeps its emphasis and link.
#[derive(Default)]
struct Open {
    style: u8,
    /// Open strong, emphasis and strikethrough markers. `usize` so that any
    /// nesting depth (issue #39) still matches openers to closers exactly.
    counts: [usize; 3],
    /// The enclosing link's target.
    link: Option<Link>,
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
    rich.drop_unused_links();
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

    /// `rt.links`, with footnotes written as `[^label]`.
    fn links(rt: &RichText) -> Vec<String> {
        rt.links
            .iter()
            .map(|l| match l {
                Link::Url(u) => u.clone(),
                Link::Footnote(f) => format!("[^{f}]"),
            })
            .collect()
    }

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
            assert_eq!(links(p), ["https://x.y"]);
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

    /// Each block as `m:tex` or the paragraph's spans as `text@STYLES>link`.
    fn split_blocks(src: &str) -> Vec<String> {
        parse(src)
            .blocks
            .iter()
            .map(|b| match b {
                Block::Math(t) => format!("m:{t}"),
                Block::Paragraph(p) => {
                    let all = links(p);
                    let spans: Vec<String> = p
                        .spans
                        .iter()
                        .map(|s| {
                            let mut tag = String::new();
                            for (bit, c) in [(BOLD, 'B'), (ITALIC, 'I'), (STRIKE, 'S'), (LINK, 'L')]
                            {
                                if s.style & bit != 0 {
                                    tag.push(c);
                                }
                            }
                            let link = s
                                .link
                                .map_or(String::new(), |l| format!(">{}", all[l as usize]));
                            format!("{}@{tag}{link}", &p.text[s.range.clone()])
                        })
                        .collect();
                    format!("p:{} links={}", spans.join("|"), all.join(","))
                }
                Block::Footnote { .. } => "footnote".into(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn display_math_keeps_the_surrounding_formatting() {
        // Issue #22: the text after a `$$…$$` lost the bold, italic,
        // strikethrough and link it was inside.
        assert_eq!(
            split_blocks("**before $$x^2$$ after**"),
            ["p:before@B links=", "m:x^2", "p:after@B links="]
        );
        assert_eq!(
            split_blocks("*italic $$x^2$$ still italic*"),
            ["p:italic@I links=", "m:x^2", "p:still italic@I links="]
        );
        assert_eq!(
            split_blocks("~~struck $$x^2$$ still struck~~ plain"),
            [
                "p:struck@S links=",
                "m:x^2",
                "p:still struck@S| plain@ links="
            ]
        );
        // The link is registered again in the next paragraph's own links.
        assert_eq!(
            split_blocks("[before $$x^2$$ after](docs.md) out"),
            [
                "p:before@L>docs.md links=docs.md",
                "m:x^2",
                "p:after@L>docs.md| out@ links=docs.md"
            ]
        );
    }

    #[test]
    fn display_math_keeps_nested_formatting_across_several_formulas() {
        assert_eq!(
            split_blocks("**[before $$x^2$$ *mid* $$y^2$$ after](https://e.com)** end"),
            [
                "p:before@BL>https://e.com links=https://e.com",
                "m:x^2",
                "p:mid@BIL>https://e.com links=https://e.com",
                "m:y^2",
                "p:after@BL>https://e.com| end@ links=https://e.com",
            ]
        );
        // Formulas right at the start or end of the formatting.
        assert_eq!(
            split_blocks("a **$$x$$ b** c"),
            ["p:a@ links=", "m:x", "p:b@B| c@ links="]
        );
        assert_eq!(
            split_blocks("a **b $$x$$** c"),
            ["p:a @|b@B links=", "m:x", "p:c@ links="]
        );
        assert_eq!(
            split_blocks("[$$x$$ b](u) c"),
            ["m:x", "p:b@L>u| c@ links=u"]
        );
        assert_eq!(
            split_blocks("[b $$x$$](u) c"),
            ["p:b@L>u links=u", "m:x", "p:c@ links="]
        );
        // A footnote reference and a link between two formulas keep their
        // own targets inside the carried emphasis.
        assert_eq!(
            split_blocks("*a $$x$$ b[^n] [l](u) $$y$$ c*\n\n[^n]: N\n"),
            [
                "p:a@I links=",
                "m:x",
                "p:b@I|[n]@IL>[^n]| @I|l@IL>u links=[^n],u",
                "m:y",
                "p:c@I links=",
                "footnote",
            ]
        );
        // A link whose carried text is all trimmed away leaves no entry.
        assert_eq!(
            split_blocks("[a $$x$$ ](u) c"),
            ["p:a@L>u links=u", "m:x", "p:c@ links="]
        );
        // Nothing leaks into the next paragraph.
        assert_eq!(
            split_blocks("**a $$x$$\n\nb"),
            ["p:**a@ links=", "m:x", "p:b@ links="]
        );
    }

    #[test]
    fn deep_emphasis_nesting_keeps_styles() {
        // Issue #39: the per-style nesting counters were `u8`, so 256 nested
        // markers overflowed (a panic in debug, styles dropped in release).
        let n = 300;
        let style_of = |src: String| {
            let doc = parse(&src);
            let Block::Paragraph(p) = &doc.blocks[0] else {
                panic!("expected a paragraph")
            };
            p.spans
                .iter()
                .map(|s| (p.text[s.range.clone()].to_string(), s.style))
                .collect::<Vec<_>>()
        };
        let wrap = |m: &str| format!("a {}x{} b", m.repeat(n), m.repeat(n));
        let expect = |style: u8| vec![("a ".to_string(), 0), ("x".into(), style), (" b".into(), 0)];
        assert_eq!(style_of(wrap("**")), expect(BOLD));
        assert_eq!(style_of(wrap("__")), expect(BOLD));
        // Strikethrough doesn't nest in GFM; check it still applies deep down.
        let src = format!("a {}~~x~~{} b", "**".repeat(n), "**".repeat(n));
        assert_eq!(style_of(src), expect(BOLD | STRIKE));
        // `*` runs mix strong and emphasis: odd counts leave one emphasis.
        let src = format!("a {}x{} b", "*".repeat(2 * n + 1), "*".repeat(2 * n + 1));
        assert_eq!(style_of(src), expect(BOLD | ITALIC));
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
        assert_eq!(links(p), ["i.png", "u"]);
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
            links(p),
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
    fn display_math_source_has_lf_line_endings() {
        for src in ["$$\r\na\r\nb\r\n$$\r\n", "$$\ra\rb\r$$", "$$\na\nb\n$$\n"] {
            let doc = parse(src);
            let [Block::Math(tex)] = &doc.blocks[..] else {
                panic!("{src:?}: {:?}", doc.blocks)
            };
            assert_eq!(&**tex, "a\nb", "{src:?}");
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

    /// Slugs of every heading, nested ones too, in document order.
    fn slugs(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Heading { slug, .. } => out.push(slug.clone()),
                Block::Quote { blocks, .. } | Block::Footnote { blocks, .. } => slugs(blocks, out),
                Block::List { items, .. } => {
                    for item in items {
                        slugs(&item.blocks, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn all_slugs(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        slugs(&parse(src).blocks, &mut out);
        out
    }

    #[test]
    fn repeated_headings_get_numbered_slugs() {
        // Issue #20: every repeat used to get the same slug, so `#setup-1`
        // led nowhere. The level doesn't matter.
        assert_eq!(
            all_slugs("# Setup\n\n## Setup\n\n### Setup\n\n#### setup\n"),
            ["setup", "setup-1", "setup-2", "setup-3"]
        );
        // A changelog repeats its section names per release.
        assert_eq!(
            all_slugs("## 1.1\n### Added\n### Fixed\n## 1.0\n### Added\n### Fixed\n"),
            ["11", "added", "fixed", "10", "added-1", "fixed-1"]
        );
        // Nested headings share the count with top-level ones.
        assert_eq!(
            all_slugs("# Setup\n\n> ## Setup\n\n- ### Setup\n\n[^n]: # Setup\n\nx[^n]\n"),
            ["setup", "setup-1", "setup-2", "setup-3"]
        );
    }

    #[test]
    fn numbered_slugs_never_collide_with_literal_ones() {
        // github-slugger: the counter skips suffixes already taken by a
        // heading that really says "Setup 1".
        assert_eq!(
            all_slugs("# Setup 1\n# Setup\n# Setup\n# Setup 1\n# Setup-2\n# Setup\n"),
            [
                "setup-1",
                "setup",
                "setup-2",
                "setup-1-1",
                "setup-2-1",
                "setup-3"
            ]
        );
        assert_eq!(
            all_slugs("# A\n# A-1\n# A\n# A\n"),
            ["a", "a-1", "a-2", "a-3"]
        );
    }

    #[test]
    fn empty_slugs_are_numbered_too() {
        // Headings without letters or digits slug to "", as on GitHub.
        assert_eq!(
            all_slugs("# ???\n## !\n#\n# -1\n"),
            ["", "-1", "-2", "-1-1"]
        );
    }

    /// `(text, link)` of each span, with the link written as in `links`.
    fn linked_spans(rt: &RichText) -> Vec<(&str, Option<String>)> {
        let all = links(rt);
        rt.spans
            .iter()
            .map(|s| {
                (
                    &rt.text[s.range.clone()],
                    s.link.map(|l| all[l as usize].clone()),
                )
            })
            .collect()
    }

    #[test]
    fn footnote_references_link_to_their_footnote() {
        // Issue #23: references were styled as links but led nowhere.
        let doc =
            parse("A[^n] b[^Why] [w](#why) c[^missing].\n\n[^n]: N.\n\n[^why]: W.\n\n# why\n");
        let Block::Paragraph(p) = &doc.blocks[0] else {
            panic!("expected a paragraph")
        };
        assert_eq!(p.text, "A[n] b[Why] w c[^missing].");
        assert_eq!(
            p.links,
            [
                Link::Footnote("n".into()),
                Link::Footnote("Why".into()),
                Link::Url("#why".into()),
            ]
        );
        assert_eq!(
            linked_spans(p),
            [
                ("A", None),
                ("[n]", Some("[^n]".into())),
                (" b", None),
                ("[Why]", Some("[^Why]".into())),
                (" ", None),
                ("w", Some("#why".into())),
                // Without a definition it's plain text (pulldown-cmark).
                (" c[^missing].", None),
            ]
        );
        assert!(p.spans[1].style & LINK != 0);
    }

    #[test]
    fn footnote_reference_inside_a_link_keeps_its_own_target() {
        // GFM (and pulldown-cmark) don't let a link contain a footnote
        // reference: the brackets stay text, and only the reference is a
        // link.
        let doc = parse("[see note[^1]](https://example.com)\n\n[^1]: x\n");
        let Block::Paragraph(p) = &doc.blocks[0] else {
            panic!("expected a paragraph")
        };
        assert_eq!(p.text, "[see note[1]](https://example.com)");
        assert_eq!(links(p), ["[^1]"]);

        // Should a parser ever emit one inside a link, the reference still
        // jumps to its footnote and the rest of the link text still goes to
        // the URL, styles and all.
        let link = Event::Start(Tag::Link {
            link_type: LinkType::Inline,
            dest_url: "https://example.com".into(),
            title: "".into(),
            id: "".into(),
        });
        let mut p = P {
            ev: vec![
                Event::Start(Tag::Strong),
                link,
                Event::Text("see ".into()),
                Event::FootnoteReference("1".into()),
                Event::Text(" more".into()),
                Event::End(TagEnd::Link),
                Event::Text(" after".into()),
                Event::End(TagEnd::Strong),
            ],
            i: 0,
            slugs: HashMap::new(),
        };
        let (rt, _) = p.inlines(false, &mut Open::default());
        assert_eq!(
            linked_spans(&rt),
            [
                ("see ", Some("https://example.com".into())),
                ("[1]", Some("[^1]".into())),
                (" more", Some("https://example.com".into())),
                (" after", None),
            ]
        );
        let styles: Vec<u8> = rt.spans.iter().map(|s| s.style).collect();
        assert_eq!(styles, [BOLD | LINK, BOLD | LINK, BOLD | LINK, BOLD]);
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
