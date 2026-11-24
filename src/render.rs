//! Block model -> GPUI elements.

use std::{cell::RefCell, collections::HashMap, path::Path, rc::Rc, sync::Arc};

use gpui::{
    AnyElement, App, ElementId, Font, FontFeatures, FontStyle, FontWeight, Hsla,
    InteractiveElement, InteractiveText, IntoElement, ObjectFit, ParentElement, Pixels,
    ScrollHandle, SharedString, StatefulInteractiveElement, StrikethroughStyle, Styled,
    StyledImage, StyledText, TextRun, UnderlineStyle, WeakEntity, Window, div, img,
    prelude::FluentBuilder, px, relative,
};

use crate::{
    graphics::{self, Kind},
    highlight::{Highlight, Tok},
    links::{LinkTarget, classify_link, percent_decode},
    markdown::{
        Align, BOLD, Block, CODE, Callout, Document, ITALIC, Image, LINK, ListItem, RichText,
        STRIKE,
    },
    rows::{self, Row},
    theme::Theme,
    viewer::Viewer,
};

#[cfg(unix)]
use crate::links::file_url;

pub const CONTENT_MAX_WIDTH: f32 = 880.;
pub const BASE_SIZE: f32 = 16.;

/// Shared horizontal scroll state for one split `Block::Code`. All chunks of
/// the same block track the same `ScrollHandle` and enforce `min_width` so
/// shorter chunks never clamp away the scroll offset of wider chunks.
#[derive(Clone)]
pub struct CodeScroll {
    pub handle: ScrollHandle,
    pub min_width: Pixels,
}

/// Per-tab cache of shared horizontal scroll handles, keyed by top-level
/// `Block::Code` index.
pub type CodeScrollMap = Rc<RefCell<HashMap<usize, CodeScroll>>>;

/// Everything a block needs to render itself. Cheap to clone.
#[derive(Clone)]
pub struct RenderCx {
    pub theme: Theme,
    pub ui_font: SharedString,
    pub mono_font: SharedString,
    pub base_dir: Arc<Path>,
    pub viewer: WeakEntity<Viewer>,
    pub code_scroll: CodeScrollMap,
}

impl RenderCx {
    fn font(&self, mono: bool, weight: FontWeight, italic: bool) -> Font {
        Font {
            family: if mono {
                self.mono_font.clone()
            } else {
                self.ui_font.clone()
            },
            features: FontFeatures::default(),
            fallbacks: None,
            weight,
            style: if italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
        }
    }
}

/// Per-row rendering state: element ids, plus the window and app, which
/// math blocks need to fetch their images.
struct Cx<'a> {
    ids: IdGen,
    window: &'a mut Window,
    app: &'a mut App,
}

#[derive(Clone, Copy)]
enum RowPad {
    None,
    Sm,
    Block,
}

fn row_frame(ix: usize, pad: RowPad, child: AnyElement) -> AnyElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .when(ix == 0, |d| d.pt_8())
        .child(
            div()
                .w_full()
                .max_w(px(CONTENT_MAX_WIDTH))
                .px_8()
                .when(matches!(pad, RowPad::Sm), |d| d.pb_1())
                .when(matches!(pad, RowPad::Block), |d| d.pb_4())
                .child(child),
        )
        .into_any_element()
}

/// Renders one virtualized list row (`ix`) of `doc`.
pub fn render_row(
    doc: &Document,
    row: &Row,
    ix: usize,
    rcx: &RenderCx,
    window: &mut Window,
    app: &mut App,
) -> AnyElement {
    let mut c = Cx {
        ids: IdGen(ix.wrapping_mul(100_000)),
        window,
        app,
    };
    let t = &rcx.theme;
    match row {
        Row::Block(block_ix) => row_frame(
            ix,
            RowPad::Block,
            render_block(&doc.blocks[*block_ix], rcx, &mut c, 0),
        ),
        Row::TableHeader { block, last } => {
            let Block::Table { aligns, header, .. } = &doc.blocks[*block] else {
                unreachable!("TableHeader row points to a Table block");
            };
            let el = table_row(aligns, header, true, rcx, &mut c.ids)
                .border_t_1()
                .border_l_1()
                .border_r_1()
                .border_color(t.border)
                .rounded_t_md()
                .when(*last, |d| d.border_b_1().rounded_b_md())
                .into_any_element();
            row_frame(ix, if *last { RowPad::Block } else { RowPad::None }, el)
        }
        Row::TableBody { block, row, last } => {
            let Block::Table { aligns, rows, .. } = &doc.blocks[*block] else {
                unreachable!("TableBody row points to a Table block");
            };
            let el = table_row(aligns, &rows[*row], false, rcx, &mut c.ids)
                .border_t_1()
                .border_l_1()
                .border_r_1()
                .border_color(t.border)
                .when(*last, |d| d.border_b_1().rounded_b_md())
                .into_any_element();
            row_frame(ix, if *last { RowPad::Block } else { RowPad::None }, el)
        }
        Row::ListItem { block, item, last } => {
            let Block::List { start, items } = &doc.blocks[*block] else {
                unreachable!("ListItem row points to a List block");
            };
            let el = list_item_row(*start, *item, &items[*item], 0, rcx, &mut c).into_any_element();
            row_frame(ix, if *last { RowPad::Block } else { RowPad::Sm }, el)
        }
        Row::CodeChunk {
            block,
            start,
            end,
            first,
            last,
        } => {
            let Block::Code {
                lang,
                code,
                highlights,
            } = &doc.blocks[*block]
            else {
                unreachable!("CodeChunk row points to a Code block");
            };
            let scroll = code_scroll_for(*block, code, rcx, c.window);
            let sliced = rows::slice_highlights(highlights, *start..*end);
            let el = code_box(
                lang,
                &code[*start..*end],
                &sliced,
                *first,
                *last,
                Some(&scroll),
                rcx,
                &mut c.ids,
            )
            .into_any_element();
            row_frame(ix, if *last { RowPad::Block } else { RowPad::None }, el)
        }
        Row::QuoteHeader { block } => {
            let Block::Quote { callout, .. } = &doc.blocks[*block] else {
                unreachable!("QuoteHeader row points to a Quote block");
            };
            let callout = callout.expect("QuoteHeader requires a callout");
            let color = t.callout(callout);
            let el = div()
                .w_full()
                .border_l_4()
                .border_color(color)
                .pl_4()
                .pt_1()
                .pb_3()
                .child(quote_callout(callout, color))
                .into_any_element();
            row_frame(ix, RowPad::None, el)
        }
        Row::QuoteBlock {
            block,
            child,
            first,
            last,
        } => {
            let Block::Quote { callout, blocks } = &doc.blocks[*block] else {
                unreachable!("QuoteBlock row points to a Quote block");
            };
            let color = callout.map(|c| t.callout(c)).unwrap_or(t.border);
            let el = div()
                .w_full()
                .border_l_4()
                .border_color(color)
                .pl_4()
                .when(*first, |d| d.pt_1())
                .when(*last, |d| d.pb_1())
                .when(!*last, |d| d.pb_3())
                .when(callout.is_none(), |d| d.text_color(t.muted))
                .child(render_block(&blocks[*child], rcx, &mut c, 1))
                .into_any_element();
            row_frame(ix, if *last { RowPad::Block } else { RowPad::None }, el)
        }
    }
}

struct IdGen(usize);
impl IdGen {
    fn next(&mut self) -> ElementId {
        self.0 += 1;
        ElementId::from(self.0)
    }
}

fn code_scroll_for(block: usize, code: &str, rcx: &RenderCx, window: &Window) -> CodeScroll {
    if let Some(existing) = rcx.code_scroll.borrow().get(&block) {
        return existing.clone();
    }
    let [a, b] = rows::widest_line_candidates(code);
    let mut min_width = measure_code_line(&code[a.clone()], rcx, window);
    if b != a {
        min_width = min_width.max(measure_code_line(&code[b], rcx, window));
    }
    let entry = CodeScroll {
        handle: ScrollHandle::new(),
        min_width,
    };
    rcx.code_scroll.borrow_mut().insert(block, entry.clone());
    entry
}

fn measure_code_line(line: &str, rcx: &RenderCx, window: &Window) -> Pixels {
    if line.is_empty() {
        return px(0.);
    }
    let run = TextRun {
        len: line.len(),
        font: rcx.font(true, FontWeight::NORMAL, false),
        color: rcx.theme.fg,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(SharedString::from(line.to_owned()), px(13.5), &[run], None)
        .width
        .ceil()
}

#[allow(clippy::too_many_arguments)]
fn code_box(
    lang: &str,
    code: &str,
    highlights: &[Highlight],
    first: bool,
    last: bool,
    scroll: Option<&CodeScroll>,
    rcx: &RenderCx,
    ids: &mut IdGen,
) -> gpui::Div {
    let t = &rcx.theme;
    div()
        .w_full()
        .bg(t.code_bg)
        .relative()
        .when(first, |d| d.rounded_t_md())
        .when(last, |d| d.rounded_b_md())
        .child(
            div()
                .id(ids.next())
                .w_full()
                .overflow_x_scroll()
                .when_some(scroll, |d, s| d.track_scroll(&s.handle))
                .px_4()
                .when(first, |d| d.pt_4())
                .when(last, |d| d.pb_4())
                .font_family(rcx.mono_font.clone())
                .text_size(px(13.5))
                .line_height(relative(1.5))
                .child(
                    div()
                        .whitespace_nowrap()
                        .min_h(px(13.5 * 1.5))
                        .when_some(scroll, |d, s| d.min_w(s.min_width))
                        .child(code_text(code, highlights, rcx)),
                ),
        )
        .when(first && !lang.is_empty(), |d| {
            d.child(
                div()
                    .absolute()
                    .top_1()
                    .right_2()
                    .text_xs()
                    .text_color(t.muted)
                    .child(SharedString::from(lang.to_owned())),
            )
        })
}

fn quote_callout(callout: Callout, color: Hsla) -> gpui::Div {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(format!("{callout:?}"))
}

fn list_item_row(
    start: Option<u64>,
    i: usize,
    item: &ListItem,
    depth: usize,
    rcx: &RenderCx,
    c: &mut Cx,
) -> gpui::Div {
    let t = &rcx.theme;
    let marker = match (item.checked, start) {
        (Some(done), _) => checkbox(done, t),
        (None, Some(n)) => div()
            .text_color(t.muted)
            .child(format!("{}.", n + i as u64)),
        (None, None) => div().text_color(t.muted).child(["•", "◦", "▪"][depth % 3]),
    };
    div()
        .w_full()
        .flex()
        .flex_row()
        .gap_2()
        .child(
            div()
                .flex_none()
                .min_w(px(18.))
                .flex()
                .justify_end()
                .child(marker),
        )
        .child(
            div().flex_1().min_w_0().flex().flex_col().gap_2().children(
                item.blocks
                    .iter()
                    .map(|b| render_block(b, rcx, c, depth + 1)),
            ),
        )
}

fn table_row(
    aligns: &[Align],
    cells: &[RichText],
    head: bool,
    rcx: &RenderCx,
    ids: &mut IdGen,
) -> gpui::Div {
    let t = &rcx.theme;
    div()
        .flex()
        .flex_row()
        .w_full()
        .when(head, |d| d.bg(t.code_bg))
        .children(cells.iter().enumerate().map(|(i, cell)| {
            let align = aligns.get(i).copied().unwrap_or(Align::None);
            div()
                .flex_1()
                .min_w_0()
                .px_3()
                .py_1p5()
                .when(i > 0, |d| d.border_l_1().border_color(t.border))
                .when(align == Align::Center, |d| d.text_center())
                .when(align == Align::Right, |d| d.text_right())
                .child(rich_text(
                    cell,
                    if head {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    },
                    t.fg,
                    rcx,
                    ids,
                ))
        }))
}

fn render_block(block: &Block, rcx: &RenderCx, c: &mut Cx, depth: usize) -> AnyElement {
    let t = &rcx.theme;
    match block {
        Block::Heading { level, text, .. } => {
            let (size, weight) = match level {
                1 => (2.0, FontWeight::BOLD),
                2 => (1.5, FontWeight::BOLD),
                3 => (1.25, FontWeight::SEMIBOLD),
                4 => (1.0, FontWeight::SEMIBOLD),
                5 => (0.875, FontWeight::SEMIBOLD),
                _ => (0.85, FontWeight::SEMIBOLD),
            };
            let color = if *level >= 6 { t.muted } else { t.fg };
            div()
                .w_full()
                .when(depth == 0, |d| d.pt(px(BASE_SIZE * 0.75)))
                .text_size(px(BASE_SIZE * size))
                .line_height(relative(1.25))
                .when(*level <= 2, |d| {
                    d.pb_2().border_b_1().border_color(t.border)
                })
                .child(rich_text(text, weight, color, rcx, &mut c.ids))
                .into_any_element()
        }
        Block::Paragraph(text) => div()
            .w_full()
            .child(rich_text(text, FontWeight::NORMAL, t.fg, rcx, &mut c.ids))
            .into_any_element(),
        Block::Images(images) => div()
            .w_full()
            .flex()
            .flex_wrap()
            .gap_2()
            .children(images.iter().map(|i| image(i, rcx)))
            .into_any_element(),
        Block::Code {
            lang,
            code,
            highlights,
        } => code_box(lang, code, highlights, true, true, None, rcx, &mut c.ids).into_any_element(),
        Block::Quote { callout, blocks } => {
            let color = callout.map(|c| t.callout(c)).unwrap_or(t.border);
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .border_l_4()
                .border_color(color)
                .pl_4()
                .py_1()
                .when(callout.is_none(), |d| d.text_color(t.muted))
                .when_some(*callout, |d, c| d.child(quote_callout(c, color)))
                .children(blocks.iter().map(|b| render_block(b, rcx, c, depth + 1)))
                .into_any_element()
        }
        Block::List { start, items } => div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .children(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| list_item_row(*start, i, item, depth, rcx, c)),
            )
            .into_any_element(),
        Block::Table {
            aligns,
            header,
            rows,
        } => {
            let mut el = div()
                .w_full()
                .flex()
                .flex_col()
                .border_1()
                .border_color(t.border)
                .rounded_md()
                .child(table_row(aligns, header, true, rcx, &mut c.ids));
            for r in rows {
                el = el.child(
                    table_row(aligns, r, false, rcx, &mut c.ids)
                        .border_t_1()
                        .border_color(t.border),
                );
            }
            el.into_any_element()
        }
        Block::Footnote { label, blocks } => div()
            .w_full()
            .flex()
            .gap_2()
            .text_sm()
            .text_color(t.muted)
            .child(div().flex_none().child(format!("[{label}]")))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(blocks.iter().map(|b| render_block(b, rcx, c, depth + 1))),
            )
            .into_any_element(),
        Block::Math(tex) => graphics::element(Kind::Math, tex, rcx, c.window, c.app),
        Block::Mermaid(src) => graphics::element(Kind::Mermaid, src, rcx, c.window, c.app),
        Block::Rule => div()
            .w_full()
            .my_2()
            .h(px(2.))
            .bg(t.border)
            .into_any_element(),
    }
}

/// A code block's text, colored with the highlights computed at parse time.
fn code_text(code: &str, highlights: &[Highlight], rcx: &RenderCx) -> StyledText {
    let t = &rcx.theme;
    let run = |len, color, italic| TextRun {
        len,
        font: rcx.font(true, FontWeight::NORMAL, italic),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut pos = 0;
    for h in highlights {
        if h.range.start > pos {
            runs.push(run(h.range.start - pos, t.fg, false));
        }
        runs.push(run(h.range.len(), t.token(h.tok), h.tok == Tok::Comment));
        pos = h.range.end;
    }
    if pos < code.len() {
        runs.push(run(code.len() - pos, t.fg, false));
    }
    StyledText::new(SharedString::from(code.to_owned())).with_runs(runs)
}

fn checkbox(done: bool, t: &Theme) -> gpui::Div {
    // Line box of the surrounding text so the box sits on the first line.
    div().h(px(BASE_SIZE * 1.6)).flex().items_center().child(
        div()
            .size(px(14.))
            .rounded_sm()
            .border_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .line_height(px(14.))
            .when(done, |d| {
                d.bg(t.accent)
                    .border_color(t.accent)
                    .text_color(t.bg)
                    .child("✓")
            })
            .when(!done, |d| d.border_color(t.muted)),
    )
}

fn image(i: &Image, rcx: &RenderCx) -> AnyElement {
    let t = rcx.theme;
    let alt: SharedString = if i.alt.is_empty() {
        i.url.clone().into()
    } else {
        i.alt.clone().into()
    };
    if image_is_link(&i.url) {
        // Unsupported URI schemes remain clickable.
        let url = i.url.clone();
        return div()
            .id(SharedString::from(i.url.clone()))
            .text_color(t.link)
            .cursor_pointer()
            .child(format!("[image: {alt}]"))
            .on_click(move |_, _, cx| cx.open_url(&url))
            .into_any_element();
    }
    let muted = t.muted;
    img(image_source(&i.url, &rcx.base_dir))
        .max_w_full()
        .object_fit(ObjectFit::ScaleDown)
        .with_fallback(move || {
            div()
                .text_color(muted)
                .child(format!("[image: {alt}]"))
                .into_any_element()
        })
        .into_any_element()
}

fn rich_text(
    rt: &RichText,
    weight: FontWeight,
    color: Hsla,
    rcx: &RenderCx,
    ids: &mut IdGen,
) -> AnyElement {
    let t = &rcx.theme;
    let mut runs = Vec::with_capacity(rt.spans.len() + 1);
    let mut pos = 0;
    let plain = |len| TextRun {
        len,
        font: rcx.font(false, weight, false),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    for s in &rt.spans {
        if s.range.start > pos {
            runs.push(plain(s.range.start - pos));
        }
        let st = s.style;
        let link = st & LINK != 0;
        let fg = if link { t.link } else { color };
        runs.push(TextRun {
            len: s.range.len(),
            font: rcx.font(
                st & CODE != 0,
                if st & BOLD != 0 {
                    FontWeight::BOLD
                } else {
                    weight
                },
                st & ITALIC != 0,
            ),
            color: fg,
            background_color: (st & CODE != 0).then_some(t.code_bg),
            underline: link.then_some(UnderlineStyle {
                thickness: px(1.),
                color: Some(fg.opacity(0.5)),
                wavy: false,
            }),
            strikethrough: (st & STRIKE != 0).then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(fg),
            }),
        });
        pos = s.range.end;
    }
    if pos < rt.text.len() {
        runs.push(plain(rt.text.len() - pos));
    }

    let styled = StyledText::new(SharedString::from(rt.text.clone())).with_runs(runs);
    if rt.links.is_empty() {
        return styled.into_any_element();
    }
    let (ranges, link_ids) = rt.link_ranges();
    let links = rt.links.clone();
    let viewer = rcx.viewer.clone();
    let base = rcx.base_dir.clone();
    InteractiveText::new(ids.next(), styled)
        .on_click(ranges, move |ix, window, cx| {
            if let Some(url) = link_ids.get(ix).and_then(|i| links.get(*i as usize)) {
                open_link(&viewer, &base, url, window, cx);
            }
        })
        .into_any_element()
}

pub(crate) fn open_link(
    viewer: &WeakEntity<Viewer>,
    base: &Path,
    url: &str,
    _: &mut Window,
    cx: &mut App,
) {
    match classify_link(base, url) {
        LinkTarget::Anchor(anchor) => {
            viewer
                .update(cx, |v, cx| v.scroll_to_anchor(None, &anchor, cx))
                .ok();
        }
        LinkTarget::External(url) => cx.open_url(&url),
        LinkTarget::Markdown { path, anchor } => {
            viewer
                .update(cx, |v, cx| {
                    v.open_paths(vec![path.clone()], cx);
                    if let Some(a) = anchor {
                        v.scroll_to_anchor(Some(&path), &a, cx);
                    }
                })
                .ok();
        }
        LinkTarget::File(path) => {
            #[cfg(windows)]
            crate::platform::ipc::open_file(&path);
            #[cfg(unix)]
            cx.open_url(&file_url(&path));
        }
    }
}

fn image_is_link(url: &str) -> bool {
    (url.contains("://") && !url.starts_with("http://") && !url.starts_with("https://"))
        || url.starts_with("data:")
}

fn image_source(url: &str, base_dir: &Path) -> gpui::ImageSource {
    if url.starts_with("http://") || url.starts_with("https://") {
        gpui::SharedUri::from(url.to_owned()).into()
    } else {
        base_dir.join(percent_decode(url)).into()
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn unsupported_image_schemes_stay_links() {
        for url in ["ftp://example.com/a.png", "data:image/png;base64,abc"] {
            assert!(image_is_link(url));
        }
        for url in [
            "http://example.com/a.png",
            "https://example.com/a.png",
            "a.png",
        ] {
            assert!(!image_is_link(url));
        }
    }

    #[test]
    fn remote_images_use_uri_resources_and_preserve_escaping() {
        for url in [
            "http://example.com/a.png",
            "https://example.com/a%20b.svg?q=x%23y",
        ] {
            assert!(matches!(image_source(url, Path::new("/docs")),
                gpui::ImageSource::Resource(gpui::Resource::Uri(uri)) if uri.as_ref() == url));
        }
    }

    #[test]
    fn local_images_resolve_against_document_directory() {
        for (url, expected) in [("a%20b.png", "/docs/a b.png"), ("/tmp/a.png", "/tmp/a.png")] {
            assert!(matches!(image_source(url, Path::new("/docs")),
                gpui::ImageSource::Resource(gpui::Resource::Path(path)) if path.as_ref() == Path::new(expected)));
        }
    }
}
