//! Display math and Mermaid diagrams, rendered to images in the background.
//!
//! Display math is typeset with RaTeX (a Rust port of KaTeX) into an SVG made
//! only of paths; Mermaid diagrams are laid out by merman (the Rust Mermaid
//! implementation Zed uses) into an SVG. Either SVG is rasterized with resvg
//! at the window's scale factor and shown with `img()`. All of that runs on
//! GPUI's background executor the first time the block becomes visible, so it
//! never delays startup or a frame: the block shows a placeholder meanwhile.
//!
//! Results live in a global LRU cache keyed by source, theme and scale, so
//! scrolling back, switching tabs or reloading an unchanged file is free.
//! When an edited block's new image is still rendering, the block keeps
//! showing its previous image (see [`Shown`]), so a live reload doesn't
//! flash a placeholder or move the page.

use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
    sync::{Arc, LazyLock, OnceLock},
};

use gpui::{
    AnyElement, App, Global, Hsla, ImageSource, IntoElement, ParentElement, Pixels, RenderImage,
    Rgba, SharedString, Size, Styled, Window, div, img, px, size,
};
use resvg::{tiny_skia, usvg};

use crate::{
    markdown::{Block, Document},
    render::{BASE_SIZE, RenderCx},
    theme::Theme,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Math,
    Mermaid,
}

/// What to render. Everything that changes the pixels is part of the key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

struct Cache {
    slots: HashMap<Key, Slot>,
    bytes: usize,
    clock: u64,
    /// [`CACHE_BYTES`], or less in tests.
    budget: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            slots: HashMap::default(),
            bytes: 0,
            clock: 0,
            budget: CACHE_BYTES,
        }
    }
}

impl Global for Cache {}

/// What [`Cache::lookup`] found.
enum Lookup {
    Ready(Arc<Graphic>),
    Failed(SharedString),
    /// Being rendered.
    Pending,
    /// Wasn't cached: it's now marked pending, and the caller must render it.
    Missing,
}

impl Cache {
    /// Looks `key` up and marks it as just used.
    fn lookup(&mut self, key: &Key) -> Lookup {
        self.clock += 1;
        let clock = self.clock;
        match self.slots.get_mut(key) {
            Some(Slot::Ready { graphic, used }) => {
                *used = clock;
                Lookup::Ready(graphic.clone())
            }
            Some(Slot::Failed(e)) => Lookup::Failed(e.clone()),
            Some(Slot::Pending) => Lookup::Pending,
            None => {
                self.slots.insert(key.clone(), Slot::Pending);
                Lookup::Missing
            }
        }
    }

    /// The image for `key` if it's rendered and still cached, marked as just
    /// used. Unlike [`Self::lookup`], it never starts a render.
    fn ready(&mut self, key: &Key) -> Option<Arc<Graphic>> {
        self.clock += 1;
        match self.slots.get_mut(key) {
            Some(Slot::Ready { graphic, used }) => {
                *used = self.clock;
                Some(graphic.clone())
            }
            _ => None,
        }
    }

    /// Stores a finished render and returns the images evicted to stay under
    /// the memory budget (their GPU textures must be dropped too).
    fn finish(&mut self, key: Key, result: Result<Graphic, String>) -> Vec<Arc<RenderImage>> {
        self.finish_within(key, result, self.budget)
    }

    /// [`Self::finish`] with an explicit budget, in bytes. The image just
    /// stored is never evicted, even if it alone exceeds the budget.
    fn finish_within(
        &mut self,
        key: Key,
        result: Result<Graphic, String>,
        budget: usize,
    ) -> Vec<Arc<RenderImage>> {
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
        while self.bytes > budget {
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

/// A math or diagram block of a tab's document, by identity: its source
/// `Arc`, compared by address. Holding the `Arc` keeps that address from
/// being reused by another block while the entry exists.
#[derive(Clone)]
struct BlockId(Arc<str>);

impl PartialEq for BlockId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for BlockId {}

impl Hash for BlockId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).cast::<u8>().hash(state);
    }
}

/// The image each math or diagram block of a tab showed last, as its cache
/// key. While an edited block's new image renders, the block shows that one
/// instead of a placeholder, at its size, so the page doesn't jump.
///
/// It holds keys, not images: the [`Cache`] stays the only owner of the
/// pixels and GPU textures. A previous image is shown only while it's still
/// cached (it's then marked as used, so it's the last to go); once evicted,
/// its texture has been dropped and the block shows the placeholder, so an
/// evicted image is never drawn (which would upload its texture again,
/// with nothing left to drop it). Memory stays within the cache budget
/// however often a file is reloaded, and closing a tab just drops this.
#[derive(Default)]
pub struct Shown(HashMap<BlockId, Key>);

/// A tab's [`Shown`], shared with its [`RenderCx`].
pub type ShownMap = Rc<RefCell<Shown>>;

impl Shown {
    /// After a reload: what each block of `old` showed now belongs to the
    /// block of `new` that replaces it. Blocks are matched in document
    /// order (nested ones included): the unchanged ones at the start and
    /// at the end, then the ones in between one to one, if there are as
    /// many on both sides. Otherwise it's unclear which edited block is
    /// which, and they start over with a placeholder.
    pub fn carry_over(&mut self, old: &Document, new: &Document) {
        let (old, new) = (graphics_in(&old.blocks), graphics_in(&new.blocks));
        let mut next = HashMap::new();
        for (o, n) in pair_up(&old, &new) {
            if let Some(key) = self.0.get(&BlockId(old[o].1.clone())) {
                next.insert(BlockId(new[n].1.clone()), key.clone());
            }
        }
        self.0 = next;
    }
}

/// The math and diagram blocks of `blocks`, in document order, nested ones
/// included.
fn graphics_in(blocks: &[Block]) -> Vec<(Kind, &Arc<str>)> {
    fn walk<'a>(blocks: &'a [Block], out: &mut Vec<(Kind, &'a Arc<str>)>) {
        for b in blocks {
            match b {
                Block::Math(s) => out.push((Kind::Math, s)),
                Block::Mermaid(s) => out.push((Kind::Mermaid, s)),
                Block::Quote { blocks, .. } | Block::Footnote { blocks, .. } => walk(blocks, out),
                Block::List { items, .. } => {
                    for item in items {
                        walk(&item.blocks, out);
                    }
                }
                Block::Heading { .. }
                | Block::Paragraph(_)
                | Block::Images(_)
                | Block::Code { .. }
                | Block::Table { .. }
                | Block::Rule => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

/// Which block of `old` each block of `new` replaces, as index pairs (see
/// [`Shown::carry_over`]). Only blocks of the same kind are paired.
fn pair_up(old: &[(Kind, &Arc<str>)], new: &[(Kind, &Arc<str>)]) -> Vec<(usize, usize)> {
    let same = |o: usize, n: usize| old[o].0 == new[n].0 && old[o].1 == new[n].1;
    let shorter = old.len().min(new.len());
    let start = (0..shorter).take_while(|&i| same(i, i)).count();
    let end = (0..shorter - start)
        .take_while(|&i| same(old.len() - 1 - i, new.len() - 1 - i))
        .count();
    let mut pairs: Vec<(usize, usize)> = (0..start).map(|i| (i, i)).collect();
    if old.len() == new.len() {
        pairs.extend(
            (start..old.len() - end)
                .filter(|&i| old[i].0 == new[i].0)
                .map(|i| (i, i)),
        );
    }
    pairs.extend((0..end).map(|i| (old.len() - 1 - i, new.len() - 1 - i)));
    pairs
}

/// What a block shows.
enum Show {
    Image(Arc<Graphic>),
    Placeholder,
    Error(SharedString),
}

/// What `block` shows for `key`, and whether `key` must be rendered now.
/// While it renders, the block keeps its previous image if that's still
/// cached and was drawn for the same theme: an image in the other theme's
/// colors could be unreadable (dark text on the dark background), whereas
/// one at another scale factor only looks a little blurry for a moment.
fn show(cache: &mut Cache, shown: &mut Shown, block: &Arc<str>, key: &Key) -> (Show, bool) {
    let id = BlockId(block.clone());
    let render = match cache.lookup(key) {
        Lookup::Ready(graphic) => {
            shown.0.insert(id, key.clone());
            return (Show::Image(graphic), false);
        }
        Lookup::Failed(e) => {
            // The edited source is broken: say so rather than keep showing
            // what it used to be.
            shown.0.remove(&id);
            return (Show::Error(e), false);
        }
        Lookup::Pending => false,
        Lookup::Missing => true,
    };
    let previous = shown
        .0
        .get(&id)
        .filter(|k| k.dark == key.dark)
        .and_then(|k| cache.ready(k));
    (previous.map_or(Show::Placeholder, Show::Image), render)
}

/// The element for a math or diagram block: the rendered image when it's
/// ready, the block's previous image or else a placeholder while it renders,
/// or the source with an error message if it can't be rendered.
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
    let (state, render) = show(cache, &mut rcx.shown.borrow_mut(), source, &key);
    if render {
        spawn(key, rcx, cx);
    }
    #[cfg(test)]
    DRAWN.with_borrow_mut(|d| {
        d.push(match &state {
            Show::Image(g) => Drawn::Image(f32::from(g.size.height)),
            Show::Placeholder => Drawn::Placeholder,
            Show::Error(_) => Drawn::Error,
        })
    });

    let t = &rcx.theme;
    match state {
        Show::Image(g) => {
            let mut image = img(ImageSource::Render(g.image.clone()))
                .w(g.size.width)
                .max_w_full()
                .flex_none();
            // Keep the aspect ratio when a wide image is scaled down.
            image.style().aspect_ratio = aspect_ratio(g.size);
            div()
                .w_full()
                .flex()
                .justify_center()
                .child(image)
                .into_any_element()
        }
        Show::Placeholder => match kind {
            Kind::Math => div().w_full().h(px(BASE_SIZE * 2.5)).into_any_element(),
            Kind::Mermaid => div()
                .w_full()
                .h(px(160.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .border_1()
                .border_color(t.border)
                .text_sm()
                .text_color(t.muted)
                .child("Rendering diagram…")
                .into_any_element(),
        },
        Show::Error(e) => div()
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
    let theme = rcx.theme;
    let font = rcx.ui_font.clone();
    let job = key.clone();
    let task = cx
        .background_executor()
        .spawn(async move { guarded(|| render(&job, &theme, &font)) });
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

/// Runs a renderer. The renderers are third-party code fed with arbitrary
/// input; a bug in one must cost a block, not the whole app (hence
/// `panic = "unwind"` in the release profile), so a panic becomes an error.
fn guarded(render: impl FnOnce() -> Result<Graphic, String>) -> Result<Graphic, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(render))
        .unwrap_or_else(|_| Err("Could not render this block (renderer crashed)".into()))
}

fn rgb(c: Hsla) -> [f32; 3] {
    let c = Rgba::from(c);
    [c.r, c.g, c.b]
}

fn hex(c: Hsla) -> String {
    let c = Rgba::from(c);
    let b = |v: f32| (v * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

fn render(key: &Key, theme: &Theme, ui_font: &str) -> Result<Graphic, String> {
    let scale = key.scale as f32 / 100.;
    match key.kind {
        Kind::Math => {
            let svg = math_svg(&key.source, rgb(theme.fg))?;
            rasterize(&svg, scale, &EMPTY_FONTS)
        }
        Kind::Mermaid => {
            let fonts = mermaid_fonts(ui_font);
            let svg = mermaid_svg(&key.source, theme, ui_font, fonts)?;
            rasterize(&svg, scale, &fonts.db)
        }
    }
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

/// Fonts for diagrams, loaded once, on first use, on a background thread.
struct MermaidFonts {
    /// System fonts for resvg, with `sans-serif` mapped to the UI font.
    db: Arc<usvg::fontdb::Database>,
    /// The UI font's regular and bold faces, so merman measures labels with
    /// the font they're drawn in. Without it, merman's built-in metrics
    /// (for "trebuchet ms") make wider fonts overflow their boxes.
    measure: Option<merman::svg::export::NativeFontContext>,
}

fn mermaid_fonts(ui_font: &str) -> &'static MermaidFonts {
    static FONTS: OnceLock<MermaidFonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        use usvg::fontdb::{Database, Family, Query, Weight};
        let mut db = Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family(ui_font);
        let mut faces = Vec::new();
        for weight in [Weight::NORMAL, Weight::BOLD] {
            let query = Query {
                families: &[Family::Name(ui_font)],
                weight,
                ..Default::default()
            };
            if let Some(id) = db.query(&query) {
                db.with_face_data(id, |data, _| faces.push(data.to_vec()));
            }
        }
        let measure = (!faces.is_empty())
            .then(|| {
                merman::svg::export::NativeFontContext::from_font_bytes(
                    faces,
                    Default::default(),
                    &merman::OperationControl::new(),
                )
                .ok()
            })
            .flatten();
        MermaidFonts {
            db: Arc::new(db),
            measure,
        }
    })
}

/// Mermaid's "base" theme, with colors from our palette.
fn mermaid_config(t: &Theme, font: &str) -> serde_json::Value {
    let (border, note_bg, slice_text) = if t.dark {
        ("#5a6270", "#3d3a2e", t.bg)
    } else {
        ("#b4b6bc", "#fbf3d9", Hsla::white())
    };
    let font = format!("{font}, sans-serif");
    serde_json::json!({
        "theme": "base",
        "darkMode": t.dark,
        "fontFamily": font,
        "themeVariables": {
            "fontFamily": font,
            "background": hex(t.bg),
            "primaryColor": hex(t.code_bg),
            "primaryTextColor": hex(t.fg),
            "primaryBorderColor": border,
            "lineColor": hex(t.muted),
            "secondaryColor": hex(t.hover),
            "tertiaryColor": hex(t.bar_bg),
            "noteBkgColor": note_bg,
            "noteTextColor": hex(t.fg),
            "noteBorderColor": hex(t.warning),
            "pie1": hex(t.accent),
            "pie2": hex(t.tip),
            "pie3": hex(t.important),
            "pie4": hex(t.warning),
            "pie5": hex(t.caution),
            "pie6": hex(t.syntax.ty),
            "pieStrokeColor": hex(t.bg),
            "pieOuterStrokeColor": hex(t.border),
            "pieSectionTextColor": hex(slice_text),
            "pieTitleTextColor": hex(t.fg),
            "pieLegendTextColor": hex(t.fg),
        }
    })
}

fn mermaid_svg(
    source: &str,
    theme: &Theme,
    ui_font: &str,
    fonts: &MermaidFonts,
) -> Result<String, String> {
    use merman::{
        Engine, MermaidConfig, OperationControl, RenderOutput, RenderRequest, Renderer, SvgRequest,
        svg::SvgPipeline,
    };

    let control = OperationControl::new();
    let mut request = SvgRequest {
        pipeline: Some(SvgPipeline::resvg_safe()),
        ..Default::default()
    };
    if let Some(measure) = &fonts.measure {
        request.environment = request
            .environment
            .with_native_font_context(measure.for_operation(&control))
            .map_err(|e| e.to_string())?;
    }
    let config = MermaidConfig::from_value(mermaid_config(theme, ui_font));
    let renderer = Renderer::new().with_engine(Engine::new().with_site_config(config));
    match renderer.render(RenderRequest::svg(source, control, request)) {
        Ok(RenderOutput::Svg(Some(svg))) => Ok(svg.svg().to_string()),
        Ok(_) => Err("Not a Mermaid diagram".into()),
        Err(e) => Err(format!("Mermaid error: {e}")),
    }
}

/// Math SVGs contain only paths, so they need no fonts.
static EMPTY_FONTS: LazyLock<Arc<usvg::fontdb::Database>> =
    LazyLock::new(|| Arc::new(usvg::fontdb::Database::new()));

/// Picks a font for a character the label's font lacks (e.g. `→` in Noto
/// Sans). usvg's default takes the first face that has it, often a monospace
/// one, and when that face covers the whole label usvg draws the whole label
/// with it. So prefer, in order: a non-monospace face, the label's style
/// (see [`style_distance`]), one from the same family group ("Noto Sans
/// Symbols" for "Noto Sans"), a symbols face (it only fills the gaps), and
/// the closest weight.
///
/// The style is a preference, not a filter: symbol, emoji and CJK fonts
/// usually have upright faces only, and an upright `→` in an italic label
/// beats a missing glyph box (issue #55). It ranks below monospace, the
/// worst choice (it can take over the whole label), and above the family
/// group, so an italic label keeps an italic fallback when there is one,
/// as it did when only faces of the same style were considered.
fn fallback_font(
    c: char,
    exclude: &[usvg::fontdb::ID],
    db: &mut Arc<usvg::fontdb::Database>,
) -> Option<usvg::fontdb::ID> {
    let db = &**db;
    let base = db.face(*exclude.first()?)?;
    let base_group = base.families.first()?.0.split(' ').next()?.to_string();
    let has_char = |id| {
        db.with_face_data(id, |data, index| {
            ttf_parser::Face::parse(data, index)
                .ok()
                .and_then(|f| f.glyph_index(c))
                .is_some()
        })
        .unwrap_or(false)
    };
    db.faces()
        .filter(|f| !exclude.contains(&f.id))
        .filter(|f| has_char(f.id))
        .min_by_key(|f| {
            let family = f.families.first().map_or("", |f| f.0.as_str());
            (
                f.monospaced || family.contains("Mono"),
                style_distance(f.style, base.style),
                !family.starts_with(&base_group),
                !family.contains("Symbols"),
                f.weight.0.abs_diff(base.weight.0),
            )
        })
        .map(|f| f.id)
}

/// How far a fallback face's style is from the label's: 0 for the same
/// style, 1 for italic and oblique (both slanted), 2 for upright and slanted.
fn style_distance(face: usvg::fontdb::Style, label: usvg::fontdb::Style) -> u8 {
    use usvg::fontdb::Style;
    match (face, label) {
        _ if face == label => 0,
        (Style::Normal, _) | (_, Style::Normal) => 2,
        _ => 1,
    }
}

fn rasterize(
    svg: &str,
    scale: f32,
    fonts: &Arc<usvg::fontdb::Database>,
) -> Result<Graphic, String> {
    let opts = usvg::Options {
        fontdb: fonts.clone(),
        font_resolver: usvg::FontResolver {
            select_font: usvg::FontResolver::default_font_selector(),
            select_fallback: Box::new(fallback_font),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &opts).map_err(|e| e.to_string())?;
    let logical = tree.size();
    let (w, h) = (logical.width(), logical.height());
    // Large diagrams are rasterized at a lower scale instead of failing.
    let scale = scale.max(1.).min(MAX_SIDE / w.max(h).max(1.));
    // At least one pixel per side, also for the logical size below: a side
    // that rounds to 0 (an extreme aspect ratio) would give a 0px size and
    // an infinite aspect ratio (issue #54).
    let (pw, ph) = (
        ((w * scale).ceil() as u32).max(1),
        ((h * scale).ceil() as u32).max(1),
    );
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("empty image")?;
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
    let buffer = image::RgbaImage::from_raw(pw, ph, data).ok_or("bad image")?;
    Ok(Graphic {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
        size: size(px(pw as f32 / scale), px(ph as f32 / scale)),
    })
}

/// Width over height, or `None` (no fixed ratio) for a size that has no
/// finite, positive ratio, which taffy can't lay out.
fn aspect_ratio(size: Size<Pixels>) -> Option<f32> {
    let ratio = f32::from(size.width) / f32::from(size.height);
    (ratio.is_finite() && ratio > 0.).then_some(ratio)
}

/// Number of (ready, failed, pending) cache entries for each kind, for tests.
#[cfg(test)]
pub(crate) fn cache_stats(cx: &App, kind: Kind) -> (usize, usize, usize) {
    let mut stats = (0, 0, 0);
    if let Some(cache) = cx.try_global::<Cache>() {
        for (k, slot) in &cache.slots {
            if k.kind == kind {
                match slot {
                    Slot::Ready { .. } => stats.0 += 1,
                    Slot::Failed(_) => stats.1 += 1,
                    Slot::Pending => stats.2 += 1,
                }
            }
        }
    }
    stats
}

/// What [`element`] showed for a block, for tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Drawn {
    /// An image, with its logical height.
    Image(f32),
    Placeholder,
    Error,
}

#[cfg(test)]
thread_local! {
    static DRAWN: RefCell<Vec<Drawn>> = const { RefCell::new(Vec::new()) };
}

/// What [`element`] showed since the last call, oldest first, for tests
/// (GPUI's `debug_bounds` keeps elements of earlier frames).
#[cfg(test)]
pub(crate) fn take_drawn() -> Vec<Drawn> {
    DRAWN.take()
}

/// Lowers the cache budget, so tests can fill it with small images.
#[cfg(test)]
pub(crate) fn set_cache_budget(cx: &mut App, bytes: usize) {
    cx.default_global::<Cache>().budget = bytes;
}

/// The pixel data in the cache, in bytes, for tests.
#[cfg(test)]
pub(crate) fn cache_bytes(cx: &App) -> usize {
    cx.try_global::<Cache>().map_or(0, |c| c.bytes)
}

/// Number of blocks with a previous image, for tests.
#[cfg(test)]
pub(crate) fn shown_len(shown: &Shown) -> usize {
    shown.0.len()
}

#[cfg(test)]
mod tests;
