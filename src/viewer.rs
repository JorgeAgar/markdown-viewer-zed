//! The single window: a tab bar plus the rendered document of the active tab.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use gpui::{
    AnyElement, App, Context, DispatchPhase, ExternalPaths, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ListAlignment, ListOffset, ListState, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathPromptOptions, Pixels, Render,
    ScrollDelta, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, TouchPhase,
    Window, actions, canvas, div, list, prelude::FluentBuilder, px,
};

use crate::{
    links,
    markdown::{self, Document},
    render::{self, CodeScrollMap, RenderCx},
    rows::Rows,
    scroll::{self, Measured, Motion, Thumb},
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

/// Pixels measured above and below the viewport by `gpui::ListState`.
const LIST_OVERDRAW: f32 = 800.;

/// Largest PageUp step that stays inside the rows the last frame measured
/// above the scroll top (`LIST_OVERDRAW`, minus a margin).
const MEASURED_STEP: f32 = LIST_OVERDRAW - 40.;

/// Height of the tab bar, above the document.
const TAB_BAR_HEIGHT: f32 = 34.;

/// Width of the scrollbar's hit area at the right edge of the document.
const SCROLLBAR_WIDTH: f32 = 14.;

/// The first `end` rows of `list`, as far as they're measured.
fn measured(list: &ListState, end: usize) -> Measured {
    let m = list.measured_before(end);
    Measured {
        count: m.count,
        measured: m.measured_count,
        height: f32::from(m.measured_height),
    }
}

/// A scrollbar thumb being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ThumbDrag {
    /// `Tab::id` of the tab being scrolled.
    tab: u64,
    /// Window y of the track's top, and its length.
    origin: f32,
    track: f32,
    /// Mouse y minus the thumb's top when the drag started.
    grab: f32,
    /// The thumb now, relative to the track.
    thumb: Thumb,
    /// Row height estimate and document height, frozen for the drag so the
    /// thumb stays under the mouse while rows get measured.
    avg: f32,
    content: f32,
}

/// Source of unique `Tab::id` and `Tab::doc_id` values.
static NEXT_TAB_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_id() -> u64 {
    NEXT_TAB_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

struct Tab {
    /// Unique among tabs, so a scrollbar drag knows if its tab is still active.
    id: u64,
    /// Fresh on every tab creation and document reload, scoping per-row GPUI
    /// element state (e.g. animated `img` frame indices) so another tab or
    /// replaced image never inherits an out-of-bounds frame index.
    doc_id: u64,
    path: PathBuf,
    base_dir: Arc<Path>,
    title: SharedString,
    doc: Arc<Document>,
    rows: Arc<Rows>,
    code_scroll: CodeScrollMap,
    mtime: Option<SystemTime>,
    list: ListState,
    /// True while every row before `list.logical_scroll_top().item_ix` is
    /// guaranteed measured in `list` (no jump, reset, or width resize skipped
    /// over unmeasured rows above).
    leading_measured: bool,
    last_viewport_width: Option<Pixels>,
    /// Rest of a PageUp that `Viewer::page_up` split into `MEASURED_STEP`s
    /// because rows above might be unmeasured.
    page_up_rest: Option<PageUpRest>,
    /// Smooth wheel scrolling and touchpad flings.
    motion: Motion,
}

/// The part of a PageUp still to scroll, one `MEASURED_STEP` per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PageUpRest {
    px: f32,
    /// True once a frame has laid out the list since the last step, so the
    /// rows within `LIST_OVERDRAW` above the scroll top are measured.
    laid_out: bool,
}

impl PageUpRest {
    /// A PageUp of `page` pixels when rows above may be unmeasured, on top of
    /// any `pending` rest. Returns the pixels to scroll now and the rest.
    fn start(pending: Option<PageUpRest>, page: f32) -> (f32, Option<PageUpRest>) {
        let total = page + pending.map_or(0., |r| r.px);
        let step = total.min(MEASURED_STEP);
        let rest = (total > step).then_some(PageUpRest {
            px: total - step,
            laid_out: false,
        });
        (step, rest)
    }

    /// One frame of a split PageUp, before the list is laid out. Returns the
    /// pixels to scroll now and the rest. Once the rows above are known to
    /// be measured (`measured_above`), the whole rest goes in one step.
    fn frame(self, measured_above: bool) -> (f32, Option<PageUpRest>) {
        let step = match (self.laid_out, measured_above) {
            (false, _) => 0.,
            (true, true) => self.px,
            (true, false) => self.px.min(MEASURED_STEP),
        };
        // This frame lays out the list after `step` (or after the step
        // `start` returned), so the next one can go ahead.
        let rest = (self.px > step).then_some(PageUpRest {
            px: self.px - step,
            laid_out: true,
        });
        (step, rest)
    }
}

impl Tab {
    fn new(path: PathBuf, res: Result<(Document, Option<SystemTime>), String>) -> Self {
        let title = tab_title(&path).into();
        let base_dir: Arc<Path> = path
            .parent()
            .map(Arc::from)
            .unwrap_or_else(|| Arc::from(Path::new("/")));
        let (doc, mtime) = res.unwrap_or_else(|e| (error_doc(&path, &e), None));
        let rows = Arc::new(Rows::from_document(&doc));
        let list = ListState::new(rows.len(), ListAlignment::Top, px(LIST_OVERDRAW));
        Self {
            id: next_id(),
            doc_id: next_id(),
            path,
            base_dir,
            title,
            doc: Arc::new(doc),
            rows,
            code_scroll: CodeScrollMap::default(),
            mtime,
            list,
            leading_measured: true,
            last_viewport_width: None,
            page_up_rest: None,
            motion: Motion::default(),
        }
    }

    /// The estimated height above the viewport and of the whole document,
    /// and the row height estimate behind them: unmeasured rows count as
    /// the average measured one.
    fn extent(&self) -> (f32, f32, f32) {
        let all = measured(&self.list, usize::MAX);
        let avg = all.average();
        let top = self.list.logical_scroll_top();
        let above = measured(&self.list, top.item_ix).estimate(avg) + f32::from(top.offset_in_item);
        (above, all.estimate(avg), avg)
    }

    /// Scrolls `dy` px but stays within the measured rows, like GPUI's own
    /// wheel handler: unmeasured rows count as 0 px, so going further could
    /// skip them. Does nothing right after a reload or reset, before any row
    /// has a height.
    fn scroll_measured(&mut self, dy: f32) {
        if self.unmeasured() {
            return;
        }
        let top = self.list.logical_scroll_top();
        let now = measured(&self.list, top.item_ix).height + f32::from(top.offset_in_item);
        let viewport = f32::from(self.list.viewport_bounds().size.height);
        let max = measured(&self.list, usize::MAX).height - viewport;
        // Never pulled back: `Bottom` parks the top past the last row.
        let target = (now + dy).min(max.max(now)).max(0.);
        self.list.scroll_by(px(target - now));
    }

    /// Limits the running glide to the room left to the document's edges,
    /// before a new wheel step adds to it.
    fn limit_motion(&mut self) {
        let (above, content, _) = self.extent();
        let viewport = f32::from(self.list.viewport_bounds().size.height);
        self.motion
            .limit(-above, (content - above - viewport).max(0.));
    }

    /// True when the document has rows but none is measured yet.
    fn unmeasured(&self) -> bool {
        let all = measured(&self.list, usize::MAX);
        all.count > 0 && all.measured == 0
    }

    /// Called at the start of each frame: scrolls this frame's part of a
    /// wheel glide or fling. Returns true while it goes on.
    fn advance_motion(&mut self, now: Instant, viewport: f32) -> bool {
        if !self.motion.is_active() {
            return false;
        }
        if self.unmeasured() {
            // Just reset (resized, reloaded): wait for the rows' heights.
            return true;
        }
        let dir = self.motion.direction();
        let top = self.list.logical_scroll_top();
        let at_top = top.item_ix == 0 && top.offset_in_item <= px(0.);
        let (above, content, _) = self.extent();
        // Not 0.5 px: a wheel step must still cover a fraction a touchpad
        // left. Float error is far below 0.01 px; a stray miss only costs
        // the rest of the glide.
        let at_bottom = above + viewport >= content - 0.01;
        if (dir < 0. && at_top) || (dir > 0. && at_bottom) {
            self.motion.stop();
            return false;
        }
        // Only cross rows the last frame measured (its overdraw reaches
        // `LIST_OVERDRAW` past both edges): unmeasured rows count as 0 px,
        // so a longer step could skip them, like a split PageUp.
        let step = self.motion.tick(now, MEASURED_STEP);
        self.list.scroll_by(px(step));
        self.motion.is_active()
    }

    /// Scrolls to the row and offset at estimated position `pos`.
    fn scroll_to_estimate(&mut self, pos: f32, avg: f32) {
        let (item_ix, offset) = scroll::locate(pos, self.list.item_count(), |ix| {
            measured(&self.list, ix).estimate(avg)
        });
        self.list.scroll_to(ListOffset {
            item_ix,
            offset_in_item: px(offset),
        });
        self.leading_measured = item_ix == 0;
        self.page_up_rest = None;
    }

    fn leading_measured(&self) -> bool {
        self.leading_measured || self.list.logical_scroll_top().item_ix == 0
    }

    /// Scrolls up `page` pixels. When rows above may be unmeasured, scrolls
    /// one `MEASURED_STEP` now and leaves the rest to `continue_page_up`, so
    /// each step only crosses rows the previous frame measured.
    fn page_up(&mut self, page: f32) {
        let (step, rest) = if self.leading_measured() {
            (page, None)
        } else {
            PageUpRest::start(self.page_up_rest, page)
        };
        self.page_up_rest = rest;
        self.motion.stop();
        self.list.scroll_by(px(-step));
    }

    /// Called at the start of each frame. Scrolls the next step of a split
    /// PageUp once the previous step has been laid out. Returns true while
    /// steps remain, so the caller asks for another frame.
    fn continue_page_up(&mut self) -> bool {
        let Some(rest) = self.page_up_rest else {
            return false;
        };
        let (step, rest) = rest.frame(self.leading_measured());
        self.page_up_rest = rest;
        self.list.scroll_by(px(-step));
        rest.is_some()
    }

    fn note_viewport_width(&mut self, width: Pixels) {
        if self
            .last_viewport_width
            .replace(width)
            .is_some_and(|prev| prev != width)
            && self.list.logical_scroll_top().item_ix > 0
        {
            // GPUI's List clears all cached item heights when width changes.
            self.leading_measured = false;
        }
    }

    fn replace(&mut self, res: Result<(Document, Option<SystemTime>), String>) {
        let (doc, mtime) = res.unwrap_or_else(|e| (error_doc(&self.path, &e), None));
        let rows = Arc::new(Rows::from_document(&doc));
        let top = self.list.logical_scroll_top();
        let mapped = rows.remap_row(&self.rows, top.item_ix);
        self.list.reset(rows.len());
        self.list.scroll_to(ListOffset {
            item_ix: mapped.item_ix,
            offset_in_item: if mapped.reset_offset {
                px(0.)
            } else {
                top.offset_in_item
            },
        });
        self.leading_measured = mapped.item_ix == 0;
        self.page_up_rest = None;
        // A glide would move before the new rows are laid out (all 0 px).
        self.motion.stop();
        self.code_scroll.borrow_mut().clear();
        self.doc_id = next_id();
        self.doc = Arc::new(doc);
        self.rows = rows;
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

/// Parent directory of `path`, skipping empty parents (e.g. a bare file name).
fn parent_dir(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    (!dir.as_os_str().is_empty()).then(|| dir.to_path_buf())
}

/// Pick the starting directory for the open-file dialog:
/// active tab's parent → last opened directory in this session → process cwd.
fn resolve_open_directory(
    active_file: Option<&Path>,
    last_open_dir: Option<&Path>,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let active_dir = active_file.and_then(parent_dir);
    [active_dir.as_deref(), last_open_dir, cwd]
        .into_iter()
        .flatten()
        .find(|dir| dir.is_dir())
        .map(Path::to_path_buf)
}

pub struct Viewer {
    tabs: Tabs<Tab>,
    focus: FocusHandle,
    fonts: Fonts,
    window_title: SharedString,
    last_open_dir: Option<PathBuf>,
    traced_startup: bool,
    thumb_drag: Option<ThumbDrag>,
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
            last_open_dir: None,
            traced_startup: false,
            thumb_drag: None,
        };
        this.add_loaded(loaded);
        this
    }

    fn record_dir(&mut self, path: &Path) {
        if let Some(dir) = parent_dir(path).filter(|d| d.is_dir()) {
            self.last_open_dir = Some(dir);
        }
    }

    fn open_dialog_directory(&self) -> Option<PathBuf> {
        resolve_open_directory(
            self.tabs.active().map(|t| t.path.as_path()),
            self.last_open_dir.as_deref(),
            std::env::current_dir().ok().as_deref(),
        )
    }

    fn add_loaded(&mut self, loaded: Vec<Loaded>) {
        for (path, res) in loaded {
            self.record_dir(&path);
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
            if self.tabs.activate_first(|t| t.path == p) {
                self.record_dir(&p);
            } else {
                loaded.push(load(p));
            }
        }
        self.add_loaded(loaded);
        cx.notify();
    }

    pub fn scroll_to_anchor(&mut self, path: Option<&Path>, anchor: &str, cx: &mut Context<Self>) {
        let tab = match path {
            Some(p) => {
                let target = normalize(p);
                self.tabs.iter_mut().find(|t| t.path == target)
            }
            None => self.tabs.active_mut(),
        };
        let Some(tab) = tab else { return };
        if let Some(block_ix) = links::heading_index(&tab.doc.blocks, anchor)
            && let Some(row_ix) = tab.rows.row_for_block(block_ix)
        {
            tab.list.scroll_to(ListOffset {
                item_ix: row_ix,
                offset_in_item: px(0.),
            });
            tab.leading_measured = row_ix == 0;
            tab.page_up_rest = None;
            tab.motion.stop();
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

    #[cfg(test)]
    fn active_list(&self) -> Option<&ListState> {
        self.tabs.active().map(|t| &t.list)
    }

    fn scroll(&mut self, dy: f32, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.active_mut() {
            tab.page_up_rest = None;
            tab.motion.stop();
            tab.list.scroll_by(px(dy));
            cx.notify();
        }
    }

    /// Mouse wheel and touchpad: a wheel step glides there smoothly, a
    /// touchpad follows the fingers and flings on when they let go.
    fn wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let now = Instant::now();
        match event.delta {
            ScrollDelta::Lines(lines) => {
                if lines.y == 0. {
                    return;
                }
                tab.limit_motion();
                tab.motion.wheel(-lines.y * scroll::LINE_HEIGHT, now);
            }
            // macOS sends the momentum after the fingers lift itself.
            ScrollDelta::Pixels(_)
                if matches!(event.touch_phase, TouchPhase::Ended) && !cfg!(target_os = "macos") =>
            {
                tab.motion.touch_end(now);
                if !tab.motion.is_active() {
                    return;
                }
            }
            ScrollDelta::Pixels(delta) => {
                if delta.y == px(0.) {
                    // Sideways (a code block's), or a touch begins: a wheel
                    // glide or a split PageUp goes on.
                    return;
                }
                let dy = tab.motion.touch(-f32::from(delta.y), now);
                tab.scroll_measured(dy);
            }
        }
        tab.page_up_rest = None;
        cx.notify();
    }

    /// A click on the scrollbar track: a page toward it, like a browser.
    fn page_toward(&mut self, up: bool, window: &Window, cx: &mut Context<Self>) {
        let p = self.page(window);
        if let Some(tab) = self.tabs.active_mut() {
            tab.page_up_rest = None;
            tab.limit_motion();
            tab.motion.wheel(if up { -p } else { p }, Instant::now());
            cx.notify();
        }
    }

    fn start_thumb_drag(
        &mut self,
        y: f32,
        origin: f32,
        track: f32,
        thumb: Thumb,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        tab.motion.stop();
        tab.page_up_rest = None;
        let (_, content, avg) = tab.extent();
        self.thumb_drag = Some(ThumbDrag {
            tab: tab.id,
            origin,
            track,
            grab: y - origin - thumb.top,
            thumb,
            avg,
            content,
        });
        cx.notify();
    }

    fn drag_thumb(&mut self, y: f32, cx: &mut Context<Self>) {
        let (Some(drag), Some(tab)) = (&mut self.thumb_drag, self.tabs.active_mut()) else {
            return;
        };
        if drag.tab != tab.id {
            // The tab was switched or closed during the drag.
            self.thumb_drag = None;
            cx.notify();
            return;
        }
        drag.thumb.top =
            (y - drag.origin - drag.grab).clamp(0., (drag.track - drag.thumb.len).max(0.));
        let pos = scroll::position_for_thumb(
            drag.thumb.top,
            drag.thumb.len,
            drag.content,
            drag.track,
            drag.track,
        );
        tab.scroll_to_estimate(pos, drag.avg);
        cx.notify();
    }

    /// While the thumb is dragged, follows the mouse anywhere, even outside
    /// the window (a div's listeners only see it over the div).
    fn track_thumb_drag(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moved = this.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        return;
                    }
                    moved
                        .update(cx, |this, cx| {
                            if e.pressed_button == Some(MouseButton::Left) {
                                this.drag_thumb(f32::from(e.position.y), cx);
                            } else {
                                // Released where we didn't see it.
                                this.thumb_drag = None;
                                cx.notify();
                            }
                        })
                        .ok();
                });
                let released = this.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                        released
                            .update(cx, |this, cx| {
                                this.thumb_drag = None;
                                cx.notify();
                            })
                            .ok();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    /// The scrollbar at the right edge of the document, when it overflows.
    fn render_scrollbar(
        &self,
        track: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tab = self.tabs.active()?;
        let drag = self.thumb_drag;
        let thumb = match drag {
            Some(drag) => drag.thumb,
            None => {
                let (top, content, _) = tab.extent();
                scroll::thumb(top, content, track, track)?
            }
        };
        let dragging = drag.is_some();
        let origin = TAB_BAR_HEIGHT;
        let idle = theme.muted.opacity(0.45);
        let active = theme.muted.opacity(0.8);
        Some(
            div()
                .id("scrollbar")
                .group("scrollbar")
                .absolute()
                .top_0()
                .right_0()
                .h_full()
                .w(px(SCROLLBAR_WIDTH))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        // Not a click on the document, which would stop the glide.
                        cx.stop_propagation();
                        let y = f32::from(e.position.y) - origin;
                        this.page_toward(y < thumb.top, window, cx);
                    }),
                )
                .child(
                    div()
                        .id("scrollbar-thumb")
                        .absolute()
                        .top(px(thumb.top))
                        .h(px(thumb.len))
                        .right(px(3.))
                        .w(px(if dragging { 8. } else { 6. }))
                        .rounded_full()
                        .bg(if dragging { active } else { idle })
                        .group_hover("scrollbar", |s| s.w(px(8.)).bg(active))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.start_thumb_drag(
                                    f32::from(e.position.y),
                                    origin,
                                    track,
                                    thumb,
                                    cx,
                                );
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn page(&self, window: &Window) -> f32 {
        (f32::from(window.viewport_size().height) - 80.).max(100.)
    }

    fn page_up(&mut self, window: &Window, cx: &mut Context<Self>) {
        let p = self.page(window);
        if let Some(tab) = self.tabs.active_mut() {
            // After jumping past unmeasured rows (Bottom, anchor, reload or
            // width resize), a full-page step could skip over 0-height rows,
            // so `Tab::page_up` splits it across frames.
            tab.page_up(p);
            cx.notify();
        }
    }

    fn render_tab_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tab-bar")
            .flex_none()
            .h(px(TAB_BAR_HEIGHT))
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
            .child(if cfg!(target_os = "macos") {
                "Drop a Markdown file here, or press Cmd+O"
            } else {
                "Drop a Markdown file here, or press Ctrl+O"
            })
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

        // The document's height: the window minus the tab bar.
        let viewport = f32::from(window.viewport_size().height) - TAB_BAR_HEIGHT;
        let active = self.tabs.active_index();
        for (ix, tab) in self.tabs.iter_mut().enumerate() {
            if ix != active {
                // Background tabs don't keep gliding.
                tab.motion.stop();
            }
        }
        if let Some(tab) = self.tabs.active_mut() {
            tab.note_viewport_width(window.viewport_size().width);
            if tab.continue_page_up() {
                // Draw again after this frame's layout to take the next step.
                // (`window.refresh()` is ignored while the window draws.)
                window.defer(cx, |window, _| window.refresh());
            }
            if tab.advance_motion(Instant::now(), viewport) {
                window.request_animation_frame();
            }
            if tab.unmeasured() {
                // Nothing laid out yet: draw once more so the scrollbar
                // is sized from measured rows, not a blind guess.
                window.request_animation_frame();
            }
        }

        if let Some(drag) = self.thumb_drag
            && self.tabs.active().is_none_or(|t| t.id != drag.tab)
        {
            // Its tab was switched away or closed.
            self.thumb_drag = None;
        }
        let scrollbar = self.render_scrollbar(viewport, &theme, cx);
        let content = match self.tabs.active() {
            Some(tab) => {
                let rcx = RenderCx {
                    theme,
                    ui_font: self.fonts.ui.clone(),
                    mono_font: self.fonts.mono.clone(),
                    base_dir: tab.base_dir.clone(),
                    viewer: cx.weak_entity(),
                    code_scroll: tab.code_scroll.clone(),
                };
                let doc = tab.doc.clone();
                let rows = tab.rows.clone();
                div()
                    .id(("tab-doc", tab.doc_id))
                    .size_full()
                    .child(
                        list(tab.list.clone(), move |ix, window, cx| {
                            render::render_row(&doc, &rows[ix], ix, &rcx, window, cx)
                        })
                        .without_scroll_wheel()
                        .size_full(),
                    )
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
            .on_action(cx.listener(|this, _: &Open, _, cx| {
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: Some("Open Markdown".into()),
                    directory: this.open_dialog_directory(),
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
            .on_action(cx.listener(|this, _: &PageUp, window, cx| this.page_up(window, cx)))
            .on_action(cx.listener(|this, _: &PageDown, window, cx| {
                let p = this.page(window);
                this.scroll(p, cx)
            }))
            .on_action(cx.listener(|this, _: &Top, _, cx| {
                if let Some(tab) = this.tabs.active_mut() {
                    tab.list.scroll_to(ListOffset::default());
                    tab.leading_measured = true;
                    tab.page_up_rest = None;
                    tab.motion.stop();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Bottom, _, cx| {
                if let Some(tab) = this.tabs.active_mut() {
                    // Scroll past the last row so GPUI fills the viewport
                    // upward from the end even when middle rows are
                    // unmeasured (height 0 in the SumTree).
                    tab.list.scroll_to(ListOffset {
                        item_ix: tab.list.item_count(),
                        offset_in_item: px(0.),
                    });
                    tab.leading_measured = false;
                    tab.page_up_rest = None;
                    tab.motion.stop();
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.open_paths(paths.paths().to_vec(), cx);
            }))
            .when(!self.tabs.is_empty(), |d| {
                d.child(self.render_tab_bar(&theme, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .relative()
                    .on_scroll_wheel(
                        cx.listener(|this, e: &ScrollWheelEvent, _, cx| this.wheel(e, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| {
                            // A click stops a glide or fling, like a tap in a browser.
                            if let Some(tab) = this.tabs.active_mut() {
                                tab.motion.stop();
                            }
                        }),
                    )
                    .child(content)
                    .children(scrollbar)
                    .when(self.thumb_drag.is_some(), |d| {
                        d.child(self.track_thumb_drag(cx))
                    }),
            )
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
    fn split_page_up_takes_one_measured_step_per_laid_out_frame() {
        let (step, rest) = PageUpRest::start(None, 2000.);
        assert_eq!(step, MEASURED_STEP);
        let rest = rest.unwrap();
        assert_eq!(
            rest,
            PageUpRest {
                px: 2000. - MEASURED_STEP,
                laid_out: false,
            }
        );
        // The frame right after the action lays out the first step.
        let (step, rest) = rest.frame(false);
        assert_eq!(step, 0.);
        let rest = rest.unwrap();
        assert_eq!(rest.px, 2000. - MEASURED_STEP);
        assert!(rest.laid_out);
        let (step, rest) = rest.frame(false);
        assert_eq!(step, MEASURED_STEP);
        let rest = rest.unwrap();
        assert_eq!(rest.px, 2000. - 2. * MEASURED_STEP);
        // The remainder is smaller than a step and ends the PageUp.
        let (step, rest) = rest.frame(false);
        assert_eq!(step, 2000. - 2. * MEASURED_STEP);
        assert_eq!(rest, None);
    }

    #[test]
    fn split_page_up_fits_in_one_step_or_adds_to_a_pending_rest() {
        assert_eq!(PageUpRest::start(None, 500.), (500., None));
        assert_eq!(
            PageUpRest::start(None, MEASURED_STEP),
            (MEASURED_STEP, None)
        );
        let pending = PageUpRest {
            px: 300.,
            laid_out: true,
        };
        assert_eq!(
            PageUpRest::start(Some(pending), 500.),
            (
                MEASURED_STEP,
                Some(PageUpRest {
                    px: 800. - MEASURED_STEP,
                    laid_out: false,
                })
            )
        );
    }

    #[test]
    fn split_page_up_finishes_at_once_when_rows_above_are_measured() {
        let rest = PageUpRest {
            px: 3000.,
            laid_out: true,
        };
        assert_eq!(rest.frame(true), (3000., None));
        // Not before the previous step has been laid out.
        let rest = PageUpRest {
            px: 3000.,
            laid_out: false,
        };
        assert_eq!(
            rest.frame(true),
            (
                0.,
                Some(PageUpRest {
                    px: 3000.,
                    laid_out: true,
                })
            )
        );
    }

    #[test]
    fn tab_page_up_splits_only_when_rows_above_may_be_unmeasured() {
        let src: String = (0..20).map(|i| format!("p{i}\n\n")).collect();
        let mut tab = Tab::new(PathBuf::from("/x/a.md"), Ok((markdown::parse(&src), None)));
        tab.list.scroll_to(ListOffset {
            item_ix: 10,
            offset_in_item: px(0.),
        });
        tab.page_up(2000.);
        assert_eq!(tab.page_up_rest, None);
        assert!(!tab.continue_page_up());

        tab.list.scroll_to(ListOffset {
            item_ix: 10,
            offset_in_item: px(0.),
        });
        tab.leading_measured = false;
        tab.page_up(2000.);
        assert_eq!(tab.page_up_rest, PageUpRest::start(None, 2000.).1);
        assert!(tab.continue_page_up());
        assert!(tab.page_up_rest.unwrap().laid_out);
        // A reload drops the rest of the PageUp.
        tab.replace(Ok((markdown::parse(&src), None)));
        assert_eq!(tab.page_up_rest, None);
        assert!(!tab.continue_page_up());
    }

    #[test]
    fn replace_keeps_heading_when_preceding_table_grows() {
        let make = |n: usize| {
            let mut s = String::from("| ID |\n|---|\n");
            for i in 0..n {
                s.push_str(&format!("| {i} |\n"));
            }
            s.push_str("\n## Heading After Table\n\nBody\n");
            markdown::parse(&s)
        };
        let mut tab = Tab::new(PathBuf::from("/x/a.md"), Ok((make(5), None)));
        // Block 0 = table (rows 0..=5), block 1 = heading (row 6).
        tab.list.scroll_to(ListOffset {
            item_ix: 6,
            offset_in_item: px(4.),
        });
        tab.replace(Ok((make(105), None)));
        // Block 1 moved to row 106; scroll stays on the heading.
        let top = tab.list.logical_scroll_top();
        assert_eq!((top.item_ix, top.offset_in_item), (106, px(4.)));
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

    #[test]
    fn parent_dir_skips_empty_and_root() {
        assert_eq!(
            parent_dir(Path::new("/a/b/readme.md")),
            Some(PathBuf::from("/a/b"))
        );
        assert_eq!(
            parent_dir(Path::new("/readme.md")),
            Some(PathBuf::from("/"))
        );
        assert_eq!(parent_dir(Path::new("readme.md")), None);
        assert_eq!(parent_dir(Path::new("/")), None);
    }

    #[test]
    fn resolve_open_directory_falls_back_in_order() {
        let dir = TempDir::new("resolve-open-dir");
        let active_dir = dir.join("active");
        let last_dir = dir.join("last");
        let cwd_dir = dir.join("cwd");
        let missing_dir = dir.join("missing");
        std::fs::create_dir(&active_dir).unwrap();
        std::fs::create_dir(&last_dir).unwrap();
        std::fs::create_dir(&cwd_dir).unwrap();

        let active_file = active_dir.join("a.md");
        let missing_file = missing_dir.join("gone.md");

        // Active tab's parent wins over last_open_dir and cwd.
        assert_eq!(
            resolve_open_directory(Some(&active_file), Some(&last_dir), Some(&cwd_dir)),
            Some(active_dir.clone())
        );
        // Missing active parent falls back to last_open_dir.
        assert_eq!(
            resolve_open_directory(Some(&missing_file), Some(&last_dir), Some(&cwd_dir)),
            Some(last_dir.clone())
        );
        // No active tab uses last_open_dir.
        assert_eq!(
            resolve_open_directory(None, Some(&last_dir), Some(&cwd_dir)),
            Some(last_dir)
        );
        // Missing last_open_dir falls back to cwd.
        assert_eq!(
            resolve_open_directory(None, Some(&missing_dir), Some(&cwd_dir)),
            Some(cwd_dir)
        );
        // Nothing exists: None.
        assert_eq!(
            resolve_open_directory(Some(&missing_file), Some(&missing_dir), Some(&missing_dir)),
            None
        );
    }

    /// A tab of `n` unmeasured rows (60 px each, by estimate) at `item_ix`.
    fn tab_at(n: usize, item_ix: usize) -> Tab {
        let mut tab = Tab::new(PathBuf::from("/x/a.md"), Ok((doc_with(n), None)));
        tab.list.scroll_to(ListOffset {
            item_ix,
            offset_in_item: px(0.),
        });
        tab.leading_measured = item_ix == 0;
        tab
    }

    #[test]
    fn reload_stops_motion() {
        let t0 = Instant::now();
        let mut tab = tab_at(100, 50);
        tab.motion.wheel(-300., t0);
        tab.replace(Ok((doc_with(100), None)));
        assert!(!tab.motion.is_active());
        assert_eq!(tab.list.logical_scroll_top().item_ix, 50);
    }

    #[test]
    fn scroll_to_estimate_tracks_leading_measured() {
        let mut tab = tab_at(100, 50);
        tab.scroll_to_estimate(0., 60.);
        assert!(tab.leading_measured);
        assert_eq!(tab.list.logical_scroll_top().item_ix, 0);
        tab.scroll_to_estimate(30. * 60. + 7., 60.);
        assert!(!tab.leading_measured);
        let top = tab.list.logical_scroll_top();
        assert_eq!((top.item_ix, top.offset_in_item), (30, px(7.)));
    }

    #[test]
    fn unmeasured_only_with_rows() {
        assert!(tab_at(3, 0).unmeasured());
        let empty = Tab::new(PathBuf::from("/x/e.md"), Ok((markdown::parse(""), None)));
        assert_eq!(empty.list.item_count(), 0);
        assert!(!empty.unmeasured());
    }
}
