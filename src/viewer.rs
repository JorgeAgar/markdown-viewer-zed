//! The single window: a tab bar plus the rendered document of the active tab.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use gpui::{
    App, Context, ExternalPaths, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, ListAlignment, ListOffset, ListState, MouseButton, ParentElement, PathPromptOptions,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, actions, div, list,
    prelude::FluentBuilder, px,
};

use crate::{
    markdown::{self, Block, Document},
    render::{self, RenderCx},
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
    let res = std::fs::read(&path)
        .map(|bytes| {
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            (markdown::parse(&String::from_utf8_lossy(&bytes)), mtime)
        })
        .map_err(|e| e.to_string());
    (path, res)
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
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
            .into();
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
    tabs: Vec<Tab>,
    active: usize,
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
    pub fn new(loaded: Vec<Loaded>, fonts: Fonts, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            tabs: Vec::new(),
            active: 0,
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
            if let Some(ix) = self.tabs.iter().position(|t| t.path == path) {
                self.active = ix;
                continue;
            }
            self.tabs.push(Tab::new(path, res));
            self.active = self.tabs.len() - 1;
        }
    }

    pub fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut loaded = Vec::new();
        for p in paths {
            let p = normalize(&p);
            if p.is_dir() {
                continue;
            }
            if let Some(ix) = self.tabs.iter().position(|t| t.path == p) {
                self.active = ix;
            } else {
                loaded.push(load(p));
            }
        }
        self.add_loaded(loaded);
        cx.notify();
    }

    pub fn scroll_to_anchor(&mut self, path: Option<&Path>, anchor: &str, cx: &mut Context<Self>) {
        let tab = match path {
            Some(p) => self.tabs.iter().find(|t| t.path == normalize(p)),
            None => self.tabs.get(self.active),
        };
        let Some(tab) = tab else { return };
        let anchor = anchor.to_lowercase();
        if let Some(ix) = tab.doc.blocks.iter().position(
            |b| matches!(b, Block::Heading { slug, .. } if *slug == anchor),
        ) {
            tab.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
    }

    fn poll_changes(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for tab in &mut self.tabs {
            let mtime = std::fs::metadata(&tab.path).and_then(|m| m.modified()).ok();
            if mtime.is_some() && mtime != tab.mtime {
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
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        if self.active > ix || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        cx.notify();
    }

    fn active_list(&self) -> Option<&ListState> {
        self.tabs.get(self.active).map(|t| &t.list)
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
                let active = ix == self.active;
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
                        this.active = ix;
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

        let title: SharedString = match self.tabs.get(self.active) {
            Some(t) => format!("{} — mdview", t.title).into(),
            None => "mdview".into(),
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }

        let content = match self.tabs.get(self.active) {
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
                list(tab.list.clone(), move |ix, _, _| {
                    render::top_block(&doc.blocks[ix], ix, &rcx)
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
                    this.close_tab(this.active, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                if !this.tabs.is_empty() {
                    this.active = (this.active + 1) % this.tabs.len();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                if !this.tabs.is_empty() {
                    this.active = (this.active + this.tabs.len() - 1) % this.tabs.len();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Reload, _, cx| {
                if let Some(tab) = this.tabs.get_mut(this.active) {
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
