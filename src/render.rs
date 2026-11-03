//! Block model -> GPUI elements.

use std::{path::Path, sync::Arc};

use gpui::{
    AnyElement, App, ElementId, Font, FontFeatures, FontStyle, FontWeight, Hsla,
    InteractiveElement, InteractiveText, IntoElement, ObjectFit, ParentElement, SharedString,
    StatefulInteractiveElement, StrikethroughStyle, Styled, StyledImage, StyledText, TextRun,
    UnderlineStyle, WeakEntity, Window, div, img, prelude::FluentBuilder, px, relative,
};

use crate::{
    markdown::{Align, BOLD, Block, CODE, ITALIC, Image, LINK, RichText, STRIKE},
    theme::Theme,
    viewer::Viewer,
};

pub const CONTENT_MAX_WIDTH: f32 = 880.;
pub const BASE_SIZE: f32 = 16.;

/// Everything a block needs to render itself. Cheap to clone.
#[derive(Clone)]
pub struct RenderCx {
    pub theme: Theme,
    pub ui_font: SharedString,
    pub mono_font: SharedString,
    pub base_dir: Arc<Path>,
    pub viewer: WeakEntity<Viewer>,
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

/// Renders top-level block `ix` (one list item).
pub fn top_block(block: &Block, ix: usize, rcx: &RenderCx) -> AnyElement {
    let mut ids = IdGen(ix * 100_000);
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
                .pb_4()
                .child(render_block(block, rcx, &mut ids, 0)),
        )
        .into_any_element()
}

struct IdGen(usize);
impl IdGen {
    fn next(&mut self) -> ElementId {
        self.0 += 1;
        ElementId::from(self.0)
    }
}

fn render_block(block: &Block, rcx: &RenderCx, ids: &mut IdGen, depth: usize) -> AnyElement {
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
                .child(rich_text(text, weight, color, rcx, ids))
                .into_any_element()
        }
        Block::Paragraph(text) => div()
            .w_full()
            .child(rich_text(text, FontWeight::NORMAL, t.fg, rcx, ids))
            .into_any_element(),
        Block::Images(images) => div()
            .w_full()
            .flex()
            .flex_wrap()
            .gap_2()
            .children(images.iter().map(|i| image(i, rcx)))
            .into_any_element(),
        Block::Code { lang, code } => div()
            .w_full()
            .rounded_md()
            .bg(t.code_bg)
            .relative()
            .child(
                div()
                    .id(ids.next())
                    .w_full()
                    .overflow_x_scroll()
                    .p_4()
                    .font_family(rcx.mono_font.clone())
                    .text_size(px(13.5))
                    .line_height(relative(1.5))
                    .child(
                        div()
                            .whitespace_nowrap()
                            .child(SharedString::from(code.clone())),
                    ),
            )
            .when(!lang.is_empty(), |d| {
                d.child(
                    div()
                        .absolute()
                        .top_1()
                        .right_2()
                        .text_xs()
                        .text_color(t.muted)
                        .child(SharedString::from(lang.clone())),
                )
            })
            .into_any_element(),
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
                .when_some(*callout, |d, c| {
                    d.child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(format!("{c:?}")),
                    )
                })
                .children(blocks.iter().map(|b| render_block(b, rcx, ids, depth + 1)))
                .into_any_element()
        }
        Block::List { start, items } => div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .children(items.iter().enumerate().map(|(i, item)| {
                let marker = match (item.checked, start) {
                    (Some(done), _) => checkbox(done, t),
                    (None, Some(n)) => div()
                        .text_color(t.muted)
                        .child(format!("{}.", n + i as u64)),
                    (None, None) => div().text_color(t.muted).child(["•", "◦", "▪"][depth % 3]),
                };
                div()
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
                                .map(|b| render_block(b, rcx, ids, depth + 1)),
                        ),
                    )
            }))
            .into_any_element(),
        Block::Table {
            aligns,
            header,
            rows,
        } => {
            let row = |cells: &[RichText], head: bool, ids: &mut IdGen| {
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
            };
            let mut el = div()
                .w_full()
                .flex()
                .flex_col()
                .border_1()
                .border_color(t.border)
                .rounded_md()
                .child(row(header, true, ids));
            for r in rows {
                el = el.child(row(r, false, ids).border_t_1().border_color(t.border));
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
                    .children(blocks.iter().map(|b| render_block(b, rcx, ids, depth + 1))),
            )
            .into_any_element(),
        Block::Rule => div()
            .w_full()
            .my_2()
            .h(px(2.))
            .bg(t.border)
            .into_any_element(),
    }
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
    if i.url.contains("://") || i.url.starts_with("data:") {
        // Remote images would need an HTTP stack; keep the viewer lean and
        // show them as links instead.
        let url = i.url.clone();
        return div()
            .id(SharedString::from(i.url.clone()))
            .text_color(t.link)
            .cursor_pointer()
            .child(format!("[image: {alt}]"))
            .on_click(move |_, _, cx| cx.open_url(&url))
            .into_any_element();
    }
    let path = rcx.base_dir.join(percent_decode(&i.url));
    let muted = t.muted;
    img(path)
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

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn open_link(viewer: &WeakEntity<Viewer>, base: &Path, url: &str, _: &mut Window, cx: &mut App) {
    if let Some(anchor) = url.strip_prefix('#') {
        let anchor = percent_decode(anchor);
        viewer
            .update(cx, |v, cx| v.scroll_to_anchor(None, &anchor, cx))
            .ok();
        return;
    }
    if url.contains("://") || url.starts_with("mailto:") {
        cx.open_url(url);
        return;
    }
    let (path, anchor) = match url.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (url, None),
    };
    let path = base.join(percent_decode(path));
    let is_md = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mdx" | "mdown" | "mkd"
        )
    });
    if is_md {
        let anchor = anchor.map(percent_decode);
        viewer
            .update(cx, |v, cx| {
                v.open_paths(vec![path.clone()], cx);
                if let Some(a) = anchor {
                    v.scroll_to_anchor(Some(&path), &a, cx);
                }
            })
            .ok();
    } else {
        cx.open_url(&format!("file://{}", path.display()));
    }
}
