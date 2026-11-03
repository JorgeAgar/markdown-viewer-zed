//! Display math and diagrams, rendered to images in the background.
//!
//! A block of display math is typeset with RaTeX (a Rust port of KaTeX) into
//! an SVG made only of paths, rasterized with resvg at the window's scale
//! factor, and shown with `img()`. All of that runs on GPUI's background
//! executor the first time the block becomes visible, so it never delays
//! startup or a frame: the block shows a placeholder for a frame or two.
//!
//! Results live in a global LRU cache keyed by source, theme and scale, so
//! scrolling back, switching tabs or reloading an unchanged file is free.

use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, LazyLock},
};

use gpui::{
    AnyElement, App, Global, Hsla, ImageSource, IntoElement, ParentElement, Pixels, RenderImage,
    Rgba, SharedString, Size, Styled, Window, div, img, px, size,
};
use resvg::{tiny_skia, usvg};

use crate::render::{BASE_SIZE, RenderCx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Math,
}

/// What to render. Everything that changes the pixels is part of the key.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    kind: Kind,
    source: Arc<str>,
    dark: bool,
    /// Window scale factor × 100.
    scale: u32,
}

struct Graphic {
    image: Arc<RenderImage>,
    /// Size in logical pixels.
    size: Size<Pixels>,
}

enum Slot {
    Pending,
    Ready { graphic: Arc<Graphic>, used: u64 },
    Failed(SharedString),
}

/// Upper bound for the pixel data kept in the cache. Live-reloading a file
/// while editing a formula creates a new image per save, so old ones must go.
const CACHE_BYTES: usize = 128 << 20;
/// Largest raster side, in device pixels.
const MAX_SIDE: f32 = 4096.;

#[derive(Default)]
struct Cache {
    slots: HashMap<Key, Slot>,
    bytes: usize,
    clock: u64,
}

impl Global for Cache {}

impl Cache {
    /// Stores a finished render and returns the images evicted to stay under
    /// the memory budget (their GPU textures must be dropped too).
    fn finish(&mut self, key: Key, result: Result<Graphic, String>) -> Vec<Arc<RenderImage>> {
        self.clock += 1;
        let slot = match result {
            Ok(g) => {
                self.bytes += bytes(&g.image);
                Slot::Ready {
                    graphic: Arc::new(g),
                    used: self.clock,
                }
            }
            Err(e) => Slot::Failed(e.into()),
        };
        self.slots.insert(key.clone(), slot);

        let mut evicted = Vec::new();
        while self.bytes > CACHE_BYTES {
            let oldest = self
                .slots
                .iter()
                .filter(|(k, _)| **k != key)
                .filter_map(|(k, s)| match s {
                    Slot::Ready { used, .. } => Some((*used, k.clone())),
                    _ => None,
                })
                .min_by_key(|(used, _)| *used);
            let Some((_, k)) = oldest else { break };
            if let Some(Slot::Ready { graphic, .. }) = self.slots.remove(&k) {
                self.bytes -= bytes(&graphic.image);
                evicted.push(graphic.image.clone());
            }
        }
        evicted
    }
}

fn bytes(image: &RenderImage) -> usize {
    image.as_bytes(0).map_or(0, |b| b.len())
}

/// The element for a math block (or, later, a diagram): the rendered image
/// when it's ready, a placeholder while it renders, or the source with an
/// error message if it can't be rendered.
pub fn element(
    kind: Kind,
    source: &Arc<str>,
    rcx: &RenderCx,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let key = Key {
        kind,
        source: source.clone(),
        dark: rcx.theme.dark,
        scale: (window.scale_factor() * 100.).round() as u32,
    };
    let cache = cx.default_global::<Cache>();
    cache.clock += 1;
    let clock = cache.clock;
    let state = match cache.slots.get_mut(&key) {
        Some(Slot::Ready { graphic, used }) => {
            *used = clock;
            Ok(Some(graphic.clone()))
        }
        Some(Slot::Failed(e)) => Err(e.clone()),
        Some(Slot::Pending) => Ok(None),
        None => {
            cache.slots.insert(key.clone(), Slot::Pending);
            spawn(key, rcx, cx);
            Ok(None)
        }
    };

    let t = &rcx.theme;
    match state {
        Ok(Some(g)) => {
            let mut image = img(ImageSource::Render(g.image.clone()))
                .w(g.size.width)
                .max_w_full()
                .flex_none();
            // Keep the aspect ratio when a wide image is scaled down.
            image.style().aspect_ratio = Some(f32::from(g.size.width) / f32::from(g.size.height));
            div()
                .w_full()
                .flex()
                .justify_center()
                .child(image)
                .into_any_element()
        }
        Ok(None) => div().w_full().h(px(BASE_SIZE * 2.5)).into_any_element(),
        Err(e) => div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .rounded_md()
            .bg(t.code_bg)
            .text_size(px(13.5))
            .child(div().text_color(t.caution).child(e))
            .child(
                div()
                    .font_family(rcx.mono_font.clone())
                    .text_color(t.muted)
                    .child(SharedString::from(source.to_string())),
            )
            .into_any_element(),
    }
}

fn spawn(key: Key, rcx: &RenderCx, cx: &mut App) {
    let fg = rgb(rcx.theme.fg);
    let job = key.clone();
    let task = cx.background_executor().spawn(async move {
        // The renderers are third-party code fed with arbitrary input; a bug
        // in one must cost a block, not the whole app (hence panic=unwind).
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&job, fg)))
            .unwrap_or_else(|_| Err("Could not render this block (renderer crashed)".into()))
    });
    let viewer = rcx.viewer.clone();
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let evicted = cx.global_mut::<Cache>().finish(key, result);
            for image in evicted {
                cx.drop_image(image, None);
            }
            viewer.update(cx, |_, cx| cx.notify()).ok();
        })
        .ok();
    })
    .detach();
}

fn rgb(c: Hsla) -> [f32; 3] {
    let c = Rgba::from(c);
    [c.r, c.g, c.b]
}

fn render(key: &Key, fg: [f32; 3]) -> Result<Graphic, String> {
    let svg = match key.kind {
        Kind::Math => math_svg(&key.source, fg)?,
    };
    rasterize(&svg, key.scale as f32 / 100., &EMPTY_FONTS)
}

/// Math is drawn at KaTeX's default size: 1.21 × the body text.
const MATH_EM: f64 = BASE_SIZE as f64 * 1.21;

fn math_svg(tex: &str, [r, g, b]: [f32; 3]) -> Result<String, String> {
    use ratex_layout::{LayoutOptions, layout, to_display_list, validate_layout};
    use ratex_types::{Color, MathStyle};

    let ast = ratex_parser::parser::parse(tex).map_err(|e| format!("Math error: {e}"))?;
    let opts = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(Color { r, g, b, a: 1.0 });
    let lbox = layout(&ast, &opts);
    validate_layout(&lbox).map_err(|e| format!("Math error: {e}"))?;
    let list = to_display_list(&lbox);
    Ok(ratex_svg::render_to_svg(
        &list,
        &ratex_svg::SvgOptions {
            font_size: MATH_EM,
            padding: 2.0,
            stroke_width: 1.0,
            embed_glyphs: true,
            ..Default::default()
        },
    ))
}

/// Math SVGs contain only paths, so they need no fonts.
static EMPTY_FONTS: LazyLock<Arc<usvg::fontdb::Database>> =
    LazyLock::new(|| Arc::new(usvg::fontdb::Database::new()));

fn rasterize(
    svg: &str,
    scale: f32,
    fonts: &Arc<usvg::fontdb::Database>,
) -> Result<Graphic, String> {
    let opts = usvg::Options {
        fontdb: fonts.clone(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &opts).map_err(|e| e.to_string())?;
    let logical = tree.size();
    let (w, h) = (logical.width(), logical.height());
    // Large diagrams are rasterized at a lower scale instead of failing.
    let scale = scale.max(1.).min(MAX_SIDE / w.max(h).max(1.));
    let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(pw.max(1), ph.max(1)).ok_or("empty image")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia gives premultiplied RGBA; GPUI wants straight BGRA.
    let mut data = pixmap.take();
    for px in data.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let buffer = image::RgbaImage::from_raw(pw.max(1), ph.max(1), data).ok_or("bad image")?;
    Ok(Graphic {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
        size: size(px(pw as f32 / scale), px(ph as f32 / scale)),
    })
}
