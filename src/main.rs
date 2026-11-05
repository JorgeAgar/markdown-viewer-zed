//! mdview — a tiny GPU-rendered Markdown viewer on top of GPUI (Zed's UI framework).
//!
//! Startup strategy (borrowed from Zed's CLI):
//! 1. If an instance is already running, hand the paths to it over a Unix
//!    socket and exit immediately — no GPU/window/font init at all.
//! 2. Otherwise, read + parse the files on a worker thread while GPUI boots
//!    (Wayland/X11 connection, font database, GPU context) on the main thread.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cli;
mod graphics;
mod http;
mod links;
mod platform;
mod render;
mod tabs;
#[cfg(test)]
mod test_util;
mod theme;
mod viewer;

use std::path::PathBuf;

use futures::{StreamExt, channel::mpsc};
// Re-exported at the crate root so the rest of the app can keep writing
// `crate::markdown`.
use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds,
    WindowDecorations, WindowHandle, WindowOptions, px, size,
};
use mdview_core::{highlight, markdown};

use viewer::{
    Bottom, CloseTab, Fonts, NextTab, Open, PageDown, PageUp, PrevTab, Quit, Reload, ScrollDown,
    ScrollUp, Top, Viewer,
};

/// Process start, for `MDVIEW_TRACE_STARTUP=1`.
pub static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn main() {
    START.get_or_init(std::time::Instant::now);
    let cli::Command::Run {
        standalone,
        mut paths,
    } = cli::parse_args(std::env::args_os().skip(1))
    else {
        print!("{}", cli::HELP);
        return;
    };
    for p in &mut paths {
        *p = viewer::normalize(p);
    }

    let listener = match platform::ipc::start(standalone, &paths) {
        platform::ipc::Instance::Forwarded => return,
        platform::ipc::Instance::Primary(listener) => listener,
    };
    let owns_socket = listener.is_some();

    // Parse in parallel with GPUI start-up.
    let preload = std::thread::spawn(move || paths.into_iter().map(viewer::load).collect());

    let (tx, mut rx) = mpsc::unbounded::<Vec<PathBuf>>();
    let app = Application::new().with_http_client(std::sync::Arc::new(http::ImageHttpClient));
    #[cfg(target_os = "macos")]
    platform::mac::configure(
        |callback| {
            app.on_open_urls(callback);
        },
        |mut callback| {
            app.on_reopen(move |_| callback());
        },
        tx.clone(),
    );
    app.run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        platform::mac::install_menus(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-o", Open, None),
            KeyBinding::new("ctrl-w", CloseTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-pagedown", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PrevTab, None),
            KeyBinding::new("ctrl-pageup", PrevTab, None),
            KeyBinding::new("ctrl-r", Reload, None),
            KeyBinding::new("f5", Reload, None),
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("up", ScrollUp, Some("Viewer")),
            KeyBinding::new("k", ScrollUp, Some("Viewer")),
            KeyBinding::new("down", ScrollDown, Some("Viewer")),
            KeyBinding::new("j", ScrollDown, Some("Viewer")),
            KeyBinding::new("pageup", PageUp, Some("Viewer")),
            KeyBinding::new("shift-space", PageUp, Some("Viewer")),
            KeyBinding::new("pagedown", PageDown, Some("Viewer")),
            KeyBinding::new("space", PageDown, Some("Viewer")),
            KeyBinding::new("home", Top, Some("Viewer")),
            KeyBinding::new("g", Top, Some("Viewer")),
            KeyBinding::new("end", Bottom, Some("Viewer")),
            KeyBinding::new("shift-g", Bottom, Some("Viewer")),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        #[cfg(not(target_os = "macos"))]
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let loaded = preload.join().unwrap_or_default();
        let mut window = open_viewer(loaded, cx);
        #[cfg(target_os = "macos")]
        cx.activate(true);

        // Single-instance server: other `mdview` invocations send us paths.
        if let Some(listener) = listener {
            std::thread::spawn(move || {
                listener.serve(|paths| tx.unbounded_send(paths).is_ok());
            });
        }
        cx.spawn(async move |cx| {
            while let Some(paths) = rx.next().await {
                cx.update(|cx| deliver_paths(&mut window, paths, cx)).ok();
            }
        })
        .detach();
    });

    if owns_socket {
        platform::ipc::cleanup();
    }
}

/// Reuse the current window, or recreate it after a macOS window close.
fn deliver_paths(window: &mut WindowHandle<Viewer>, paths: Vec<PathBuf>, cx: &mut App) {
    if window
        .update(cx, |viewer, window, cx| {
            viewer.open_paths(paths.clone(), cx);
            window.activate_window();
        })
        .is_err()
    {
        let loaded = paths
            .into_iter()
            .map(|p| viewer::load(viewer::normalize(&p)))
            .collect();
        *window = open_viewer(loaded, cx);
    }
}

/// All entry points (CLI, Finder and Dock) create the same viewer.
fn open_viewer(loaded: Vec<viewer::Loaded>, cx: &mut App) -> WindowHandle<Viewer> {
    let fonts = pick_fonts(cx);
    let desired = size(px(920.), px(1040.));
    #[cfg(windows)]
    let desired = cx
        .primary_display()
        .map(|display| windows_initial_size(display.bounds().size))
        .unwrap_or(desired);
    let bounds = Bounds::centered(None, desired, cx);
    cx.open_window(viewer_window_options(bounds), |window, cx| {
        cx.new(|cx| Viewer::new(loaded, fonts, window, cx))
    })
    .expect("failed to open window")
}

/// Shared native-window contract: initial opens and Dock reopens must retain
/// native chrome, desktop grouping, minimum size and the opaque startup path.
fn viewer_window_options(bounds: Bounds<gpui::Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("mdview".into()),
            ..Default::default()
        }),
        app_id: Some("mdview".into()),
        window_decorations: Some(WindowDecorations::Server),
        window_min_size: Some(size(px(320.), px(240.))),
        ..Default::default()
    }
}

// GPUI advertises fallback names even when the underlying family is absent.
// On macOS a missing family resolves to regular Helvetica, losing text styles.
const UI_FONT_CANDIDATES: &[&str] = &[
    "Inter",
    "IBM Plex Sans",
    #[cfg(target_os = "macos")]
    "Helvetica Neue",
    "Noto Sans",
    "Cantarell",
    "DejaVu Sans",
    "Liberation Sans",
    "Segoe UI",
    #[cfg(not(target_os = "macos"))]
    "Helvetica Neue",
    "Helvetica",
];

fn select_font(
    known_names: &std::collections::HashSet<String>,
    override_name: Option<&str>,
    candidates: &[&str],
) -> gpui::SharedString {
    override_name
        .into_iter()
        .chain(candidates.iter().copied())
        .find(|name| known_names.contains(*name))
        .map(|name| name.to_string().into())
        .unwrap_or_else(|| ".SystemUIFont".into())
}

/// Prefer available families without spawning fontconfig at startup.
fn pick_fonts(cx: &App) -> Fonts {
    let known_names = cx.text_system().all_font_names().into_iter().collect();
    Fonts {
        ui: select_font(
            &known_names,
            std::env::var("MDVIEW_FONT").ok().as_deref(),
            UI_FONT_CANDIDATES,
        ),
        mono: select_font(
            &known_names,
            std::env::var("MDVIEW_MONO_FONT").ok().as_deref(),
            &[
                "JetBrains Mono",
                "Lilex",
                "Fira Code",
                "Noto Sans Mono",
                "DejaVu Sans Mono",
                "Liberation Mono",
                "Consolas",
                "Menlo",
                "Monaco",
            ],
        ),
    }
}

#[cfg(any(windows, test))]
fn windows_initial_size(display: gpui::Size<gpui::Pixels>) -> gpui::Size<gpui::Pixels> {
    // GPUI centers the requested client size without constraining it to the
    // monitor. Reserve space for the native frame and taskbar on small screens.
    size(
        (display.width - px(128.)).clamp(px(320.), px(920.)),
        (display.height - px(128.)).clamp(px(240.), px(1040.)),
    )
}

#[cfg(test)]
mod window_contract_tests {
    use super::*;
    #[test]
    fn native_window_contract_survives_initialization_and_reopening() {
        let bounds = Bounds::new(gpui::point(px(10.), px(20.)), size(px(920.), px(1040.)));
        let options = viewer_window_options(bounds);
        assert!(
            matches!(options.window_bounds, Some(WindowBounds::Windowed(b)) if b == bounds),
            "preserve caller's placement when opening/reopening"
        );
        let titlebar = options
            .titlebar
            .expect("native window controls must remain present");
        assert_eq!(
            titlebar.title.as_ref().map(|title| title.to_string()),
            Some("mdview".to_string()),
            "identify the window before document rendering"
        );
        assert_eq!(
            options.app_id.as_deref(),
            Some("mdview"),
            "desktop grouping must remain stable"
        );
        assert_eq!(
            options.window_min_size,
            Some(size(px(320.), px(240.))),
            "prevent unusably small viewer windows"
        );
        assert!(
            matches!(options.window_decorations, Some(WindowDecorations::Server)),
            "opaque startup optimization needs explicit server decorations"
        );
        assert!(matches!(
            options.window_background,
            gpui::WindowBackgroundAppearance::Opaque
        ));
        assert!(options.show && options.focus && options.is_resizable);
    }
}

#[cfg(test)]
mod window_size_tests {
    use super::*;
    #[test]
    fn windows_initial_window_keeps_controls_on_small_desktops() {
        for display in [
            size(px(1024.), px(768.)),
            size(px(800.), px(600.)),
            size(px(720.), px(1280.)),
        ] {
            let initial = windows_initial_size(display);
            assert!(
                initial.width + px(128.) <= display.width,
                "leave horizontal room for native frame"
            );
            assert!(
                initial.height + px(128.) <= display.height,
                "keep title bar and lower controls on screen"
            );
            assert!(initial.width >= px(320.) && initial.height >= px(240.));
        }
        assert_eq!(
            windows_initial_size(size(px(2560.), px(1440.))),
            size(px(920.), px(1040.))
        );
    }
}

#[cfg(test)]
mod font_selection_tests {
    use super::*;
    #[test]
    fn font_selection_keeps_overrides_and_ordered_fallbacks() {
        let names = ["Preferred", "Native", "Override"]
            .map(str::to_string)
            .into_iter()
            .collect();
        assert_eq!(
            select_font(&names, Some("Override"), &["Preferred", "Native"]).to_string(),
            "Override"
        );
        assert_eq!(
            select_font(&names, Some("Missing"), &["Preferred", "Native"]).to_string(),
            "Preferred"
        );
        assert_eq!(
            select_font(&names, None, &["Missing", "Native"]).to_string(),
            "Native"
        );
        assert_eq!(
            select_font(&names, None, &["Missing"]).to_string(),
            ".SystemUIFont"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_prefers_native_family_over_advertised_fallback() {
        let names = ["Noto Sans", "Helvetica Neue"]
            .map(str::to_string)
            .into_iter()
            .collect();
        assert_eq!(
            select_font(&names, None, UI_FONT_CANDIDATES).to_string(),
            "Helvetica Neue"
        );
    }
}
