//! Tests for display math and Mermaid rendering, and the image cache.
//!
//! Display math only uses the KaTeX fonts embedded in RaTeX, so its output is
//! deterministic and checked against golden PNGs in `tests/golden/math/`.
//! Run with `UPDATE_GOLDEN=1` to (re)write them after an intended change. On
//! a mismatch the actual image and a diff are written to
//! `target/golden-failures/math/`.
//!
//! Mermaid uses system fonts, so diagrams are only checked for rendering,
//! being non-blank and respecting `MAX_SIDE`.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use image::RgbaImage;

use super::*;
use crate::markdown::{self, Block};

fn key(kind: Kind, source: &str, dark: bool, scale: u32) -> Key {
    Key {
        kind,
        source: source.into(),
        dark,
        scale,
    }
}

fn theme(dark: bool) -> Theme {
    if dark { Theme::dark() } else { Theme::light() }
}

fn render_math(tex: &str, dark: bool, scale: u32) -> Result<Graphic, String> {
    render(&key(Kind::Math, tex, dark, scale), &theme(dark), "")
}

/// A UI font that's installed, like `pick_fonts` would choose.
fn ui_font() -> &'static str {
    static FONT: OnceLock<String> = OnceLock::new();
    FONT.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        let families: Vec<String> = db
            .faces()
            .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
            .collect();
        [
            "Inter",
            "IBM Plex Sans",
            "Noto Sans",
            "Cantarell",
            "DejaVu Sans",
            "Liberation Sans",
        ]
        .iter()
        .find(|f| families.iter().any(|n| n == *f))
        .map(|f| f.to_string())
        .or_else(|| families.first().cloned())
        .unwrap_or_else(|| "sans-serif".into())
    })
}

fn render_mermaid(src: &str, dark: bool, scale: u32) -> Result<Graphic, String> {
    render(
        &key(Kind::Mermaid, src, dark, scale),
        &theme(dark),
        ui_font(),
    )
}

/// Straight-alpha RGBA pixels of a rendered image (GPUI's are BGRA).
fn rgba(g: &Graphic) -> RgbaImage {
    let size = g.image.size(0);
    let mut data = g.image.as_bytes(0).unwrap().to_vec();
    for px in data.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, data).unwrap()
}

fn pixel_size(g: &Graphic) -> (u32, u32) {
    let s = g.image.size(0);
    (s.width.0 as u32, s.height.0 as u32)
}

// ---------------------------------------------------------------------------
// Golden images for display math.

const MATH: &[(&str, &str)] = &[
    ("frac", r"\frac{a+b}{c-d} = \dfrac{1}{2} + \tfrac{3}{4}"),
    ("sqrt", r"\sqrt{x^2+y^2} + \sqrt[3]{8}"),
    (
        "matrix",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix} \begin{bmatrix} 1 & 0 \\ 0 & 1 \end{bmatrix}",
    ),
    (
        "cases",
        r"|x| = \begin{cases} x & x \ge 0 \\ -x & x < 0 \end{cases}",
    ),
    (
        "integral",
        r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
    ),
    (
        "sum",
        r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6} \qquad \prod_{i=1}^{n} i = n!",
    ),
    (
        "accents",
        r"\hat{a}\ \bar{b}\ \vec{v}\ \tilde{n}\ \dot{x}\ \ddot{y}\ \overline{AB}\ \widehat{xyz}",
    ),
    (
        "delimiters",
        r"\left( \frac{a}{b} \right) \left[ \sum_i x_i \right] \left\{ \int_0^1 f \right\} \left| \frac{1}{2} \right|",
    ),
    ("text", r"x = 1 \text{ if and only if } y > 0"),
    ("color", r"\color{red}{a} + \textcolor{blue}{b} = c"),
    (
        "greek",
        r"\alpha\beta\gamma\Gamma\Delta\Omega\omega \to \infty",
    ),
    (
        "demo",
        "\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}\n\\qquad\n\\sum_{n=1}^{\\infty} \\frac{1}{n^2} = \\frac{\\pi^2}{6}",
    ),
];

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn golden_dir() -> PathBuf {
    Path::new(MANIFEST_DIR).join("tests/golden/math")
}

fn failure_dir() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(MANIFEST_DIR).join("target"));
    target.join("golden-failures/math")
}

/// A channel may differ by this much (out of 255) without counting.
const CHANNEL_TOLERANCE: u8 = 8;
/// Share of pixels allowed to differ by more than that (antialiasing noise).
const MAX_DIFF_SHARE: f64 = 0.001;

/// Compares `actual` with the golden image `name`; returns a failure
/// message, after writing the actual image and a diff for inspection.
fn check_golden(name: &str, actual: &RgbaImage) -> Option<String> {
    let path = golden_dir().join(format!("{name}.png"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        actual.save(&path).unwrap();
        return None;
    }
    let fail_dir = failure_dir();
    let save_actual = || {
        std::fs::create_dir_all(&fail_dir).unwrap();
        let p = fail_dir.join(format!("{name}-actual.png"));
        actual.save(&p).unwrap();
        p
    };
    let expected = match image::open(&path) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            let p = save_actual();
            return Some(format!(
                "{name}: can't read {} ({e}); actual image: {}. Run with UPDATE_GOLDEN=1 to create it.",
                path.display(),
                p.display()
            ));
        }
    };
    if expected.dimensions() != actual.dimensions() {
        let p = save_actual();
        return Some(format!(
            "{name}: size {:?}, golden {:?}; actual image: {}",
            actual.dimensions(),
            expected.dimensions(),
            p.display()
        ));
    }
    let (w, h) = actual.dimensions();
    let mut diff = RgbaImage::new(w, h);
    let mut differing = 0usize;
    let mut worst = 0u8;
    for (x, y, a) in actual.enumerate_pixels() {
        let e = expected.get_pixel(x, y);
        let d = (0..4).map(|i| a[i].abs_diff(e[i])).max().unwrap();
        worst = worst.max(d);
        if d > CHANNEL_TOLERANCE {
            differing += 1;
            diff.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
        } else {
            // The expected image, faded, for orientation.
            let v = 255 - e[3] / 3;
            diff.put_pixel(x, y, image::Rgba([v, v, v, 255]));
        }
    }
    let share = differing as f64 / (w as f64 * h as f64);
    if share <= MAX_DIFF_SHARE {
        return None;
    }
    let p = save_actual();
    let d = fail_dir.join(format!("{name}-diff.png"));
    diff.save(&d).unwrap();
    Some(format!(
        "{name}: {differing} pixels ({:.2}%) differ, max channel difference {worst}; actual: {}, diff (red): {}",
        share * 100.,
        p.display(),
        d.display()
    ))
}

#[test]
fn math_golden_images() {
    let mut failures = Vec::new();
    let mut expected_files = Vec::new();
    let mut cases: Vec<(String, &str, bool, u32)> = Vec::new();
    for (name, tex) in MATH {
        for dark in [false, true] {
            let theme = if dark { "dark" } else { "light" };
            cases.push((format!("{name}-{theme}"), tex, dark, 100));
        }
    }
    // HiDPI: rasterized at the scale factor, shown at the same logical size.
    cases.push(("frac-light@2x".into(), MATH[0].1, false, 200));
    cases.push(("integral-dark@1.5x".into(), MATH[4].1, true, 150));
    for (name, tex, dark, scale) in &cases {
        expected_files.push(format!("{name}.png"));
        match render_math(tex, *dark, *scale) {
            Ok(g) => failures.extend(check_golden(name, &rgba(&g))),
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    // No stale goldens left behind after renaming or removing a case.
    let mut on_disk: Vec<String> = std::fs::read_dir(golden_dir())
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    on_disk.sort();
    expected_files.sort();
    if std::env::var_os("UPDATE_GOLDEN").is_none() && on_disk != expected_files {
        failures.push(format!(
            "tests/golden/math has {on_disk:?}, expected {expected_files:?}"
        ));
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn math_scale_keeps_the_logical_size() {
    let one = render_math(MATH[0].1, false, 100).unwrap();
    let two = render_math(MATH[0].1, false, 200).unwrap();
    let (w1, h1) = pixel_size(&one);
    let (w2, h2) = pixel_size(&two);
    assert_eq!(one.size, size(px(w1 as f32), px(h1 as f32)));
    assert!(w2.abs_diff(w1 * 2) <= 2 && h2.abs_diff(h1 * 2) <= 2);
    let dw = f32::from(two.size.width) - f32::from(one.size.width);
    let dh = f32::from(two.size.height) - f32::from(one.size.height);
    assert!(
        dw.abs() <= 1. && dh.abs() <= 1.,
        "{:?} vs {:?}",
        one.size,
        two.size
    );
    // Scale factors below 1 still render at 1.
    let half = render_math(MATH[0].1, false, 50).unwrap();
    assert_eq!(pixel_size(&half), (w1, h1));
}

#[test]
fn math_is_drawn_in_the_text_color() {
    for dark in [false, true] {
        let fg = gpui::Rgba::from(theme(dark).fg);
        let want = [fg.r, fg.g, fg.b].map(|v| (v * 255.).round() as i32);
        let img = rgba(&render_math(r"\frac{a}{b} + \sum_i x_i", dark, 100).unwrap());
        let mut opaque = 0;
        for p in img.pixels() {
            if p[3] == 255 {
                opaque += 1;
                for i in 0..3 {
                    assert!(
                        (p[i] as i32 - want[i]).abs() <= 1,
                        "dark={dark}: pixel {p:?}, fg {want:?}"
                    );
                }
            }
        }
        assert!(opaque > 20, "dark={dark}: formula is (nearly) blank");
        // The background is transparent.
        assert_eq!(img.get_pixel(0, 0)[3], 0);
    }
}

#[test]
fn math_errors() {
    for tex in [
        r"\frac{a}",
        r"\frac{",
        "}",
        "{",
        r"x^",
        r"\left( x",
        r"\begin{pmatrix} a",
        r"\end{matrix}",
        r"\notacommand",
        r"\sqrt[3",
        &format!("{}x{}", "{".repeat(100), "}".repeat(100)),
        &format!("{}x{}", r"\frac{1}{".repeat(100), "}".repeat(100)),
    ] {
        let r = std::panic::catch_unwind(|| render_math(tex, false, 100));
        match r {
            Ok(Err(e)) => assert!(e.starts_with("Math error: "), "{tex}: {e}"),
            Ok(Ok(_)) => panic!("{tex:?} rendered"),
            Err(_) => panic!("{tex:?} panicked"),
        }
    }
}

/// Every prefix of every formula (most of them malformed) gives a result or
/// an error, never a panic.
#[test]
fn math_prefixes_never_panic() {
    let mut panics = Vec::new();
    for (_, tex) in MATH {
        for (i, _) in tex.char_indices().skip(1) {
            let prefix = &tex[..i];
            if std::panic::catch_unwind(|| render_math(prefix, true, 100)).is_err() {
                panics.push(prefix.to_string());
            }
        }
    }
    assert!(panics.is_empty(), "panicked on: {panics:#?}");
}

/// Pinned: empty or blank sources typeset to a tiny (padding-only) image.
#[test]
fn math_empty() {
    for tex in ["", " ", "\n"] {
        let g = render_math(tex, false, 100).unwrap();
        let (w, h) = pixel_size(&g);
        assert!(w <= 8 && h <= 8, "{tex:?}: {w}×{h}");
    }
}

// ---------------------------------------------------------------------------
// Mermaid.

const DIAGRAMS: &[(&str, &str)] = &[
    (
        "flowchart",
        "flowchart TD\n  A[Start] --> B{Is it?}\n  B -->|Yes| C[OK]\n  B -->|No| D[End]\n  C --> D",
    ),
    ("graph", "graph LR; A-->B; B-->C; C-->A"),
    (
        "sequence",
        "sequenceDiagram\n  participant A as Alice\n  participant B as Bob\n  A->>B: Hello Bob\n  B-->>A: Hi Alice\n  Note over A,B: A note\n  loop Every minute\n    A->>B: ping\n  end",
    ),
    (
        "class",
        "classDiagram\n  class Animal {\n    +String name\n    +eat() void\n  }\n  class Dog\n  Animal <|-- Dog\n  Dog : +bark()",
    ),
    (
        "state",
        "stateDiagram-v2\n  [*] --> Still\n  Still --> Moving\n  Moving --> Still\n  Moving --> Crash\n  Crash --> [*]",
    ),
    (
        "er",
        "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE-ITEM : contains\n  CUSTOMER {\n    string name\n    int id\n  }",
    ),
    (
        "gantt",
        "gantt\n  title A Gantt Diagram\n  dateFormat YYYY-MM-DD\n  section Section\n  A task :a1, 2024-01-01, 30d\n  Another task :after a1, 20d",
    ),
    (
        "pie",
        "pie title Pets\n  \"Dogs\" : 386\n  \"Cats\" : 85\n  \"Rats\" : 15",
    ),
    (
        "mindmap",
        "mindmap\n  root((mindmap))\n    Origins\n      Long history\n    Research\n      On effectiveness\n    Tools\n      Pen and paper",
    ),
    (
        "journey",
        "journey\n  title My working day\n  section Go to work\n    Make tea: 5: Me\n    Go upstairs: 3: Me\n    Do work: 1: Me, Cat",
    ),
    (
        "gitgraph",
        "gitGraph\n  commit\n  branch develop\n  checkout develop\n  commit\n  checkout main\n  merge develop",
    ),
    (
        "timeline",
        "timeline\n  title History\n  2002 : LinkedIn\n  2004 : Facebook : Google\n  2005 : YouTube",
    ),
    (
        "quadrant",
        "quadrantChart\n  title Reach\n  x-axis Low Reach --> High Reach\n  y-axis Low Engagement --> High Engagement\n  quadrant-1 Expand\n  quadrant-2 Promote\n  quadrant-3 Re-evaluate\n  quadrant-4 Improve\n  Campaign A: [0.3, 0.6]\n  Campaign B: [0.45, 0.23]",
    ),
    (
        "unicode",
        "flowchart LR\n  A[Café → naïve] --> B[日本語]\n  B --> C[\"Quotes & <amp>\"]",
    ),
];

/// Mermaid and math sources from every document in `examples/`.
fn example_graphics() -> Vec<(String, Kind, String)> {
    let mut out = Vec::new();
    let dir = Path::new(MANIFEST_DIR).join("examples");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    fn walk(blocks: &[Block], name: &str, out: &mut Vec<(String, Kind, String)>) {
        for b in blocks {
            match b {
                Block::Math(s) => out.push((name.into(), Kind::Math, s.to_string())),
                Block::Mermaid(s) => out.push((name.into(), Kind::Mermaid, s.to_string())),
                Block::Quote { blocks, .. } | Block::Footnote { blocks, .. } => {
                    walk(blocks, name, out)
                }
                Block::List { items, .. } => {
                    for i in items {
                        walk(&i.blocks, name, out);
                    }
                }
                _ => {}
            }
        }
    }
    for f in files {
        let doc = markdown::parse(&std::fs::read_to_string(&f).unwrap());
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        walk(&doc.blocks, &name, &mut out);
    }
    out
}

/// Not blank: has visible pixels in more than one color.
fn assert_visible(name: &str, g: &Graphic) {
    let img = rgba(g);
    let (w, h) = img.dimensions();
    assert!(w >= 8 && h >= 8, "{name}: {w}×{h}");
    let visible = img.pixels().filter(|p| p[3] > 0).count();
    assert!(visible > 50, "{name}: only {visible} visible pixels");
    let mut colors: Vec<[u8; 4]> = img.pixels().map(|p| p.0).collect();
    colors.sort();
    colors.dedup();
    assert!(colors.len() >= 3, "{name}: {} colors", colors.len());
}

fn assert_within_max_side(name: &str, g: &Graphic) {
    let (w, h) = pixel_size(g);
    assert!(
        w as f32 <= MAX_SIDE && h as f32 <= MAX_SIDE,
        "{name}: {w}×{h} exceeds MAX_SIDE"
    );
}

#[test]
fn mermaid_diagrams_render() {
    let mut failures = Vec::new();
    for (name, src) in DIAGRAMS {
        for dark in [false, true] {
            match render_mermaid(src, dark, 100) {
                Ok(g) => {
                    assert_visible(name, &g);
                    assert_within_max_side(name, &g);
                }
                Err(e) => failures.push(format!("{name} (dark={dark}): {e}")),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn example_documents_render() {
    let all = example_graphics();
    assert!(
        all.iter().any(|(_, k, _)| *k == Kind::Mermaid)
            && all.iter().any(|(_, k, _)| *k == Kind::Math),
        "examples/ should contain math and a diagram"
    );
    for (file, kind, src) in all {
        for dark in [false, true] {
            let k = key(kind, &src, dark, 100);
            let g = render(&k, &theme(dark), ui_font())
                .unwrap_or_else(|e| panic!("{file}: {kind:?} failed: {e}\n{src}"));
            assert_visible(&file, &g);
            assert_within_max_side(&file, &g);
        }
    }
}

#[test]
fn huge_diagrams_are_scaled_down_to_max_side() {
    let chain: String = (0..120)
        .map(|i| format!("  N{i} --> N{}\n", i + 1))
        .collect();
    let src = format!("flowchart LR\n{chain}");
    for scale in [100, 200] {
        let g = render_mermaid(&src, false, scale).unwrap();
        let (w, _) = pixel_size(&g);
        assert_eq!(w as f32, MAX_SIDE, "scale {scale}");
        // The logical size is still the diagram's natural size.
        assert!(f32::from(g.size.width) > MAX_SIDE);
    }
}

#[test]
fn mermaid_errors() {
    for src in [
        "",
        "this is not mermaid",
        "flowchart LR\n  A -->",
        "graph TD; A[unclosed",
        "sequenceDiagram\n  A->>",
        "classDiagram\n  class {",
        "pie\n  \"a\" : notanumber",
        "stateDiagram-v2\n  [*] --> ",
        "erDiagram\n  A ||--o{",
        // Resource limits of merman.
        &format!(
            "flowchart LR\n{}A{}",
            (0..300)
                .map(|i| format!("subgraph S{i}\n"))
                .collect::<String>(),
            "\nend".repeat(300)
        ),
    ] {
        let r = std::panic::catch_unwind(|| render_mermaid(src, false, 100));
        match r {
            Ok(Err(e)) => assert!(!e.is_empty()),
            Ok(Ok(_)) => panic!("{src:?} rendered"),
            Err(_) => panic!("{src:?} panicked"),
        }
    }
}

#[test]
fn mermaid_prefixes_never_panic() {
    let mut panics = Vec::new();
    for (_, src) in &DIAGRAMS[..3] {
        for (i, _) in src.char_indices().skip(1).step_by(3) {
            let prefix = &src[..i];
            if std::panic::catch_unwind(|| render_mermaid(prefix, false, 100)).is_err() {
                panics.push(prefix.to_string());
            }
        }
    }
    assert!(panics.is_empty(), "panicked on: {panics:#?}");
}

#[test]
fn mermaid_config_uses_the_theme() {
    for dark in [false, true] {
        let t = theme(dark);
        let c = mermaid_config(&t, "Some Font");
        assert_eq!(c["theme"], "base");
        assert_eq!(c["darkMode"], dark);
        assert_eq!(c["fontFamily"], "Some Font, sans-serif");
        let v = &c["themeVariables"];
        assert_eq!(v["fontFamily"], "Some Font, sans-serif");
        assert_eq!(v["background"], hex(t.bg));
        assert_eq!(v["primaryTextColor"], hex(t.fg));
        assert_eq!(v["primaryColor"], hex(t.code_bg));
        assert_eq!(v["lineColor"], hex(t.muted));
        assert_eq!(v["pie1"], hex(t.accent));
        for (k, val) in v.as_object().unwrap() {
            if k != "fontFamily" {
                let s = val.as_str().unwrap();
                assert!(
                    s.len() == 7
                        && s.starts_with('#')
                        && s[1..].chars().all(|c| c.is_ascii_hexdigit()),
                    "{k}: {s}"
                );
            }
        }
    }
}

#[test]
fn color_helpers() {
    assert_eq!(hex(Theme::dark().bg), "#282c34");
    assert_eq!(hex(Theme::light().fg), "#242529");
    assert_eq!(hex(Hsla::white()), "#ffffff");
    let [r, g, b] = rgb(Hsla::black());
    assert_eq!([r, g, b], [0., 0., 0.]);
    let [r, g, b] = rgb(Theme::dark().bg);
    assert!((r - 0x28 as f32 / 255.).abs() < 1e-3);
    assert!((g - 0x2c as f32 / 255.).abs() < 1e-3);
    assert!((b - 0x34 as f32 / 255.).abs() < 1e-3);
}

// ---------------------------------------------------------------------------
// Rasterizing.

fn raster(svg: &str, scale: f32) -> Result<Graphic, String> {
    rasterize(svg, scale, &EMPTY_FONTS)
}

#[test]
fn rasterize_converts_to_straight_bgra() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="1">
        <rect x="0" width="1" height="1" fill="#ff0000"/>
        <rect x="1" width="1" height="1" fill="#0000ff" fill-opacity="0.5"/>
    </svg>"##;
    let g = raster(svg, 1.).unwrap();
    assert_eq!(pixel_size(&g), (3, 1));
    assert_eq!(g.size, size(px(3.), px(1.)));
    let bytes = g.image.as_bytes(0).unwrap();
    assert_eq!(&bytes[0..4], &[0, 0, 255, 255], "opaque red, BGRA");
    // Half-transparent blue: straight (not premultiplied) color.
    assert_eq!(bytes[4], 255);
    assert_eq!(&bytes[5..7], &[0, 0]);
    assert!(bytes[7].abs_diff(128) <= 1, "alpha {}", bytes[7]);
    assert_eq!(&bytes[8..12], &[0, 0, 0, 0], "transparent");
}

#[test]
fn rasterize_scales() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="4"><rect width="10" height="4"/></svg>"#;
    let g = raster(svg, 2.).unwrap();
    assert_eq!(pixel_size(&g), (20, 8));
    assert_eq!(g.size, size(px(10.), px(4.)));
    let g = raster(svg, 1.5).unwrap();
    assert_eq!(pixel_size(&g), (15, 6));
    // Below 1 is clamped to 1.
    let g = raster(svg, 0.25).unwrap();
    assert_eq!(pixel_size(&g), (10, 4));
}

#[test]
fn rasterize_caps_the_size() {
    for (w, h) in [
        (10_000, 10),
        (4097, 3),
        (3, 9_999),
        (12_345, 6_789),
        (8191, 8191),
        (5000, 1),
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><rect width="{w}" height="{h}"/></svg>"#
        );
        for scale in [1., 2.] {
            let g = raster(&svg, scale).unwrap();
            let (pw, ph) = pixel_size(&g);
            assert!(
                pw as f32 <= MAX_SIDE && ph as f32 <= MAX_SIDE,
                "{w}×{h} @{scale}: {pw}×{ph}"
            );
            assert_eq!(pw.max(ph) as f32, MAX_SIDE, "{w}×{h} @{scale}");
        }
    }
}

#[test]
fn rasterize_errors() {
    assert!(raster("not svg", 1.).is_err());
    assert!(raster("<svg", 1.).is_err());
    assert!(
        raster(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#,
            1.
        )
        .is_err()
    );
}

// ---------------------------------------------------------------------------
// Panics.

#[test]
fn guarded_turns_panics_into_errors() {
    let r = guarded(|| panic!("renderer bug"));
    assert_eq!(
        r.err().as_deref(),
        Some("Could not render this block (renderer crashed)")
    );
    let r = guarded(|| Err("Math error: x".into()));
    assert_eq!(r.err().as_deref(), Some("Math error: x"));
    let r = guarded(|| Ok(graphic(2, 2)));
    assert_eq!(pixel_size(&r.unwrap()), (2, 2));
    // A panic with a non-string payload too.
    let r = guarded(|| std::panic::panic_any(42_u8));
    assert!(r.is_err());
}

/// The release profile must unwind, or `guarded` can't catch anything.
#[test]
fn release_profile_unwinds() {
    let toml = std::fs::read_to_string(Path::new(MANIFEST_DIR).join("Cargo.toml")).unwrap();
    let release = toml.split("[profile.release]").nth(1).unwrap();
    let release = release.split("\n[").next().unwrap();
    assert!(
        release
            .lines()
            .any(|l| l.trim().replace(' ', "") == "panic=\"unwind\""),
        "[profile.release] must keep panic = \"unwind\""
    );
}

// ---------------------------------------------------------------------------
// The cache.

fn graphic(w: u32, h: u32) -> Graphic {
    Graphic {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(RgbaImage::new(
            w, h,
        ))])),
        size: size(px(w as f32), px(h as f32)),
    }
}

fn k(name: &str) -> Key {
    key(Kind::Math, name, false, 100)
}

fn is_ready(cache: &mut Cache, name: &str) -> bool {
    matches!(cache.slots.get(&k(name)), Some(Slot::Ready { .. }))
}

#[test]
fn cache_budget_constants() {
    // Documented in docs/rendering.md.
    assert_eq!(CACHE_BYTES, 128 * 1024 * 1024);
    assert_eq!(MAX_SIDE, 4096.);
}

#[test]
fn cache_lookup_states() {
    let mut cache = Cache::default();
    assert!(matches!(cache.lookup(&k("a")), Lookup::Missing));
    assert!(
        matches!(cache.lookup(&k("a")), Lookup::Pending),
        "only rendered once"
    );
    assert!(cache.finish(k("a"), Ok(graphic(2, 3))).is_empty());
    match cache.lookup(&k("a")) {
        Lookup::Ready(g) => assert_eq!(pixel_size(&g), (2, 3)),
        _ => panic!("not ready"),
    }
    assert_eq!(cache.bytes, 2 * 3 * 4);

    assert!(matches!(cache.lookup(&k("bad")), Lookup::Missing));
    cache.finish(k("bad"), Err("Math error: nope".into()));
    match cache.lookup(&k("bad")) {
        Lookup::Failed(e) => assert_eq!(e.as_ref(), "Math error: nope"),
        _ => panic!("not failed"),
    }
    assert_eq!(cache.bytes, 24, "errors cost no bytes");
}

#[test]
fn cache_key_includes_everything_that_changes_pixels() {
    let mut cache = Cache::default();
    let keys = [
        key(Kind::Math, "x", false, 100),
        key(Kind::Math, "x", true, 100),
        key(Kind::Math, "x", false, 200),
        key(Kind::Mermaid, "x", false, 100),
        key(Kind::Math, "y", false, 100),
    ];
    for key in &keys {
        assert!(
            matches!(cache.lookup(key), Lookup::Missing),
            "{:?}",
            key.source
        );
    }
    assert_eq!(cache.slots.len(), keys.len());
}

#[test]
fn cache_evicts_least_recently_used() {
    let mut cache = Cache::default();
    // 40 bytes each, budget 100: at most two fit.
    let budget = 100;
    for name in ["a", "b"] {
        cache.lookup(&k(name));
        assert!(
            cache
                .finish_within(k(name), Ok(graphic(10, 1)), budget)
                .is_empty()
        );
    }
    assert_eq!(cache.bytes, 80);
    // Use "a" again: "b" is now the least recently used.
    assert!(matches!(cache.lookup(&k("a")), Lookup::Ready(_)));
    cache.lookup(&k("c"));
    let evicted = cache.finish_within(k("c"), Ok(graphic(10, 1)), budget);
    assert_eq!(evicted.len(), 1);
    assert!(!is_ready(&mut cache, "b"));
    assert!(is_ready(&mut cache, "a") && is_ready(&mut cache, "c"));
    assert_eq!(cache.bytes, 80);
    assert!(
        matches!(cache.lookup(&k("b")), Lookup::Missing),
        "rendered again"
    );

    // A big image evicts as many as needed, oldest first.
    cache.lookup(&k("d"));
    let evicted = cache.finish_within(k("d"), Ok(graphic(20, 1)), budget);
    assert_eq!(evicted.len(), 2);
    assert!(is_ready(&mut cache, "d"));
    assert_eq!(cache.bytes, 80);
}

#[test]
fn cache_returns_the_evicted_images() {
    let mut cache = Cache::default();
    cache.finish_within(k("a"), Ok(graphic(10, 1)), 50);
    let stored = match cache.lookup(&k("a")) {
        Lookup::Ready(g) => g.image.clone(),
        _ => unreachable!(),
    };
    let evicted = cache.finish_within(k("b"), Ok(graphic(10, 1)), 50);
    assert_eq!(evicted.len(), 1);
    assert!(Arc::ptr_eq(&evicted[0], &stored), "the GPU texture to drop");
}

#[test]
fn cache_keeps_the_new_image_even_over_budget() {
    let mut cache = Cache::default();
    cache.finish_within(k("a"), Ok(graphic(1, 1)), 10);
    let evicted = cache.finish_within(k("huge"), Ok(graphic(100, 100)), 10);
    assert_eq!(evicted.len(), 1);
    assert!(is_ready(&mut cache, "huge"));
    assert_eq!(cache.bytes, 100 * 100 * 4);
    // The next one evicts it, though.
    let evicted = cache.finish_within(k("b"), Ok(graphic(1, 1)), 10);
    assert_eq!(evicted.len(), 1);
    assert!(!is_ready(&mut cache, "huge"));
    assert_eq!(cache.bytes, 4);
}

#[test]
fn cache_never_evicts_pending_or_failed_slots() {
    let mut cache = Cache::default();
    cache.lookup(&k("pending"));
    cache.lookup(&k("failed"));
    cache.finish_within(k("failed"), Err("e".into()), 0);
    let evicted = cache.finish_within(k("a"), Ok(graphic(10, 10)), 0);
    assert!(evicted.is_empty());
    assert!(matches!(cache.lookup(&k("pending")), Lookup::Pending));
    assert!(matches!(cache.lookup(&k("failed")), Lookup::Failed(_)));
}

#[test]
fn cache_with_the_real_budget() {
    let mut cache = Cache::default();
    // 1024 × 1024 × 4 = 4 MiB each: 32 fit in 128 MiB.
    for i in 0..32 {
        let evicted = cache.finish(k(&i.to_string()), Ok(graphic(1024, 1024)));
        assert!(evicted.is_empty(), "{i}");
    }
    assert_eq!(cache.bytes, CACHE_BYTES);
    let evicted = cache.finish(k("one more"), Ok(graphic(1024, 1024)));
    assert_eq!(evicted.len(), 1);
    assert!(!is_ready(&mut cache, "0"), "the oldest goes");
    assert_eq!(cache.bytes, CACHE_BYTES);
}

#[test]
fn rasterize_never_gives_an_empty_size() {
    // Issue #54: a side that rounds to 0 pixels (here an extreme aspect
    // ratio, scaled down to `MAX_SIDE`) got a 1-pixel image but a 0px
    // logical size, so the aspect ratio was `inf` or `NaN`.
    for (w, h) in [("1e30", "1e-30"), ("1e-30", "1e30"), ("1e20", "1e-20")] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><rect width="1" height="1"/></svg>"#
        );
        for scale in [1., 2.] {
            let g = raster(&svg, scale).unwrap();
            let (lw, lh) = (f32::from(g.size.width), f32::from(g.size.height));
            assert!(lw.is_finite() && lw > 0., "{w}×{h} @{scale}: width {lw}");
            assert!(lh.is_finite() && lh > 0., "{w}×{h} @{scale}: height {lh}");
            let ratio = aspect_ratio(g.size).unwrap();
            assert!(ratio.is_finite() && ratio > 0., "{w}×{h} @{scale}: {ratio}");
        }
    }
}

#[test]
fn aspect_ratio_of_bad_sizes_is_none() {
    assert_eq!(aspect_ratio(size(px(10.), px(4.))), Some(2.5));
    for (w, h) in [
        (10., 0.),
        (0., 10.),
        (0., 0.),
        (f32::INFINITY, 1.),
        (1., f32::NAN),
    ] {
        assert_eq!(aspect_ratio(size(px(w), px(h))), None, "{w}×{h}");
    }
}

// ---------------------------------------------------------------------------
// Fallback fonts for diagram labels.

use usvg::fontdb::{Database, FaceInfo, ID, Language, Source, Stretch, Style, Weight};

/// A face backed by one of the KaTeX fonts (for its glyphs), described with
/// the given family, style, weight and pitch, so each test controls exactly
/// what `fallback_font` ranks.
fn face(db: &mut Database, data: &str, family: &str, style: Style, weight: u16, mono: bool) -> ID {
    let bytes = ratex_katex_fonts::ttf_bytes(&format!("KaTeX_{data}.ttf"))
        .unwrap()
        .into_owned();
    db.push_face_info(FaceInfo {
        id: ID::dummy(),
        source: Source::Binary(Arc::new(bytes)),
        index: 0,
        families: vec![(family.into(), Language::English_UnitedStates)],
        post_script_name: family.replace(' ', ""),
        style,
        weight: Weight(weight),
        stretch: Stretch::Normal,
        monospaced: mono,
    })
}

fn fallback(db: Database, c: char, base: ID) -> Option<ID> {
    fallback_font(c, &[base], &mut Arc::new(db))
}

// What the KaTeX fonts used below cover.
const ARROW: char = '→'; // Main-Regular and AMS-Regular, no italic face
const CHECK: char = '✓'; // AMS-Regular only
const BANG: char = '!'; // Main-*, Typewriter; not AMS-Regular

#[test]
fn katex_fonts_cover_what_the_fallback_tests_assume() {
    let has = |file: &str, c: char| {
        let data = ratex_katex_fonts::ttf_bytes(&format!("KaTeX_{file}.ttf")).unwrap();
        ttf_parser::Face::parse(&data, 0)
            .unwrap()
            .glyph_index(c)
            .is_some()
    };
    for (file, arrow, check, bang) in [
        ("Main-Italic", false, false, true),
        ("Main-Regular", true, false, true),
        ("AMS-Regular", true, true, false),
        ("Typewriter-Regular", false, false, true),
    ] {
        assert_eq!(has(file, ARROW), arrow, "{file} {ARROW}");
        assert_eq!(has(file, CHECK), check, "{file} {CHECK}");
        assert_eq!(has(file, BANG), bang, "{file} {BANG}");
    }
}

/// Issue #55: symbol, emoji and CJK fonts are mostly upright only, so an
/// italic label found no fallback at all and drew a missing glyph box.
#[test]
fn fallback_for_italic_text_takes_an_upright_face_when_no_italic_one_has_it() {
    let mut db = Database::new();
    let base = face(
        &mut db,
        "Main-Italic",
        "Test Sans",
        Style::Italic,
        400,
        false,
    );
    face(
        &mut db,
        "Main-Italic",
        "Other Serif",
        Style::Italic,
        400,
        false,
    );
    let symbols = face(
        &mut db,
        "AMS-Regular",
        "Test Sans Symbols",
        Style::Normal,
        400,
        false,
    );
    assert_eq!(fallback(db, CHECK, base), Some(symbols));

    for style in [Style::Italic, Style::Oblique] {
        let mut db = Database::new();
        let base = face(&mut db, "Main-Italic", "Test Sans", style, 400, false);
        let upright = face(
            &mut db,
            "Main-Regular",
            "Other Sans",
            Style::Normal,
            400,
            false,
        );
        assert_eq!(fallback(db, ARROW, base), Some(upright), "{style:?}");
    }
}

#[test]
fn fallback_prefers_the_label_style_when_several_faces_have_it() {
    // The matching face comes last: a tie would pick the first one.
    for style in [Style::Italic, Style::Oblique, Style::Normal] {
        let mut db = Database::new();
        let base = face(&mut db, "AMS-Regular", "Test Sans", style, 400, false);
        let mut others = Vec::new();
        for s in [Style::Normal, Style::Italic, Style::Oblique] {
            if s != style {
                others.push(face(&mut db, "Main-Regular", "Other", s, 400, false));
            }
        }
        let same = face(&mut db, "Main-Regular", "Other", style, 400, false);
        assert_eq!(fallback(db, BANG, base), Some(same), "{style:?}");
    }

    // Italic and oblique are closer to each other than to upright.
    for (label, slanted) in [
        (Style::Italic, Style::Oblique),
        (Style::Oblique, Style::Italic),
    ] {
        let mut db = Database::new();
        let base = face(&mut db, "AMS-Regular", "Test Sans", label, 400, false);
        face(&mut db, "Main-Regular", "Other", Style::Normal, 400, false);
        let close = face(&mut db, "Main-Regular", "Other", slanted, 400, false);
        assert_eq!(fallback(db, BANG, base), Some(close), "{label:?}");
    }
}

#[test]
fn fallback_ranks_monospace_first_then_style_then_family_group() {
    // A proportional upright face beats a monospace italic one.
    let mut db = Database::new();
    let base = face(
        &mut db,
        "AMS-Regular",
        "Test Sans",
        Style::Italic,
        400,
        false,
    );
    let upright = face(&mut db, "Main-Regular", "Other", Style::Normal, 400, false);
    face(
        &mut db,
        "Typewriter-Regular",
        "Other",
        Style::Italic,
        400,
        true,
    );
    face(
        &mut db,
        "Typewriter-Regular",
        "Other Mono",
        Style::Italic,
        400,
        false,
    );
    assert_eq!(fallback(db, BANG, base), Some(upright));

    // An italic face of another family group beats an upright one of the
    // label's group.
    let mut db = Database::new();
    let base = face(
        &mut db,
        "AMS-Regular",
        "Test Sans",
        Style::Italic,
        400,
        false,
    );
    face(
        &mut db,
        "Main-Regular",
        "Test Serif",
        Style::Normal,
        400,
        false,
    );
    let italic = face(&mut db, "Main-Regular", "Other", Style::Italic, 400, false);
    assert_eq!(fallback(db, BANG, base), Some(italic));
}

#[test]
fn fallback_then_prefers_the_family_group_symbols_and_weight() {
    let pick = |faces: &[(&str, &str, u16)], c: char| {
        let mut db = Database::new();
        let base = face(
            &mut db,
            "Main-Italic",
            "Test Sans",
            Style::Italic,
            400,
            false,
        );
        let ids: Vec<ID> = faces
            .iter()
            .map(|(data, family, weight)| {
                face(&mut db, data, family, Style::Italic, *weight, false)
            })
            .collect();
        let got = fallback(db, c, base)?;
        ids.iter().position(|id| *id == got)
    };
    // The same family group.
    assert_eq!(
        pick(
            &[
                ("AMS-Regular", "Other", 400),
                ("AMS-Regular", "Test Serif", 400)
            ],
            CHECK
        ),
        Some(1)
    );
    // A symbols face.
    assert_eq!(
        pick(
            &[
                ("AMS-Regular", "Other", 400),
                ("AMS-Regular", "Other Symbols", 400)
            ],
            CHECK
        ),
        Some(1)
    );
    // The closest weight, lighter or heavier.
    assert_eq!(
        pick(
            &[("AMS-Regular", "Other", 700), ("AMS-Regular", "Other", 300)],
            CHECK
        ),
        Some(1)
    );
    assert_eq!(
        pick(
            &[("AMS-Regular", "Other", 100), ("AMS-Regular", "Other", 500)],
            CHECK
        ),
        Some(1)
    );
    // Only faces that have the character, never the label's own.
    assert_eq!(
        pick(
            &[
                ("Main-Regular", "Test Sans", 400),
                ("AMS-Regular", "Other", 900)
            ],
            CHECK
        ),
        Some(1)
    );
    assert_eq!(pick(&[("Typewriter-Regular", "Other", 400)], CHECK), None);
}

#[test]
fn fallback_skips_excluded_faces_and_needs_a_base() {
    let mut db = Database::new();
    let base = face(
        &mut db,
        "Main-Italic",
        "Test Sans",
        Style::Italic,
        400,
        false,
    );
    let tried = face(
        &mut db,
        "AMS-Regular",
        "Test Sans Symbols",
        Style::Normal,
        400,
        false,
    );
    let other = face(&mut db, "AMS-Regular", "Other", Style::Normal, 400, false);
    let mut db = Arc::new(db);
    assert_eq!(fallback_font(CHECK, &[base, tried], &mut db), Some(other));
    assert_eq!(fallback_font(CHECK, &[base, tried, other], &mut db), None);
    assert_eq!(fallback_font(CHECK, &[], &mut db), None);
}

#[test]
fn fallback_avoids_mismatched_style_latin_takeover_and_preserves_italics_end_to_end() {
    // When an upright symbol face (lacking Latin) and an upright full-Latin
    // face in the same family both have `→`, prefer the symbol face so usvg
    // only replaces `→` and keeps the italic base face for the Latin text.
    let mut db = Database::new();
    let base = face(
        &mut db,
        "Main-Italic",
        "Test Sans",
        Style::Italic,
        400,
        false,
    );
    // Same family group as `base`, upright and oblique, both cover Latin + `→`.
    face(
        &mut db,
        "Main-Regular",
        "Test Sans",
        Style::Normal,
        400,
        false,
    );
    face(
        &mut db,
        "Main-Regular",
        "Test Sans",
        Style::Oblique,
        400,
        false,
    );
    // Different family group, no "Symbol" in its name, upright, no Latin `'a'`.
    let gap_filler = face(
        &mut db,
        "AMS-Regular",
        "Other Math",
        Style::Normal,
        400,
        false,
    );
    assert_eq!(fallback(db, ARROW, base), Some(gap_filler));

    // A matching-style face still beats an upright gap-filler.
    let mut db = Database::new();
    let base = face(
        &mut db,
        "Main-Italic",
        "Test Sans",
        Style::Italic,
        400,
        false,
    );
    face(
        &mut db,
        "AMS-Regular",
        "Other Math",
        Style::Normal,
        400,
        false,
    );
    let same_style = face(
        &mut db,
        "Main-Regular",
        "Other Sans",
        Style::Italic,
        400,
        false,
    );
    assert_eq!(fallback(db, ARROW, base), Some(same_style));

    // End-to-end through usvg/resvg: italic `"abc →"` renders `"abc"` with the
    // italic base face (identical to italic `"abc"` and distinct from upright)
    // while drawing `→` from the upright symbol fallback.
    let mut db = Database::new();
    face(
        &mut db,
        "Main-Italic",
        "Test Sans",
        Style::Italic,
        400,
        false,
    );
    face(
        &mut db,
        "Main-Regular",
        "Test Sans",
        Style::Normal,
        400,
        false,
    );
    face(
        &mut db,
        "AMS-Regular",
        "Other Math",
        Style::Normal,
        400,
        false,
    );
    let fonts = Arc::new(db);
    let svg = |style: &str, text: &str| {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="40">
                <text x="10" y="30" font-family="Test Sans" font-style="{style}" font-size="24" fill="#000000">{text}</text>
            </svg>"##
        )
    };
    let italic_arrow = rgba(&rasterize(&svg("italic", "abc →"), 1., &fonts).unwrap());
    let italic_only = rgba(&rasterize(&svg("italic", "abc"), 1., &fonts).unwrap());
    let upright_arrow = rgba(&rasterize(&svg("normal", "abc →"), 1., &fonts).unwrap());

    let abc_right = (0..italic_only.width())
        .filter(|&x| (0..italic_only.height()).any(|y| italic_only.get_pixel(x, y)[3] > 0))
        .max()
        .expect("italic abc rendered");
    let mut differs_from_upright = 0;
    for y in 0..italic_only.height() {
        for x in 0..=abc_right {
            assert_eq!(
                italic_arrow.get_pixel(x, y),
                italic_only.get_pixel(x, y),
                "italic base face lost at ({x}, {y})"
            );
            if italic_arrow.get_pixel(x, y) != upright_arrow.get_pixel(x, y) {
                differs_from_upright += 1;
            }
        }
    }
    assert!(
        differs_from_upright > 20,
        "italic and upright abc should look different ({differs_from_upright} px)"
    );
    let arrow_pixels = (abc_right + 1..italic_arrow.width())
        .flat_map(|x| (0..italic_arrow.height()).map(move |y| (x, y)))
        .filter(|&(x, y)| italic_arrow.get_pixel(x, y)[3] > 0)
        .count();
    assert!(
        arrow_pixels > 10,
        "arrow should render from the upright fallback ({arrow_pixels} px)"
    );
}

#[test]
fn style_distances() {
    use Style::*;
    for s in [Normal, Italic, Oblique] {
        assert_eq!(style_distance(s, s), 0);
    }
    assert_eq!(style_distance(Italic, Oblique), 1);
    assert_eq!(style_distance(Oblique, Italic), 1);
    for s in [Italic, Oblique] {
        assert_eq!(style_distance(Normal, s), 2);
        assert_eq!(style_distance(s, Normal), 2);
    }
}

// ---------------------------------------------------------------------------
// The previous image, while an edited block renders.

fn image_of(show: &Show) -> Option<&Arc<Graphic>> {
    match show {
        Show::Image(g) => Some(g),
        _ => None,
    }
}

fn is_placeholder(show: &Show) -> bool {
    matches!(show, Show::Placeholder)
}

fn cached(cache: &Cache, key: &Key) -> Option<Arc<Graphic>> {
    match cache.slots.get(key) {
        Some(Slot::Ready { graphic, .. }) => Some(graphic.clone()),
        _ => None,
    }
}

fn shown_key<'a>(shown: &'a Shown, block: &Arc<str>) -> Option<&'a Key> {
    shown.0.get(&BlockId(block.clone()))
}

#[test]
fn a_block_shows_a_placeholder_until_its_first_image_is_ready() {
    let (mut cache, mut shown) = (Cache::default(), Shown::default());
    let block: Arc<str> = "x".into();
    let (s, render) = show(&mut cache, &mut shown, &block, &k("x"));
    assert!(is_placeholder(&s) && render);
    let (s, render) = show(&mut cache, &mut shown, &block, &k("x"));
    assert!(is_placeholder(&s) && !render, "rendered once");
    assert_eq!(shown_len(&shown), 0);

    cache.finish(k("x"), Ok(graphic(4, 2)));
    let (s, render) = show(&mut cache, &mut shown, &block, &k("x"));
    assert_eq!(pixel_size(image_of(&s).unwrap()), (4, 2));
    assert!(!render);
    assert_eq!(shown_key(&shown, &block), Some(&k("x")));
}

#[test]
fn an_edited_block_keeps_its_previous_image_until_the_new_one_is_ready() {
    let (mut cache, mut shown) = (Cache::default(), Shown::default());
    let block: Arc<str> = "x".into();
    show(&mut cache, &mut shown, &block, &k("x"));
    cache.finish(k("x"), Ok(graphic(4, 2)));
    let old = image_of(&show(&mut cache, &mut shown, &block, &k("x")).0)
        .unwrap()
        .clone();

    // Edited: the new image renders, the old one stays, at its size.
    for first in [true, false] {
        let (s, render) = show(&mut cache, &mut shown, &block, &k("y"));
        assert!(Arc::ptr_eq(image_of(&s).unwrap(), &old));
        assert_eq!(render, first);
    }
    // Then the new one takes over and becomes the previous image.
    cache.finish(k("y"), Ok(graphic(6, 3)));
    let (s, _) = show(&mut cache, &mut shown, &block, &k("y"));
    let new = image_of(&s).unwrap().clone();
    assert_eq!(pixel_size(&new), (6, 3));
    assert_eq!(shown_key(&shown, &block), Some(&k("y")));
    let (s, _) = show(&mut cache, &mut shown, &block, &k("z"));
    assert!(Arc::ptr_eq(image_of(&s).unwrap(), &new));
}

#[test]
fn a_broken_edit_shows_the_error_not_the_previous_image() {
    let (mut cache, mut shown) = (Cache::default(), Shown::default());
    let block: Arc<str> = "x".into();
    show(&mut cache, &mut shown, &block, &k("x"));
    cache.finish(k("x"), Ok(graphic(4, 2)));
    show(&mut cache, &mut shown, &block, &k("x"));

    let (s, _) = show(&mut cache, &mut shown, &block, &k("bad"));
    assert!(
        image_of(&s).is_some(),
        "still the previous image while it renders"
    );
    cache.finish(k("bad"), Err("Math error: nope".into()));
    match show(&mut cache, &mut shown, &block, &k("bad")) {
        (Show::Error(e), false) => assert_eq!(e.as_ref(), "Math error: nope"),
        _ => panic!("not the error"),
    }
    assert_eq!(shown_len(&shown), 0, "forgotten");
    // Fixed again: there's nothing left to show meanwhile.
    let (s, render) = show(&mut cache, &mut shown, &block, &k("fixed"));
    assert!(is_placeholder(&s) && render);
}

#[test]
fn the_previous_image_is_kept_across_scale_factors_but_not_themes() {
    let (mut cache, mut shown) = (Cache::default(), Shown::default());
    let block: Arc<str> = "x".into();
    let light = key(Kind::Math, "x", false, 100);
    show(&mut cache, &mut shown, &block, &light);
    cache.finish(light.clone(), Ok(graphic(4, 2)));
    show(&mut cache, &mut shown, &block, &light);

    let (s, render) = show(
        &mut cache,
        &mut shown,
        &block,
        &key(Kind::Math, "x", false, 200),
    );
    assert!(image_of(&s).is_some() && render, "another scale factor");
    let (s, render) = show(
        &mut cache,
        &mut shown,
        &block,
        &key(Kind::Math, "x", true, 100),
    );
    assert!(is_placeholder(&s) && render, "the other theme's colors");
}

#[test]
fn an_evicted_previous_image_is_never_drawn_again() {
    let (mut cache, mut shown) = (Cache::default(), Shown::default());
    let block: Arc<str> = "x".into();
    cache.finish_within(k("a"), Ok(graphic(10, 1)), 50);
    show(&mut cache, &mut shown, &block, &k("a"));
    assert!(image_of(&show(&mut cache, &mut shown, &block, &k("b")).0).is_some());
    // Something else evicts it (and its texture is dropped).
    let evicted = cache.finish_within(k("other"), Ok(graphic(10, 1)), 40);
    assert_eq!(evicted.len(), 1);
    let (s, render) = show(&mut cache, &mut shown, &block, &k("b"));
    assert!(is_placeholder(&s) && !render);
    assert!(cached(&cache, &k("a")).is_none(), "not brought back");
    assert_eq!(cache.bytes, 40);
}

#[test]
fn a_previous_image_on_screen_is_the_last_to_be_evicted() {
    let (mut cache, mut shown) = (Cache::default(), Shown::default());
    let block: Arc<str> = "x".into();
    // 40 bytes each, room for two.
    let budget = 100;
    cache.finish_within(k("a"), Ok(graphic(10, 1)), budget);
    show(&mut cache, &mut shown, &block, &k("a"));
    cache.finish_within(k("d"), Ok(graphic(10, 1)), budget);
    // "a" is older than "d", but it's on screen while "b" renders.
    show(&mut cache, &mut shown, &block, &k("b"));
    cache.finish_within(k("c"), Ok(graphic(10, 1)), budget);
    assert!(cached(&cache, &k("a")).is_some());
    assert!(cached(&cache, &k("d")).is_none());
    assert!(cache.ready(&k("d")).is_none());
}

/// Live-reloading a block over and over: one previous image per block, and
/// the pixels stay within the budget, every image either cached or handed
/// back to have its texture dropped.
#[test]
fn repeated_edits_stay_within_the_budget() {
    let (mut cache, mut shown) = (Cache::default(), Shown::default());
    let block: Arc<str> = "x".into();
    let budget = 3 * 40;
    let mut evicted = 0;
    for i in 0..50 {
        let key = k(&i.to_string());
        let (s, render) = show(&mut cache, &mut shown, &block, &key);
        assert!(render);
        assert_eq!(is_placeholder(&s), i == 0, "{i}");
        evicted += cache
            .finish_within(key.clone(), Ok(graphic(10, 1)), budget)
            .len();
        assert!(cache.bytes <= budget, "{i}: {} bytes", cache.bytes);
        assert!(image_of(&show(&mut cache, &mut shown, &block, &key).0).is_some());
        assert_eq!(shown_len(&shown), 1);
    }
    let ready = cache
        .slots
        .values()
        .filter(|s| matches!(s, Slot::Ready { .. }))
        .count();
    assert_eq!(ready, 3);
    assert_eq!(ready + evicted, 50);
}

#[test]
fn cache_ready_never_starts_a_render() {
    let mut cache = Cache::default();
    assert!(cache.ready(&k("a")).is_none());
    assert!(cache.slots.is_empty());
    cache.lookup(&k("a"));
    assert!(cache.ready(&k("a")).is_none(), "pending");
    cache.finish(k("bad"), Err("e".into()));
    assert!(cache.ready(&k("bad")).is_none());
    cache.finish(k("a"), Ok(graphic(1, 1)));
    assert!(cache.ready(&k("a")).is_some());
}

// Which blocks are which across a reload.

/// The math and diagram blocks of `src`, as `(kind, source)`.
fn graphics_of(doc: &Document) -> Vec<(Kind, String)> {
    graphics_in(&doc.blocks)
        .into_iter()
        .map(|(kind, s)| (kind, s.to_string()))
        .collect()
}

#[test]
fn graphics_in_finds_nested_blocks_in_document_order() {
    let doc = markdown::parse(
        "$$\na\n$$\n\n> ```mermaid\n> graph LR; B-->C\n> ```\n\n\
         - item\n\n  $$\n  c\n  $$\n\nText[^n]\n\n[^n]: Note\n\n    $$\n    d\n    $$\n\n\
         # Heading\n\n| x |\n|---|\n| y |\n\n```rust\nfn main() {}\n```\n\n---\n\n![i](i.png)\n",
    );
    assert_eq!(
        graphics_of(&doc),
        [
            (Kind::Math, "a".into()),
            (Kind::Mermaid, "graph LR; B-->C".into()),
            (Kind::Math, "c".into()),
            (Kind::Math, "d".into()),
        ]
    );
}

fn as_refs(v: &[(Kind, Arc<str>)]) -> Vec<(Kind, &Arc<str>)> {
    v.iter().map(|(k, s)| (*k, s)).collect()
}

/// `pair_up` for blocks written as letters: a lowercase one is math, an
/// uppercase one a diagram, and the letter is its source.
fn pairs(old: &str, new: &str) -> Vec<(usize, usize)> {
    let blocks = |s: &str| -> Vec<(Kind, Arc<str>)> {
        s.chars()
            .map(|c| {
                let kind = if c.is_lowercase() {
                    Kind::Math
                } else {
                    Kind::Mermaid
                };
                (kind, c.to_lowercase().to_string().into())
            })
            .collect()
    };
    let (old, new) = (blocks(old), blocks(new));
    let mut p = pair_up(&as_refs(&old), &as_refs(&new));
    p.sort();
    p
}

#[test]
fn pair_up_matches_unchanged_ends_then_the_middle_one_to_one() {
    // Old blocks, new blocks, and which old one each new one replaces.
    type Case = (&'static str, &'static str, &'static [(usize, usize)]);
    let cases: &[Case] = &[
        ("", "", &[]),
        ("a", "", &[]),
        ("", "a", &[]),
        // Unchanged.
        ("aB", "aB", &[(0, 0), (1, 1)]),
        // One edited, at the start, in the middle or at the end.
        ("aBc", "xBc", &[(0, 0), (1, 1), (2, 2)]),
        ("aBc", "aYc", &[(0, 0), (1, 1), (2, 2)]),
        ("aBc", "aBx", &[(0, 0), (1, 1), (2, 2)]),
        // Two edited.
        ("aBc", "xYc", &[(0, 0), (1, 1), (2, 2)]),
        // Edited into another kind: not the same block.
        ("aB", "AB", &[(1, 1)]),
        ("aB", "Ab", &[]),
        // Inserted or removed: the others keep theirs, a new one has none.
        ("ac", "axc", &[(0, 0), (1, 2)]),
        ("axc", "ac", &[(0, 0), (2, 1)]),
        ("a", "xa", &[(0, 1)]),
        ("a", "ax", &[(0, 0)]),
        // Edited and inserted: unclear which is which.
        ("aBc", "axYc", &[(0, 0), (2, 3)]),
        // Deleted one and inserted another: keep the unchanged block, leave
        // the inserted one unmatched instead of pairing it with another block.
        ("ab", "bc", &[(1, 0)]),
        ("ab", "ca", &[(0, 1)]),
        ("AB", "BC", &[(1, 0)]),
        // Unchanged middle block splits edits and insertions into separate gaps.
        ("abc", "xby", &[(0, 0), (1, 1), (2, 2)]),
        ("abc", "axcy", &[(0, 0), (1, 1), (2, 2)]),
        ("abcd", "bxc", &[(1, 0), (2, 2)]),
        // Longer middle LCS wins whether advancing old or new first, and
        // consecutive identical anchors advance both indices without reusing
        // one new element for repeated old elements.
        ("xbc", "bcx", &[(1, 0), (2, 1)]),
        ("bcx", "xbc", &[(0, 1), (1, 2)]),
        ("abbc", "dbbe", &[(0, 0), (1, 1), (2, 2), (3, 3)]),
        ("aaabc", "bcxay", &[(3, 0), (4, 1)]),
        // Reordered blocks match an unchanged one without cross-pairing.
        ("ab", "ba", &[(1, 0)]),
        // Duplicates aren't counted twice.
        ("aa", "a", &[(0, 0)]),
        ("a", "aa", &[(0, 0)]),
        ("aa", "aa", &[(0, 0), (1, 1)]),
    ];
    for (old, new, want) in cases {
        assert_eq!(pairs(old, new), *want, "{old:?} → {new:?}");
    }
}

#[test]
fn pair_up_caps_middle_diff_table_size() {
    // At the exact cap (255 + 1) * (255 + 1) == 65_536, the middle pairs;
    // one block larger on either side skips the middle (even with a shared
    // middle element) and keeps only the unchanged ends.
    let make = |prefix: &str, count: usize, mid: bool| -> Vec<(Kind, Arc<str>)> {
        let mut v = vec![(Kind::Math, "head".into())];
        for i in 0..count {
            let s = if mid && i == 1 {
                "mid".into()
            } else {
                format!("{prefix}{i}").into()
            };
            v.push((Kind::Math, s));
        }
        v.push((Kind::Math, "tail".into()));
        v
    };
    let old_fit = make("a", 255, false);
    let new_fit = make("b", 255, false);
    assert_eq!(
        pair_up(&as_refs(&old_fit), &as_refs(&new_fit)).len(),
        257,
        "exact cap pairs the middle"
    );
    for (m, n) in [(256, 255), (255, 256)] {
        let old_big = make("a", m, true);
        let new_big = make("b", n, true);
        assert_eq!(
            pair_up(&as_refs(&old_big), &as_refs(&new_big)),
            vec![(0, 0), (m + 1, n + 1)],
            "{m}×{n}: above cap keeps only the unchanged ends"
        );
    }
}

#[test]
fn carry_over_moves_previous_images_to_the_edited_blocks() {
    let old = markdown::parse("$$\na\n$$\n\n> $$\n> b\n> $$\n\n- $$\n  c\n  $$\n");
    let new = markdown::parse("# New\n\n$$\na\n$$\n\n> $$\n> b2\n> $$\n\n- $$\n  c\n  $$\n");
    let (old_g, new_g) = (graphics_in(&old.blocks), graphics_in(&new.blocks));
    assert_eq!(old_g.len(), 3);
    assert_eq!(new_g.len(), 3);
    let mut shown = Shown::default();
    for (i, (_, s)) in old_g.iter().enumerate() {
        shown.0.insert(BlockId((*s).clone()), k(&format!("key{i}")));
    }
    shown.carry_over(&old, &new);
    for (i, (_, s)) in new_g.iter().enumerate() {
        assert_eq!(shown_key(&shown, s), Some(&k(&format!("key{i}"))), "{i}");
    }
    // Only the new blocks are left; the old ones' sources are let go.
    assert_eq!(shown_len(&shown), 3);
    for (_, s) in &old_g {
        assert!(shown_key(&shown, s).is_none());
        assert_eq!(Arc::strong_count(s), 1);
    }

    // A block that showed nothing yet stays without a previous image.
    let mut shown = Shown::default();
    shown.carry_over(&old, &new);
    assert_eq!(shown_len(&shown), 0);
}

#[test]
fn block_ids_are_the_source_arcs_not_their_text() {
    use std::hash::{BuildHasher, RandomState};
    let a: Arc<str> = "same".into();
    let b: Arc<str> = "same".into();
    assert!(BlockId(a.clone()) == BlockId(a.clone()));
    assert!(BlockId(a.clone()) != BlockId(b.clone()));
    let hasher = RandomState::new();
    assert_eq!(
        hasher.hash_one(BlockId(a.clone())),
        hasher.hash_one(BlockId(a.clone()))
    );
    assert_ne!(
        hasher.hash_one(BlockId(a.clone())),
        hasher.hash_one(BlockId(b.clone()))
    );
}

#[test]
fn cache_ready_makes_the_image_the_most_recently_used() {
    let used = |cache: &Cache, name: &str| match cache.slots.get(&k(name)) {
        Some(Slot::Ready { used, .. }) => *used,
        _ => panic!("{name} not ready"),
    };
    let mut cache = Cache::default();
    cache.finish(k("a"), Ok(graphic(1, 1)));
    cache.finish(k("b"), Ok(graphic(1, 1)));
    assert!(used(&cache, "a") < used(&cache, "b"));
    cache.ready(&k("a"));
    assert!(used(&cache, "a") > used(&cache, "b"), "strictly newer");
}
