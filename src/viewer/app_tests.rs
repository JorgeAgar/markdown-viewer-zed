//! Viewer tests on GPUI's headless test platform (`gpui/test-support`):
//! actions, opening files, live reload, link clicks, and drawing every
//! example document. Windows are drawn whenever they're dirty at the end of
//! an update, so a panic while rendering fails the test.

use std::time::Duration;

use gpui::{
    AnyView, AppContext, AvailableSpace, Entity, TestAppContext, VisualTestContext, point, size,
};

use super::*;
use crate::{graphics, render, test_util::TempDir};

fn fonts() -> Fonts {
    Fonts {
        ui: "Noto Sans".into(),
        mono: "Noto Sans Mono".into(),
    }
}

fn parsed(src: &str) -> Result<(Document, Option<SystemTime>), String> {
    Ok((markdown::parse(src), None))
}

/// A window with a Viewer showing `loaded`.
fn open(loaded: Vec<Loaded>) -> (Entity<Viewer>, &'static mut VisualTestContext) {
    let cx = Box::leak(Box::new(TestAppContext::single()));
    cx.add_window_view(|window, cx| Viewer::new(loaded, fonts(), window, cx))
}

fn open_docs(names: &[&str]) -> (Entity<Viewer>, &'static mut VisualTestContext) {
    open(
        names
            .iter()
            .map(|n| (PathBuf::from(format!("/x/{n}")), parsed(&format!("# {n}"))))
            .collect(),
    )
}

fn titles(v: &Entity<Viewer>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        v.read(cx)
            .tabs
            .iter()
            .map(|t| t.title.to_string())
            .collect()
    })
}

fn active(v: &Entity<Viewer>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| v.read(cx).tabs.active().map(|t| t.title.to_string()))
}

fn active_text(v: &Entity<Viewer>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| format!("{:?}", v.read(cx).tabs.active().unwrap().doc.blocks))
}

fn scroll_top(v: &Entity<Viewer>, cx: &mut VisualTestContext) -> (usize, f32) {
    cx.update(|_, cx| {
        let top = v.read(cx).active_list().unwrap().logical_scroll_top();
        (top.item_ix, f32::from(top.offset_in_item))
    })
}

/// The title the Viewer last set (the test window doesn't keep it).
fn window_title(v: &Entity<Viewer>, cx: &mut VisualTestContext) -> String {
    cx.run_until_parked();
    cx.update(|_, cx| v.read(cx).window_title.to_string())
}

#[test]
fn starts_on_the_last_loaded_tab() {
    let (v, cx) = open_docs(&["a.md", "b.md", "c.md"]);
    assert_eq!(titles(&v, cx), ["a.md", "b.md", "c.md"]);
    assert_eq!(active(&v, cx).as_deref(), Some("c.md"));
    assert_eq!(window_title(&v, cx), "c.md — mdview");
}

#[test]
fn duplicate_startup_paths_open_one_tab() {
    let (v, cx) = open_docs(&["a.md", "b.md", "a.md"]);
    assert_eq!(titles(&v, cx), ["a.md", "b.md"]);
    assert_eq!(active(&v, cx).as_deref(), Some("a.md"));
}

#[test]
fn next_and_previous_tab_wrap_around() {
    let (v, cx) = open_docs(&["a.md", "b.md", "c.md"]);
    cx.dispatch_action(NextTab);
    assert_eq!(active(&v, cx).as_deref(), Some("a.md"));
    cx.dispatch_action(NextTab);
    assert_eq!(active(&v, cx).as_deref(), Some("b.md"));
    cx.dispatch_action(PrevTab);
    cx.dispatch_action(PrevTab);
    assert_eq!(active(&v, cx).as_deref(), Some("c.md"));
    assert_eq!(window_title(&v, cx), "c.md — mdview");
}

#[test]
fn close_tab_activates_the_neighbor_then_closes_the_window() {
    let (v, cx) = open_docs(&["a.md", "b.md", "c.md"]);
    cx.dispatch_action(PrevTab); // b
    cx.dispatch_action(CloseTab);
    assert_eq!(titles(&v, cx), ["a.md", "c.md"]);
    assert_eq!(active(&v, cx).as_deref(), Some("c.md"), "right neighbor");
    cx.dispatch_action(CloseTab);
    assert_eq!(active(&v, cx).as_deref(), Some("a.md"), "left neighbor");
    assert_eq!(window_title(&v, cx), "a.md — mdview");
    cx.dispatch_action(CloseTab);
    assert!(titles(&v, cx).is_empty());
    assert_eq!(window_title(&v, cx), "mdview");
    // Tab actions without tabs are no-ops.
    cx.dispatch_action(NextTab);
    cx.dispatch_action(PrevTab);
    cx.dispatch_action(Reload);
    cx.dispatch_action(ScrollDown);
    cx.dispatch_action(Bottom);
    assert_eq!(cx.windows().len(), 1);
    // Closing with no tabs closes the window.
    cx.dispatch_action(CloseTab);
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
}

#[test]
fn open_paths_dedupes_skips_directories_and_shows_errors() {
    let dir = TempDir::new("open");
    std::fs::write(dir.join("a.md"), "# A\n").unwrap();
    std::fs::write(dir.join("b.md"), "# B\n").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    let (v, cx) = open(vec![load(normalize(&dir.join("a.md")))]);
    v.update(cx, |v, cx| {
        v.open_paths(
            vec![
                dir.join("b.md"),
                dir.join("sub"),
                dir.join("sub/../a.md"),
                dir.join("./b.md"),
                dir.join("missing.md"),
            ],
            cx,
        )
    });
    assert_eq!(titles(&v, cx), ["a.md", "b.md", "missing.md"]);
    assert_eq!(active(&v, cx).as_deref(), Some("missing.md"));
    assert!(active_text(&v, cx).contains("Could not open file"));
    // Re-opening switches to the tab.
    v.update(cx, |v, cx| v.open_paths(vec![dir.join("a.md")], cx));
    assert_eq!(active(&v, cx).as_deref(), Some("a.md"));
    assert_eq!(titles(&v, cx).len(), 3);
    // Only directories: nothing happens.
    v.update(cx, |v, cx| v.open_paths(vec![dir.to_path_buf()], cx));
    assert_eq!(titles(&v, cx).len(), 3);
    assert_eq!(active(&v, cx).as_deref(), Some("a.md"));
}

#[test]
fn reload_action_rereads_the_active_tab() {
    let dir = TempDir::new("reload");
    let path = normalize(&dir.join("a.md"));
    std::fs::write(&path, "# Before\n").unwrap();
    let (v, cx) = open(vec![load(path.clone())]);
    assert!(active_text(&v, cx).contains("Before"));
    std::fs::write(&path, "# After\n").unwrap();
    cx.dispatch_action(Reload);
    assert!(active_text(&v, cx).contains("After"));
    std::fs::remove_file(&path).unwrap();
    cx.dispatch_action(Reload);
    assert!(active_text(&v, cx).contains("Could not open file"));
}

fn set_mtime(path: &Path, secs: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

#[test]
fn live_reload_polls_modification_times() {
    let dir = TempDir::new("live");
    let a = normalize(&dir.join("a.md"));
    let b = normalize(&dir.join("b.md"));
    std::fs::write(&a, "# A1\n").unwrap();
    std::fs::write(&b, "# B1\n").unwrap();
    set_mtime(&a, 1_000);
    set_mtime(&b, 1_000);
    let (v, cx) = open(vec![load(a.clone()), load(b.clone())]);
    let doc_of = |v: &Entity<Viewer>, cx: &mut VisualTestContext, ix: usize| {
        cx.update(|_, cx| format!("{:?}", v.read(cx).tabs.iter().nth(ix).unwrap().doc.blocks))
    };

    // Same mtime: not reloaded, even if the content changed.
    std::fs::write(&a, "# A2\n").unwrap();
    set_mtime(&a, 1_000);
    cx.executor().advance_clock(Duration::from_millis(800));
    cx.run_until_parked();
    assert!(doc_of(&v, cx, 0).contains("A1"));

    // New mtime: reloaded at the next poll, background tab included.
    set_mtime(&a, 2_000);
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    assert!(
        doc_of(&v, cx, 0).contains("A1"),
        "not before the 750 ms timer"
    );
    cx.executor().advance_clock(Duration::from_millis(800));
    cx.run_until_parked();
    assert!(doc_of(&v, cx, 0).contains("A2"));
    assert!(doc_of(&v, cx, 1).contains("B1"));

    // Deleted: the last content stays.
    std::fs::remove_file(&b).unwrap();
    cx.executor().advance_clock(Duration::from_millis(800));
    cx.run_until_parked();
    assert!(doc_of(&v, cx, 1).contains("B1"));
    // Back again: reloaded.
    std::fs::write(&b, "# B2\n").unwrap();
    cx.executor().advance_clock(Duration::from_millis(800));
    cx.run_until_parked();
    assert!(doc_of(&v, cx, 1).contains("B2"));
}

#[test]
fn live_reload_stops_with_the_window() {
    let dir = TempDir::new("live-stop");
    let a = normalize(&dir.join("a.md"));
    std::fs::write(&a, "# A\n").unwrap();
    let (_v, cx) = open(vec![load(a)]);
    cx.dispatch_action(CloseTab);
    cx.dispatch_action(CloseTab);
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
    // The poll loop notices the Viewer is gone and ends instead of panicking.
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
}

fn long_doc() -> String {
    let mut s = String::from("# Top\n\n");
    for i in 0..60 {
        s += &format!(
            "## Section {i}\n\nSome text for paragraph {i}, long enough to wrap maybe.\n\n"
        );
    }
    s
}

#[test]
fn scroll_actions() {
    let (v, cx) = open(vec![(PathBuf::from("/x/long.md"), parsed(&long_doc()))]);
    cx.run_until_parked();
    assert_eq!(scroll_top(&v, cx), (0, 0.));
    cx.dispatch_action(ScrollDown);
    let one = scroll_top(&v, cx);
    assert!(one > (0, 0.), "{one:?}");
    cx.dispatch_action(PageDown);
    let page = scroll_top(&v, cx);
    assert!(page > one, "{page:?} vs {one:?}");
    cx.dispatch_action(ScrollUp);
    let back = scroll_top(&v, cx);
    assert!(back < page && back > (0, 0.), "{back:?}");
    cx.dispatch_action(PageUp);
    cx.dispatch_action(PageUp);
    assert_eq!(scroll_top(&v, cx), (0, 0.), "clamped at the top");
    cx.dispatch_action(Bottom);
    cx.run_until_parked();
    let bottom = scroll_top(&v, cx);
    assert!(bottom.0 > page.0, "{bottom:?}: the last row is revealed");
    cx.dispatch_action(Top);
    assert_eq!(scroll_top(&v, cx), (0, 0.));
}

#[test]
fn reload_keeps_the_scroll_position() {
    let dir = TempDir::new("reload-scroll");
    let path = normalize(&dir.join("long.md"));
    std::fs::write(&path, long_doc()).unwrap();
    let (v, cx) = open(vec![load(path.clone())]);
    v.update(cx, |v, cx| v.scroll_to_anchor(None, "section-30", cx));
    let before = scroll_top(&v, cx);
    assert_eq!(before.0, 61);
    cx.dispatch_action(Reload);
    assert_eq!(scroll_top(&v, cx), before);
}

#[test]
fn anchors_scroll_to_headings() {
    let (v, cx) = open(vec![
        (PathBuf::from("/x/long.md"), parsed(&long_doc())),
        (PathBuf::from("/x/other.md"), parsed("# Other")),
    ]);
    cx.dispatch_action(PrevTab);
    v.update(cx, |v, cx| v.scroll_to_anchor(None, "Section-10", cx));
    assert_eq!(scroll_top(&v, cx), (21, 0.));
    v.update(cx, |v, cx| v.scroll_to_anchor(None, "no-such-heading", cx));
    assert_eq!(scroll_top(&v, cx), (21, 0.), "unknown anchors don't move");
    // In another (open) tab, by path. It doesn't switch tabs by itself.
    cx.dispatch_action(NextTab);
    v.update(cx, |v, cx| {
        v.scroll_to_anchor(Some(Path::new("/x/long.md")), "section-2", cx)
    });
    cx.dispatch_action(PrevTab);
    assert_eq!(scroll_top(&v, cx), (5, 0.));
    // A path that isn't open: nothing.
    v.update(cx, |v, cx| {
        v.scroll_to_anchor(Some(Path::new("/x/nope.md")), "section-2", cx)
    });
}

#[test]
fn clicking_links_opens_tabs_and_scrolls() {
    let dir = TempDir::new("links");
    let base = normalize(&dir);
    std::fs::write(
        base.join("index.md"),
        "# Index\n\n[next](b%20c.md#Part-2)\n",
    )
    .unwrap();
    let filler = "Filler paragraph.\n\n".repeat(80);
    std::fs::write(
        base.join("b c.md"),
        format!("# Part 1\n\n{filler}# Part 2\n\n{filler}"),
    )
    .unwrap();
    let (v, cx) = open(vec![load(base.join("index.md"))]);
    let weak = v.downgrade();
    cx.update(|window, cx| render::open_link(&weak, &base, "b%20c.md#Part-2", window, cx));
    assert_eq!(titles(&v, cx), ["index.md", "b c.md"]);
    assert_eq!(active(&v, cx).as_deref(), Some("b c.md"));
    assert_eq!(scroll_top(&v, cx), (81, 0.));
    // An anchor link scrolls the current tab.
    cx.update(|window, cx| render::open_link(&weak, &base, "#part-1", window, cx));
    assert_eq!(scroll_top(&v, cx), (0, 0.));
    // Following the link again switches back to the open tab.
    cx.dispatch_action(PrevTab);
    cx.update(|window, cx| render::open_link(&weak, &base, "./b c.md", window, cx));
    assert_eq!(titles(&v, cx).len(), 2);
    assert_eq!(active(&v, cx).as_deref(), Some("b c.md"));
    // External links and other files go to the system, not to a tab.
    cx.update(|window, cx| {
        render::open_link(&weak, &base, "https://example.com/x.md", window, cx);
        render::open_link(&weak, &base, "image.png", window, cx);
    });
    assert_eq!(titles(&v, cx).len(), 2);
}

/// Markdown that stresses rendering: multibyte text around styled spans
/// and links, links nested in lists/quotes/tables/footnotes (each needs its
/// own ElementId), code, images, math and diagrams.
const STRESS: &str = r#"# Ünïcödé *héading* with [a link](#x) and `cödé`

Paragraph with **bold _nested italic_** ~~strike~~, [link 1](a.md), [link 2](b.md#é),
emoji 📝 [🔗 link](https://example.com) and `inline` code, all on one line: ñ.

> [!WARNING]
> A callout with [a link](c.md) and **bold**.
>
> > Nested quote with [another](d.md).

1. First [one](e.md)
   - nested [two](f.md)
     - deeper [three](g.md)
2. Second
- [x] done [task](h.md)
- [ ] todo

| Left | Center | Right |
|:-----|:------:|------:|
| [x](i.md) | **b** | `c` |
| ü | [y](j.md) | 3 |

Footnote ref[^1].

[^1]: The note, with [a link](k.md).

```rust
fn main() { println!("héllo {}", 1 + 2); } // comment
```

```
plain code block without a language
```

![missing image](nope.png) ![remote](https://example.com/x.png)

Inline ![img](x.png) image in text.

$$
\frac{a}{b} + \sum_{i=1}^n x_i
$$

$$
\notacommand{x}
$$

```mermaid
flowchart LR
  A --> B
```

```mermaid
not a diagram
```

---

###### Level six [link](l.md)
"#;

fn example_docs() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut docs: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .map(|p| {
            let src = std::fs::read_to_string(&p).unwrap();
            (p, src)
        })
        .collect();
    docs.sort();
    docs.push((PathBuf::from("/x/stress.md"), STRESS.into()));
    docs.push((PathBuf::from("/x/empty.md"), String::new()));
    docs.push((PathBuf::from("/x/README.md"), long_doc()));
    docs
}

/// Draws every row of every document in the window (light: the test
/// platform's appearance), scrolling through it, before and after the
/// math and diagram images are ready.
#[test]
fn draws_every_document_in_the_window() {
    let docs = example_docs();
    let loaded = docs.iter().map(|(p, s)| (p.clone(), parsed(s))).collect();
    let (v, cx) = open(loaded);
    for _ in 0..docs.len() {
        cx.dispatch_action(NextTab);
        cx.run_until_parked(); // background renders finish and redraw
        for _ in 0..200 {
            cx.dispatch_action(PageDown);
            cx.run_until_parked();
            let (ix, _) = scroll_top(&v, cx);
            let n = cx.update(|_, cx| v.read(cx).active_list().unwrap().item_count());
            if ix + 1 >= n {
                break;
            }
        }
        cx.dispatch_action(Top);
        cx.run_until_parked();
    }
}

/// The rows of a document, as the Viewer's list renders them, with any theme.
struct Rows {
    doc: Arc<Document>,
    rcx: RenderCx,
}

impl Render for Rows {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(920.)).children(
            self.doc
                .blocks
                .iter()
                .enumerate()
                .map(|(ix, b)| render::top_block(b, ix, &self.rcx, window, cx)),
        )
    }
}

/// Lays out and paints every block of every document with both themes (the
/// test window is always light).
#[test]
fn draws_every_document_in_both_themes() {
    let (v, cx) = open(vec![]);
    let weak = v.downgrade();
    for (path, src) in example_docs() {
        let doc = Arc::new(markdown::parse(&src));
        for theme in [Theme::light(), Theme::dark()] {
            let rcx = RenderCx {
                theme,
                ui_font: fonts().ui,
                mono_font: fonts().mono,
                base_dir: Arc::from(path.parent().unwrap()),
                viewer: weak.clone(),
            };
            let rows = cx.new(|_| Rows {
                doc: doc.clone(),
                rcx,
            });
            // Twice: placeholders first, then the finished images.
            for _ in 0..2 {
                cx.draw(
                    point(px(0.), px(0.)),
                    size(
                        AvailableSpace::Definite(px(920.)),
                        AvailableSpace::MinContent,
                    ),
                    |_, _| AnyView::from(rows.clone()),
                );
                cx.run_until_parked();
            }
        }
    }
    // The images were rendered and drawn: the math and diagram blocks of
    // the examples and the stress document, light and dark, plus the two
    // broken ones.
    let (math, mermaid) = cx.update(|_, cx| {
        (
            graphics::cache_stats(cx, graphics::Kind::Math),
            graphics::cache_stats(cx, graphics::Kind::Mermaid),
        )
    });
    assert!(math.0 >= 4 && math.1 == 2 && math.2 == 0, "math {math:?}");
    assert!(
        mermaid.0 >= 4 && mermaid.1 == 2 && mermaid.2 == 0,
        "mermaid {mermaid:?}"
    );
}

#[test]
fn standalone_remote_image_is_requested_by_the_viewer() {
    use gpui::http_client::{FakeHttpClient, Response};
    use std::sync::{Arc, Mutex};

    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let client = FakeHttpClient::create(move |request| {
        recorded.lock().unwrap().push(request.uri().to_string());
        async {
            Ok(Response::builder()
                .status(200)
                .body(include_bytes!("../../assets/icon.svg").as_slice().into())
                .unwrap())
        }
    });
    let cx = Box::leak(Box::new(TestAppContext::single()));
    cx.update(|cx| cx.set_http_client(client));
    let (_, cx) = cx.add_window_view(|window, cx| {
        Viewer::new(
            vec![(
                PathBuf::from("/docs/image.md"),
                parsed("![secure](https://example.com/a%20b.svg)\n\n![plain](http://example.com/a.svg)"),
            )],
            fonts(),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    let mut requests = requests.lock().unwrap().clone();
    requests.sort();
    assert_eq!(
        requests,
        ["http://example.com/a.svg", "https://example.com/a%20b.svg"]
    );
}
