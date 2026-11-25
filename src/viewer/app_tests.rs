//! Viewer tests on GPUI's headless test platform (`gpui/test-support`):
//! actions, opening files, live reload, link clicks, and drawing every
//! example document. Windows are drawn whenever they're dirty at the end of
//! an update, so a panic while rendering fails the test.

use std::time::Duration;

use gpui::{
    AnyView, AppContext, AvailableSpace, Entity, TestAppContext, VisualContext, VisualTestContext,
    point, size,
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
    let loaded = loaded
        .into_iter()
        .map(|(p, doc)| (normalize(&p), doc))
        .collect();
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
struct DocRows {
    doc: Arc<Document>,
    rows: Arc<Rows>,
    rcx: RenderCx,
}

impl Render for DocRows {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(920.)).children(
            (0..self.rows.len())
                .map(|ix| render::render_row(&self.doc, &self.rows[ix], ix, &self.rcx, window, cx)),
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
        let doc_rows = Arc::new(Rows::from_document(&doc));
        for theme in [Theme::light(), Theme::dark()] {
            let rcx = RenderCx {
                theme,
                ui_font: fonts().ui,
                mono_font: fonts().mono,
                base_dir: Arc::from(path.parent().unwrap()),
                viewer: weak.clone(),
                code_scroll: Default::default(),
            };
            let rows = cx.new(|_| DocRows {
                doc: doc.clone(),
                rows: doc_rows.clone(),
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

#[test]
fn platform_open_events_reuse_and_recreate_the_viewer() {
    let dir = TempDir::new("platform-open");
    let a = dir.join("a.md");
    let b = dir.join("b.md");
    std::fs::write(&a, "# A").unwrap();
    std::fs::write(&b, "# B").unwrap();
    let cx = TestAppContext::single();
    let mut handle = cx.update(|cx| crate::open_viewer(Vec::new(), cx));
    cx.update(|cx| {
        crate::deliver_paths(&mut handle, vec![a.clone()], cx);
        crate::deliver_paths(&mut handle, vec![b.clone()], cx);
        assert_eq!(cx.windows().len(), 1);
        let view = handle.entity(cx).unwrap();
        let names: Vec<_> = view
            .read(cx)
            .tabs
            .iter()
            .map(|t| t.title.to_string())
            .collect();
        assert_eq!(names, ["a.md", "b.md"]);
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    cx.update(|cx| {
        assert!(cx.windows().is_empty());
        crate::deliver_paths(&mut handle, vec![b], cx);
        assert_eq!(cx.windows().len(), 1);
        let view = handle.entity(cx).unwrap();
        assert_eq!(view.read(cx).tabs.active().unwrap().title.as_ref(), "b.md");
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    cx.update(|cx| {
        crate::deliver_paths(&mut handle, vec![], cx);
        assert_eq!(cx.windows().len(), 1);
        assert!(handle.entity(cx).unwrap().read(cx).tabs.is_empty());
    });
}

#[test]
fn open_dialog_directory_prefers_active_tab_then_last_opened_then_cwd() {
    let dir = TempDir::new("open-dialog-dir");
    let d1 = dir.join("one");
    let d2 = dir.join("two");
    std::fs::create_dir(&d1).unwrap();
    std::fs::create_dir(&d2).unwrap();
    let f1 = d1.join("a.md");
    let f2 = d2.join("b.md");
    std::fs::write(&f1, "# One\n").unwrap();
    std::fs::write(&f2, "# Two\n").unwrap();
    let d1 = normalize(&d1);
    let d2 = normalize(&d2);
    let cwd = std::env::current_dir().unwrap();

    let dialog_dir = |v: &Entity<Viewer>, cx: &mut VisualTestContext| {
        cx.update(|_, cx| v.read(cx).open_dialog_directory())
    };

    // Empty window starts in the process working directory.
    let (v, cx) = open(vec![]);
    assert_eq!(dialog_dir(&v, cx), Some(cwd.clone()));

    // Opening files uses the active tab's parent directory.
    v.update(cx, |v, cx| v.open_paths(vec![f1.clone()], cx));
    assert_eq!(dialog_dir(&v, cx), Some(d1.clone()));
    v.update(cx, |v, cx| v.open_paths(vec![f2.clone()], cx));
    assert_eq!(dialog_dir(&v, cx), Some(d2.clone()));

    // Switching tabs changes the dialog directory to the active tab's parent.
    cx.dispatch_action(PrevTab);
    assert_eq!(dialog_dir(&v, cx), Some(d1.clone()));

    // Closing all tabs falls back to the last opened directory (d2).
    cx.dispatch_action(CloseTab);
    cx.dispatch_action(CloseTab);
    assert!(titles(&v, cx).is_empty());
    assert_eq!(dialog_dir(&v, cx), Some(d2.clone()));

    // Re-opening an already open tab updates last_open_dir too.
    v.update(cx, |v, cx| v.open_paths(vec![f1.clone(), f2.clone()], cx));
    v.update(cx, |v, cx| v.open_paths(vec![f1.clone()], cx));
    // Opening a path in a non-existent directory does not clobber last_open_dir.
    v.update(cx, |v, cx| {
        v.open_paths(vec![dir.join("missing/nope.md")], cx)
    });
    cx.dispatch_action(CloseTab);
    cx.dispatch_action(CloseTab);
    cx.dispatch_action(CloseTab);
    assert!(titles(&v, cx).is_empty());
    assert_eq!(dialog_dir(&v, cx), Some(d1.clone()));

    // Startup-loaded files also populate last_open_dir after their tab closes.
    let (v2, cx2) = open(vec![load(normalize(&f2))]);
    assert_eq!(dialog_dir(&v2, cx2), Some(d2.clone()));
    cx2.dispatch_action(CloseTab);
    assert_eq!(dialog_dir(&v2, cx2), Some(d2.clone()));

    // If the recorded directory disappears, fall back to cwd.
    std::fs::remove_dir_all(&d2).unwrap();
    assert_eq!(dialog_dir(&v2, cx2), Some(cwd));
}

#[test]
fn large_table_virtualizes_rows_and_jumps_to_bottom() {
    let mut md = String::from("# Big table\n\n| ID | Desc |\n|---:|:---|\n");
    for i in 1..=2_000 {
        md.push_str(&format!("| {i:05} | Row **{i}** `v-{i}` |\n"));
    }
    let (v, cx) = open(vec![(PathBuf::from("/x/big.md"), parsed(&md))]);
    cx.run_until_parked();
    // 1 heading + 1 table header + 2000 body rows.
    let count = cx.update(|_, cx| v.read(cx).active_list().unwrap().item_count());
    assert_eq!(count, 2_002);

    // Scrolling down advances smoothly into the table.
    cx.dispatch_action(PageDown);
    cx.run_until_parked();
    let after_page = scroll_top(&v, cx);
    assert!(after_page.0 > 0 && after_page.0 < 200, "{after_page:?}");

    // Bottom jumps all the way to the end of the 2000-row table even though
    // middle rows were never measured.
    cx.dispatch_action(Bottom);
    cx.run_until_parked();
    let bottom = scroll_top(&v, cx);
    assert!(
        bottom.0 > 1_900 && bottom.0 < count,
        "expected scroll top near the end of the table, got {bottom:?}"
    );

    // PageUp from the bottom steps smoothly upward without teleporting over
    // unmeasured rows.
    cx.dispatch_action(PageUp);
    cx.run_until_parked();
    let after_page_up = scroll_top(&v, cx);
    assert!(
        after_page_up.0 > 1_800 && after_page_up.0 < bottom.0,
        "expected PageUp from bottom to move up one page, got {after_page_up:?} from {bottom:?}"
    );

    // Top returns cleanly to the start.
    cx.dispatch_action(Top);
    cx.run_until_parked();
    assert_eq!(scroll_top(&v, cx), (0, 0.));

    // Bottom on an empty document stays at (0, 0.).
    let (empty_v, empty_cx) = open(vec![(PathBuf::from("/x/empty.md"), parsed(""))]);
    empty_cx.dispatch_action(Bottom);
    empty_cx.run_until_parked();
    assert_eq!(scroll_top(&empty_v, empty_cx), (0, 0.));
}

#[test]
fn anchors_account_for_split_tables_lists_code_and_quotes() {
    let mut md = String::new();
    // Block 0: table (1 header + 3 body rows = 4 rows)
    md.push_str("| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |\n\n");
    // Block 1: list (3 items = 3 rows)
    md.push_str("- a\n- b\n- c\n\n");
    // Block 2: large code block (70 lines -> 2 chunks = 2 rows)
    md.push_str("```rust\n/* multi-line\n");
    for i in 0..68 {
        md.push_str(&format!("let x{i} = {i};\n"));
    }
    md.push_str("*/\n```\n\n");
    // Block 3: multi-block callout quote (1 header + 2 blocks = 3 rows)
    md.push_str("> [!NOTE]\n> First\n>\n> Second\n\n");
    // Block 4: multi-block plain quote (2 blocks = 2 rows)
    md.push_str("> Plain one\n>\n> Plain two\n\n");
    // Block 5: header-only table (1 row)
    md.push_str("| Only |\n|---|\n\n");
    // Block 6: target heading -> row index 4 + 3 + 2 + 3 + 2 + 1 = 15
    md.push_str("## Target Heading\n\nBody\n");

    let (v, cx) = open(vec![(PathBuf::from("/x/mixed.md"), parsed(&md))]);
    cx.run_until_parked();
    v.update(cx, |v, cx| v.scroll_to_anchor(None, "target-heading", cx));
    cx.run_until_parked();
    // Anchor targets row 15 (clamped upward only if the tail is shorter than
    // the viewport; here we assert the tab's row mapping directly too).
    let mapped = cx.update(|_, cx| {
        let tab = v.read(cx).tabs.active().unwrap();
        let block_ix = links::heading_index(&tab.doc.blocks, "target-heading").unwrap();
        tab.rows.row_for_block(block_ix)
    });
    assert_eq!(mapped, Some(15));
}

#[test]
fn draws_split_large_code_quotes_and_header_only_table_in_both_themes() {
    let mut md = String::from("| Header only |\n|---|\n\n");
    md.push_str("> [!TIP]\n> First paragraph\n>\n> Second paragraph\n\n");
    md.push_str("> Plain first\n>\n> Plain second\n\n");
    md.push_str("```rust\n/* start comment\n");
    for i in 0..130 {
        md.push_str(&format!("// line {i}\n"));
    }
    md.push_str("end */\n\n```\n");

    let (v, cx) = open(vec![]);
    let weak = v.downgrade();
    let doc = Arc::new(markdown::parse(&md));
    let doc_rows = Arc::new(Rows::from_document(&doc));
    assert!(
        doc_rows.len() > 8,
        "expected split rows, got {}",
        doc_rows.len()
    );
    for theme in [Theme::light(), Theme::dark()] {
        let rcx = RenderCx {
            theme,
            ui_font: fonts().ui,
            mono_font: fonts().mono,
            base_dir: Arc::from(Path::new("/x")),
            viewer: weak.clone(),
            code_scroll: Default::default(),
        };
        let rows = cx.new(|_| DocRows {
            doc: doc.clone(),
            rows: doc_rows.clone(),
            rcx,
        });
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

#[test]
fn reload_keeps_heading_position_when_preceding_table_grows_or_shrinks() {
    let make = |rows: usize| {
        let mut s = String::from("| ID | Desc |\n|---:|:---|\n");
        for i in 1..=rows {
            s.push_str(&format!("| {i} | row {i} |\n"));
        }
        s.push_str("\n## After Table\n\n");
        for i in 0..40 {
            s.push_str(&format!("Trailing paragraph {i}.\n\n"));
        }
        s
    };
    let dir = TempDir::new("reload-table-grow");
    let path = normalize(&dir.join("table.md"));
    std::fs::write(&path, make(5)).unwrap();

    let (v, cx) = open(vec![load(path.clone())]);
    cx.run_until_parked();
    v.update(cx, |v, cx| v.scroll_to_anchor(None, "after-table", cx));
    cx.run_until_parked();
    assert_eq!(scroll_top(&v, cx).0, 6);

    // Table grows from 5 to 105 rows: reload keeps `## After Table` (now row 106).
    std::fs::write(&path, make(105)).unwrap();
    cx.dispatch_action(Reload);
    cx.run_until_parked();
    assert_eq!(scroll_top(&v, cx).0, 106);

    // Table shrinks to 2 rows: reload keeps `## After Table` (now row 3).
    std::fs::write(&path, make(2)).unwrap();
    cx.dispatch_action(Reload);
    cx.run_until_parked();
    assert_eq!(scroll_top(&v, cx).0, 3);
}

#[test]
fn split_code_block_synchronizes_horizontal_scroll_across_chunks() {
    use gpui::{ScrollDelta, ScrollWheelEvent};

    // 90 lines (> 64 -> 2 chunks): lines 1..=64 are very wide, lines 65..=90
    // are short. Both chunks must stay horizontally synchronized and the
    // shorter second chunk must not clamp away the horizontal scroll offset.
    let mut md = String::from("```rust\n");
    for i in 1..=64 {
        md.push_str(&format!("let line_{i:03} = \"{}\";\n", "x".repeat(220)));
    }
    for i in 65..=90 {
        md.push_str(&format!("let short_{i:03} = {i};\n"));
    }
    md.push_str("```\n");

    let (v, cx) = open(vec![(PathBuf::from("/x/wide-code.md"), parsed(&md))]);
    cx.run_until_parked();
    let count = cx.update(|_, cx| v.read(cx).active_list().unwrap().item_count());
    assert_eq!(count, 2, "90-line code block splits into 2 chunks");

    // Scroll vertically so the boundary between line 64 (chunk 0) and line 65
    // (chunk 1) is visible on screen at the same time.
    cx.update(|_, cx| {
        v.read(cx).active_list().unwrap().scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(1050.),
        });
    });
    cx.update(|_, cx| v.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();

    let (b0, b1) = cx.update(|_, cx| {
        let list = v.read(cx).active_list().unwrap();
        (
            list.bounds_for_item(0).unwrap(),
            list.bounds_for_item(1).unwrap(),
        )
    });
    let viewport = cx.update(|_, cx| v.read(cx).active_list().unwrap().viewport_bounds());
    assert!(
        b0.bottom() > viewport.top() && b0.bottom() < viewport.bottom(),
        "chunk 0 visible"
    );
    assert!(
        b1.top() >= viewport.top() && b1.top() < viewport.bottom(),
        "chunk 1 visible"
    );

    // Horizontal scroll gesture over chunk 0 (line 64).
    let pos_chunk0 = point(b0.center().x, b0.bottom() - px(10.));
    cx.simulate_event(ScrollWheelEvent {
        position: pos_chunk0,
        delta: ScrollDelta::Pixels(point(px(-150.), px(0.))),
        ..Default::default()
    });
    cx.run_until_parked();

    let offset_after_chunk0 = cx.update(|_, cx| {
        v.read(cx)
            .tabs
            .active()
            .unwrap()
            .code_scroll
            .borrow()
            .get(&0)
            .expect("shared code scroll handle created")
            .handle
            .offset()
    });
    assert_eq!(
        offset_after_chunk0.x,
        px(-150.),
        "chunk 1 (shorter lines) must not clamp away chunk 0's horizontal scroll"
    );

    // Horizontal scroll gesture over chunk 1 (line 65) updates the same handle.
    let pos_chunk1 = point(b1.center().x, b1.top() + px(10.));
    cx.simulate_event(ScrollWheelEvent {
        position: pos_chunk1,
        delta: ScrollDelta::Pixels(point(px(-80.), px(0.))),
        ..Default::default()
    });
    cx.run_until_parked();

    let offset_after_chunk1 = cx.update(|_, cx| {
        v.read(cx)
            .tabs
            .active()
            .unwrap()
            .code_scroll
            .borrow()
            .get(&0)
            .unwrap()
            .handle
            .offset()
    });
    assert_eq!(offset_after_chunk1.x, px(-230.));
}

#[test]
fn split_code_block_with_wide_emoji_does_not_clamp_on_second_chunk() {
    use gpui::{ScrollDelta, ScrollWheelEvent};

    // Exact PR #16 regression + adversarial zero-width Unicode lines: first 64
    // lines have 100 emoji each; second chunk has 110 ASCII `x`, 60 tabs, and
    // 150 zero-width joiners (beating every static char/byte/non-ASCII/tab
    // heuristic). `SharedWidth` must still adopt chunk 0's actual shaped width
    // so scrolling -150px horizontally on chunk 0 is never clamped.
    let mut md = String::from("```text\n");
    let emoji_line = "😀".repeat(100);
    for _ in 1..=64 {
        md.push_str(&emoji_line);
        md.push('\n');
    }
    md.push_str(&"x".repeat(110));
    md.push('\n');
    md.push_str(&"\t".repeat(60));
    md.push('\n');
    md.push_str(&"\u{200d}".repeat(150));
    md.push('\n');
    for i in 68..=90 {
        md.push_str(&format!("line {i}\n"));
    }
    md.push_str("```\n");

    let (v, cx) = open(vec![(PathBuf::from("/x/emoji-code.md"), parsed(&md))]);
    cx.run_until_parked();
    cx.update(|_, cx| {
        v.read(cx).active_list().unwrap().scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(1050.),
        });
    });
    cx.update(|_, cx| v.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();

    let b0 = cx.update(|_, cx| {
        v.read(cx)
            .active_list()
            .unwrap()
            .bounds_for_item(0)
            .unwrap()
    });
    cx.simulate_event(ScrollWheelEvent {
        position: point(b0.center().x, b0.bottom() - px(10.)),
        delta: ScrollDelta::Pixels(point(px(-150.), px(0.))),
        ..Default::default()
    });
    cx.run_until_parked();

    let offset = cx.update(|_, cx| {
        v.read(cx)
            .tabs
            .active()
            .unwrap()
            .code_scroll
            .borrow()
            .get(&0)
            .unwrap()
            .handle
            .offset()
    });
    assert_eq!(offset.x, px(-150.));
}

#[test]
fn reload_keeps_shrunk_code_block_visible_instead_of_hiding_above_viewport() {
    let make = |lines: usize| {
        let mut s = String::from("```rust\n");
        for i in 0..lines {
            s.push_str(&format!("let line_{i} = {i};\n"));
        }
        s.push_str("```\n\n");
        for i in 0..40 {
            s.push_str(&format!("Paragraph {i} after the code block.\n\n"));
        }
        s
    };

    let dir = TempDir::new("reload-shrunk-code");
    let path = normalize(&dir.join("code.md"));
    std::fs::write(&path, make(150)).unwrap();

    let (v, cx) = open(vec![load(path.clone())]);
    cx.run_until_parked();

    // Scroll 500px inside the second chunk (item_ix = 1) of the 150-line block.
    cx.update(|_, cx| {
        v.read(cx).active_list().unwrap().scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(500.),
        });
    });
    cx.update(|_, cx| v.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    assert_eq!(scroll_top(&v, cx), (1, 500.));

    // Shrink the code block to 1 line and reload: offset_in_item must reset so
    // the shrunk block stays visible at the top of the viewport.
    std::fs::write(&path, make(1)).unwrap();
    cx.dispatch_action(Reload);
    cx.run_until_parked();

    assert_eq!(scroll_top(&v, cx), (0, 0.));
    let (bounds, viewport) = cx.update(|_, cx| {
        let list = v.read(cx).active_list().unwrap();
        (list.bounds_for_item(0).unwrap(), list.viewport_bounds())
    });
    assert!(
        bounds.bottom() > viewport.top() && bounds.top() >= viewport.top(),
        "shrunk code block must stay visible in viewport: bounds={bounds:?}, viewport={viewport:?}"
    );
}

#[test]
fn tall_window_page_up_returns_to_top_and_protects_after_jump() {
    // 1. Long paragraph document in a 1500px-tall window: PageDown (1420px)
    // followed by PageUp must return all the way to the top (0, 0.).
    let mut md = String::from("# Start\n\n");
    for i in 0..200 {
        md.push_str(&format!(
            "## Heading {i}\n\nParagraph {i} with enough text for a tall document.\n\n"
        ));
    }
    let (v, cx) = open(vec![(PathBuf::from("/x/tall.md"), parsed(&md))]);
    let handle = cx.window_handle();
    cx.simulate_window_resize(handle, size(px(920.), px(1500.)));
    cx.run_until_parked();

    assert_eq!(scroll_top(&v, cx), (0, 0.));
    cx.dispatch_action(PageDown);
    cx.run_until_parked();
    let down = scroll_top(&v, cx);
    assert!(
        down.0 > 10,
        "expected PageDown to advance well down: {down:?}"
    );
    cx.dispatch_action(PageUp);
    cx.run_until_parked();
    assert_eq!(
        scroll_top(&v, cx),
        (0, 0.),
        "PageUp after normal PageDown in a 1500px window must return to the top"
    );

    // 2. After jumping to Bottom or an anchor in the same 1500px window, PageUp
    // stays within measured leading overdraw instead of teleporting over
    // unmeasured rows back toward row 0.
    cx.dispatch_action(Bottom);
    cx.run_until_parked();
    let bottom = scroll_top(&v, cx);
    assert!(bottom.0 > 350, "{bottom:?}");
    cx.dispatch_action(PageUp);
    cx.run_until_parked();
    let after_bottom_page_up = scroll_top(&v, cx);
    assert!(
        after_bottom_page_up.0 > 300 && after_bottom_page_up.0 < bottom.0,
        "PageUp after Bottom must not teleport over unmeasured rows: {after_bottom_page_up:?} from {bottom:?}"
    );

    // Anchor jump to a deep heading -> PageUp also stays local.
    cx.dispatch_action(Top);
    cx.run_until_parked();
    v.update(cx, |v, cx| v.scroll_to_anchor(None, "heading-150", cx));
    cx.run_until_parked();
    let anchor = scroll_top(&v, cx);
    assert_eq!(anchor.0, 301);
    cx.dispatch_action(PageUp);
    cx.run_until_parked();
    let after_anchor_page_up = scroll_top(&v, cx);
    assert!(
        after_anchor_page_up.0 > 250 && after_anchor_page_up.0 < anchor.0,
        "PageUp after anchor jump must not teleport over unmeasured rows: {after_anchor_page_up:?}"
    );

    // Returning to Top restores full-page PageUp symmetry.
    cx.dispatch_action(Top);
    cx.run_until_parked();
    cx.dispatch_action(PageDown);
    cx.run_until_parked();
    cx.dispatch_action(PageUp);
    cx.run_until_parked();
    assert_eq!(scroll_top(&v, cx), (0, 0.));

    // 3. After a width resize while scrolled down, GPUI invalidates cached
    // heights; PageUp must clamp so it doesn't teleport over invalidated rows.
    for _ in 0..4 {
        cx.dispatch_action(PageDown);
        cx.run_until_parked();
    }
    let before_resize = scroll_top(&v, cx);
    assert!(before_resize.0 > 50, "{before_resize:?}");
    cx.simulate_window_resize(handle, size(px(800.), px(1500.)));
    cx.run_until_parked();
    cx.dispatch_action(PageUp);
    cx.run_until_parked();
    let after_resize_page_up = scroll_top(&v, cx);
    assert!(
        after_resize_page_up.0 > 20 && after_resize_page_up.0 < before_resize.0,
        "PageUp after width resize must not teleport to top: {after_resize_page_up:?} from {before_resize:?}"
    );
}
