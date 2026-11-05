//! The single window: a tab bar plus the rendered document of the active tab.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use gpui::{
    App, Context, ExternalPaths, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, ListAlignment, ListOffset, ListState, MouseButton, ParentElement,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled, Window, actions,
    div, list, prelude::FluentBuilder, px,
};

use crate::{
    links,
    markdown::{self, Document},
    render::{self, RenderCx},
    tabs::Tabs,
    theme::Theme,
};

actions!(
    mdview,
    [
        Open, CloseTab, NextTab, PrevTab, Reload, Quit, ScrollUp, ScrollDown, PageUp, PageDown,
        Top, Bottom
    ]
);

pub type Loaded = (PathBuf, Result<(Document, Option<SystemTime>), String>);

/// Read + parse a file. Safe to call from any thread.
pub fn load(path: PathBuf) -> Loaded {
    // Stat before reading: if the file changes in between, the content is
    // newer than the mtime and the next live-reload poll reads it again.
    // The other order would pair the new mtime with the old content.
    let mtime = modified(&path);
    let res = std::fs::read(&path)
        .map(|bytes| (markdown::parse(&String::from_utf8_lossy(&bytes)), mtime))
        .map_err(|e| e.to_string());
    (path, res)
}

/// The file's modification time, if it can be read.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Live reload: whether a file whose modification time is now `now` must be
/// read again. A file that disappeared (`None`) keeps its last content.
fn needs_reload(loaded: Option<SystemTime>, now: Option<SystemTime>) -> bool {
    now.is_some() && now != loaded
}

pub fn normalize(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        }
    })
}

struct Tab {
    path: PathBuf,
    title: SharedString,
    doc: Arc<Document>,
    mtime: Option<SystemTime>,
    list: ListState,
}

impl Tab {
    fn new(path: PathBuf, res: Result<(Document, Option<SystemTime>), String>) -> Self {
        let title = tab_title(&path).into();
        let (doc, mtime) = res.unwrap_or_else(|e| (error_doc(&path, &e), None));
        let list = ListState::new(doc.blocks.len(), ListAlignment::Top, px(800.));
        Self {
            path,
            title,
            doc: Arc::new(doc),
            mtime,
            list,
        }
    }

    fn replace(&mut self, res: Result<(Document, Option<SystemTime>), String>) {
        let (doc, mtime) = res.unwrap_or_else(|e| (error_doc(&self.path, &e), None));
        let top = self.list.logical_scroll_top();
        self.list.reset(doc.blocks.len());
        self.list.scroll_to(ListOffset {
            item_ix: top.item_ix.min(doc.blocks.len().saturating_sub(1)),
            offset_in_item: top.offset_in_item,
        });
        self.doc = Arc::new(doc);
        self.mtime = mtime;
    }
}

/// The file name, or the whole path when there's none (`/`, `..`).
fn tab_title(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn error_doc(path: &Path, err: &str) -> Document {
    markdown::parse(&format!(
        "# Could not open file\n\n`{}`\n\n> {}\n",
        path.display(),
        err
    ))
}

pub struct Fonts {
    pub ui: SharedString,
    pub mono: SharedString,
}

pub struct Viewer {
    tabs: Tabs<Tab>,
    focus: FocusHandle,
    fonts: Fonts,
    window_title: SharedString,
    traced_startup: bool,
}

impl Focusable for Viewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Viewer {
    pub fn new(
        loaded: Vec<Loaded>,
        fonts: Fonts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();

        // Cheap live-reload: stat open files periodically.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(750))
                    .await;
                if this.update(cx, |this, cx| this.poll_changes(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut this = Self {
            tabs: Tabs::default(),
            focus,
            fonts,
            window_title: SharedString::default(),
            traced_startup: false,
        };
        this.add_loaded(loaded);
        this
    }

    fn add_loaded(&mut self, loaded: Vec<Loaded>) {
        for (path, res) in loaded {
            if !self.tabs.activate_first(|t| t.path == path) {
                self.tabs.push(Tab::new(path, res));
            }
        }
    }

    pub fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut loaded = Vec::new();
        for p in paths {
            let p = normalize(&p);
            if p.is_dir() {
                continue;
            }
            if !self.tabs.activate_first(|t| t.path == p) {
                loaded.push(load(p));
            }
        }
        self.add_loaded(loaded);
        cx.notify();
    }

    pub fn scroll_to_anchor(&mut self, path: Option<&Path>, anchor: &str, cx: &mut Context<Self>) {
        let tab = match path {
            Some(p) => self.tabs.iter().find(|t| t.path == normalize(p)),
            None => self.tabs.active(),
        };
        let Some(tab) = tab else { return };
        if let Some(ix) = links::heading_index(&tab.doc.blocks, anchor) {
            tab.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
    }

    fn poll_changes(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for tab in self.tabs.iter_mut() {
            if needs_reload(tab.mtime, modified(&tab.path)) {
                let (_, res) = load(tab.path.clone());
                tab.replace(res);
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.tabs.close(ix).is_some() {
            cx.notify();
        }
    }

    fn active_list(&self) -> Option<&ListState> {
        self.tabs.active().map(|t| &t.list)
    }

    fn scroll(&mut self, dy: f32, cx: &mut Context<Self>) {
        if let Some(l) = self.active_list() {
            l.scroll_by(px(dy));
            cx.notify();
        }
    }

    fn page(&self, window: &Window) -> f32 {
        (f32::from(window.viewport_size().height) - 80.).max(100.)
    }

    fn render_tab_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tab-bar")
            .flex_none()
            .h(px(34.))
            .w_full()
            .flex()
            .flex_row()
            .overflow_x_scroll()
            .bg(theme.bar_bg)
            .border_b_1()
            .border_color(theme.border)
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let active = ix == self.tabs.active_index();
                div()
                    .id(("tab", ix))
                    .group("tab")
                    .flex_none()
                    .h_full()
                    .pl_3()
                    .pr_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_r_1()
                    .border_color(theme.border)
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |d| d.bg(theme.bg).text_color(theme.fg))
                    .when(!active, |d| {
                        d.text_color(theme.muted).hover(|s| s.bg(theme.hover))
                    })
                    .child(tab.title.clone())
                    .child(
                        div()
                            .id(("close", ix))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .text_color(theme.muted)
                            .hover(|s| s.bg(theme.hover).text_color(theme.fg))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(ix, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tabs.activate(ix);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| this.close_tab(ix, cx)),
                    )
            }))
    }

    fn render_empty(&self, theme: &Theme) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(theme.muted)
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("mdview"),
            )
            .child("Drop a Markdown file here, or press Ctrl+O")
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(window.appearance());

        if !self.traced_startup {
            self.traced_startup = true;
            if std::env::var_os("MDVIEW_TRACE_STARTUP").is_some() {
                let start = *crate::START.get().unwrap();
                window.on_next_frame(move |_, cx| {
                    eprintln!("first frame: {:?}", start.elapsed());
                    if std::env::var_os("MDVIEW_TRACE_STARTUP").is_some_and(|v| v == "quit") {
                        cx.quit();
                    }
                });
            }
        }

        let title: SharedString = match self.tabs.active() {
            Some(t) => format!("{} — mdview", t.title).into(),
            None => "mdview".into(),
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }

        let content = match self.tabs.active() {
            Some(tab) => {
                let rcx = RenderCx {
                    theme,
                    ui_font: self.fonts.ui.clone(),
                    mono_font: self.fonts.mono.clone(),
                    base_dir: tab
                        .path
                        .parent()
                        .map(Arc::from)
                        .unwrap_or_else(|| Arc::from(Path::new("/"))),
                    viewer: cx.weak_entity(),
                };
                let doc = tab.doc.clone();
                list(tab.list.clone(), move |ix, window, cx| {
                    render::top_block(&doc.blocks[ix], ix, &rcx, window, cx)
                })
                .size_full()
                .into_any_element()
            }
            None => self.render_empty(&theme).into_any_element(),
        };

        div()
            .id("viewer")
            .key_context("Viewer")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.fg)
            .font_family(self.fonts.ui.clone())
            .text_size(px(render::BASE_SIZE))
            .line_height(gpui::relative(1.6))
            .on_action(cx.listener(|_, _: &Open, _, cx| {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: Some("Open Markdown".into()),
                });
                cx.spawn(async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = rx.await {
                        this.update(cx, |this, cx| this.open_paths(paths, cx)).ok();
                    }
                })
                .detach();
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.tabs.is_empty() {
                    window.remove_window();
                } else {
                    this.close_tab(this.tabs.active_index(), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                if !this.tabs.is_empty() {
                    this.tabs.next();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                if !this.tabs.is_empty() {
                    this.tabs.prev();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Reload, _, cx| {
                if let Some(tab) = this.tabs.active_mut() {
                    let (_, res) = load(tab.path.clone());
                    tab.replace(res);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ScrollUp, _, cx| this.scroll(-60., cx)))
            .on_action(cx.listener(|this, _: &ScrollDown, _, cx| this.scroll(60., cx)))
            .on_action(cx.listener(|this, _: &PageUp, window, cx| {
                let p = this.page(window);
                this.scroll(-p, cx)
            }))
            .on_action(cx.listener(|this, _: &PageDown, window, cx| {
                let p = this.page(window);
                this.scroll(p, cx)
            }))
            .on_action(cx.listener(|this, _: &Top, _, cx| {
                if let Some(l) = this.active_list() {
                    l.scroll_to(ListOffset::default());
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Bottom, _, cx| {
                if let Some(l) = this.active_list() {
                    let n = l.item_count();
                    if n > 0 {
                        l.scroll_to_reveal_item(n - 1);
                        cx.notify();
                    }
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.open_paths(paths.paths().to_vec(), cx);
            }))
            .when(!self.tabs.is_empty(), |d| {
                d.child(self.render_tab_bar(&theme, cx))
            })
            .child(div().flex_1().min_h_0().w_full().child(content))
    }
}

#[cfg(test)]
mod app_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{markdown::Block, test_util::TempDir};
    #[cfg(unix)]
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    fn heading(doc: &Document, ix: usize) -> String {
        match &doc.blocks[ix] {
            Block::Heading { text, .. } => text.text.clone(),
            b => panic!("block {ix} is not a heading: {b:?}"),
        }
    }

    fn all_text(doc: &Document) -> String {
        format!("{:?}", doc.blocks)
    }

    #[test]
    fn load_reads_and_parses() {
        let dir = TempDir::new("load");
        let path = dir.join("a.md");
        std::fs::write(&path, "# Title\n\nBody\n").unwrap();
        let (p, res) = load(path.clone());
        assert_eq!(p, path);
        let (doc, mtime) = res.unwrap();
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(heading(&doc, 0), "Title");
        assert_eq!(mtime, modified(&path));
        assert!(mtime.is_some());
    }

    /// The mtime must be taken *before* reading. Otherwise a save that lands
    /// between the read and the stat is recorded with the old content, and
    /// live reload never picks it up. A FIFO makes that race deterministic:
    /// reading blocks until a writer comes, and the write bumps the mtime.
    #[cfg(unix)]
    #[test]
    fn load_takes_the_mtime_before_reading() {
        let dir = TempDir::new("fifo");
        let fifo = dir.join("pipe.md");
        let ok = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "mkfifo failed");
        let before = modified(&fifo).unwrap();
        let writer = {
            let fifo = fifo.clone();
            std::thread::spawn(move || {
                // Coarse timestamps: make sure the write gets a later one.
                std::thread::sleep(Duration::from_millis(50));
                std::fs::write(&fifo, "# Saved\n").unwrap();
            })
        };
        let (doc, mtime) = load(fifo.clone()).1.unwrap();
        writer.join().unwrap();
        assert_eq!(heading(&doc, 0), "Saved");
        assert!(
            modified(&fifo).unwrap() > before,
            "the write bumps the mtime"
        );
        assert_eq!(
            mtime,
            Some(before),
            "mtime read after the content: the next live-reload poll would miss the change"
        );
    }

    #[test]
    fn load_missing_file() {
        let dir = TempDir::new("missing");
        let (p, res) = load(dir.join("nope.md"));
        assert_eq!(p, dir.join("nope.md"));
        let err = res.err().unwrap();
        #[cfg(unix)]
        assert!(err.contains("No such file"), "{err}");
        #[cfg(windows)]
        assert!(err.contains("cannot find"), "{err}");
    }

    #[test]
    fn load_directory() {
        let dir = TempDir::new("dir");
        let err = load(dir.to_path_buf()).1.err().unwrap();
        #[cfg(unix)]
        assert!(err.contains("directory"), "{err}");
        #[cfg(windows)]
        assert!(err.contains("denied"), "{err}");
    }

    #[test]
    fn load_non_utf8_is_lossy() {
        let dir = TempDir::new("lossy");
        let path = dir.join("bad.md");
        std::fs::write(&path, b"# caf\xe9 \xff\n\nok\n").unwrap();
        let doc = load(path).1.unwrap().0;
        assert_eq!(heading(&doc, 0), "caf\u{FFFD} \u{FFFD}");
    }

    #[test]
    fn load_empty_file() {
        let dir = TempDir::new("empty");
        let path = dir.join("e.md");
        std::fs::write(&path, "").unwrap();
        let (doc, mtime) = load(path).1.unwrap();
        assert!(doc.blocks.is_empty());
        assert!(mtime.is_some());
    }

    #[test]
    fn normalize_paths() {
        let dir = TempDir::new("normalize");
        let file = dir.join("a.md");
        std::fs::write(&file, "x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        let canon = std::fs::canonicalize(&file).unwrap();
        // Existing files are canonicalized, so different spellings dedupe.
        assert_eq!(normalize(&file), canon);
        assert_eq!(normalize(&dir.join("sub/../a.md")), canon);
        assert_eq!(normalize(&dir.join("./a.md")), canon);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&file, dir.join("link.md")).unwrap();
            assert_eq!(normalize(&dir.join("link.md")), canon);
        }
        // Missing absolute paths are kept as they are (not cleaned up).
        let missing = dir.join("sub/../missing.md");
        assert_eq!(normalize(&missing), missing);
        // Missing relative paths are made absolute against the cwd.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            normalize(Path::new("no-such-dir-mdview/x.md")),
            cwd.join("no-such-dir-mdview/x.md")
        );
        // An existing relative path is canonicalized too.
        assert_eq!(
            normalize(Path::new("Cargo.toml")),
            std::fs::canonicalize(cwd.join("Cargo.toml")).unwrap()
        );
        // Non-UTF-8 names survive.
        #[cfg(unix)]
        {
            let raw = dir.join(OsStr::from_bytes(b"caf\xe9.md"));
            assert_eq!(normalize(&raw), raw);
        }
    }

    #[test]
    fn reload_detection() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let t1 = t0 + Duration::from_nanos(if cfg!(windows) { 100 } else { 1 });
        assert!(!needs_reload(Some(t0), Some(t0)), "unchanged");
        assert!(needs_reload(Some(t0), Some(t1)), "modified");
        assert!(needs_reload(Some(t1), Some(t0)), "older (restored backup)");
        assert!(!needs_reload(Some(t0), None), "deleted: keep the content");
        assert!(needs_reload(None, Some(t0)), "appeared after a failed load");
        assert!(!needs_reload(None, None), "still missing");
    }

    #[test]
    fn titles() {
        assert_eq!(tab_title(Path::new("/a/b/readme.md")), "readme.md");
        assert_eq!(tab_title(Path::new("/a/b/")), "b");
        assert_eq!(tab_title(Path::new("/")), "/");
        assert_eq!(tab_title(Path::new("/a/..")), "/a/..");
        #[cfg(unix)]
        assert_eq!(
            tab_title(Path::new(OsStr::from_bytes(b"/x/caf\xe9.md"))),
            "caf\u{FFFD}.md"
        );
    }

    #[test]
    fn error_document() {
        let doc = error_doc(
            Path::new("/x/a b.md"),
            "No such file or directory (os error 2)",
        );
        assert_eq!(heading(&doc, 0), "Could not open file");
        let text = all_text(&doc);
        assert!(text.contains("/x/a b.md"), "{text}");
        assert!(text.contains("No such file or directory"), "{text}");
        assert!(matches!(doc.blocks[2], Block::Quote { .. }));
        // Markdown in the path or the error isn't a problem for the parse.
        let doc = error_doc(Path::new("/x/*a* [b](c).md"), "# not a heading");
        assert_eq!(doc.blocks.len(), 3);
    }

    #[test]
    fn tab_from_error_shows_the_error_document() {
        let tab = Tab::new(PathBuf::from("/x/gone.md"), Err("boom".into()));
        assert_eq!(tab.title.as_ref(), "gone.md");
        assert_eq!(tab.mtime, None);
        assert_eq!(heading(&tab.doc, 0), "Could not open file");
        assert_eq!(tab.list.item_count(), tab.doc.blocks.len());
    }

    fn doc_with(n: usize) -> Document {
        let src: String = (0..n).map(|i| format!("Paragraph {i}\n\n")).collect();
        let doc = markdown::parse(&src);
        assert_eq!(doc.blocks.len(), n);
        doc
    }

    #[test]
    fn replace_keeps_the_scroll_position() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
        let mut tab = Tab::new(PathBuf::from("/x/a.md"), Ok((doc_with(10), Some(t0))));
        assert_eq!(tab.list.item_count(), 10);
        tab.list.scroll_to(ListOffset {
            item_ix: 4,
            offset_in_item: px(12.),
        });
        let t1 = t0 + Duration::from_secs(1);
        tab.replace(Ok((doc_with(12), Some(t1))));
        assert_eq!(tab.list.item_count(), 12);
        assert_eq!(tab.doc.blocks.len(), 12);
        assert_eq!(tab.mtime, Some(t1));
        let top = tab.list.logical_scroll_top();
        assert_eq!((top.item_ix, top.offset_in_item), (4, px(12.)));
    }

    #[test]
    fn replace_with_a_shorter_document_clamps_the_scroll_position() {
        let mut tab = Tab::new(PathBuf::from("/x/a.md"), Ok((doc_with(10), None)));
        tab.list.scroll_to(ListOffset {
            item_ix: 8,
            offset_in_item: px(5.),
        });
        tab.replace(Ok((doc_with(3), None)));
        let top = tab.list.logical_scroll_top();
        assert_eq!(top.item_ix, 2, "last row of the new document");
        assert_eq!(tab.list.item_count(), 3);
        // Down to nothing.
        tab.replace(Ok((doc_with(0), None)));
        assert_eq!(tab.list.item_count(), 0);
        assert_eq!(tab.list.logical_scroll_top().item_ix, 0);
    }

    #[test]
    fn replace_with_an_error() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
        let mut tab = Tab::new(PathBuf::from("/x/a.md"), Ok((doc_with(2), Some(t0))));
        tab.replace(Err("gone".into()));
        assert_eq!(tab.mtime, None);
        assert_eq!(heading(&tab.doc, 0), "Could not open file");
        assert!(all_text(&tab.doc).contains("gone"));
        assert_eq!(tab.title.as_ref(), "a.md", "the title doesn't change");
    }
}
