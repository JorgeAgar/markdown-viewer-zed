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
    WindowDecorations, WindowOptions, px, size,
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

    let app = Application::new().with_http_client(std::sync::Arc::new(http::ImageHttpClient));
    app.run(move |cx: &mut App| {
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
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let fonts = pick_fonts(cx);
        let loaded = preload.join().unwrap_or_default();

        let desired = size(px(920.), px(1040.));
        #[cfg(windows)]
        let desired = cx
            .primary_display()
            .map(|display| windows_initial_size(display.bounds().size))
            .unwrap_or(desired);
        let bounds = Bounds::centered(None, desired, cx);
        let window = cx
            .open_window(
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
                },
                |window, cx| cx.new(|cx| Viewer::new(loaded, fonts, window, cx)),
            )
            .expect("failed to open window");

        // Single-instance server: other `mdview` invocations send us paths.
        if let Some(listener) = listener {
            let (tx, mut rx) = mpsc::unbounded::<Vec<PathBuf>>();
            std::thread::spawn(move || {
                listener.serve(|paths| tx.unbounded_send(paths).is_ok());
            });
            cx.spawn(async move |cx| {
                while let Some(paths) = rx.next().await {
                    window
                        .update(cx, |viewer, window, cx| {
                            viewer.open_paths(paths, cx);
                            window.activate_window();
                        })
                        .ok();
                }
            })
            .detach();
        }
    });

    if owns_socket {
        platform::ipc::cleanup();
    }
}

/// Pick the first installed font from a short preference list (overridable via
/// MDVIEW_FONT / MDVIEW_MONO_FONT). Avoids spawning fontconfig at startup.
fn pick_fonts(cx: &App) -> Fonts {
    let installed: std::collections::HashSet<String> =
        cx.text_system().all_font_names().into_iter().collect();
    let pick = |env: &str, candidates: &[&'static str]| -> gpui::SharedString {
        let from_env = std::env::var(env).ok();
        from_env
            .iter()
            .map(|s| s.as_str())
            .chain(candidates.iter().copied())
            .find(|name| installed.contains(*name))
            .map(|s| s.to_string().into())
            .unwrap_or_else(|| ".SystemUIFont".into())
    };
    Fonts {
        ui: pick(
            "MDVIEW_FONT",
            &[
                "Inter",
                "IBM Plex Sans",
                "Noto Sans",
                "Cantarell",
                "DejaVu Sans",
                "Liberation Sans",
                "Segoe UI",
            ],
        ),
        mono: pick(
            "MDVIEW_MONO_FONT",
            &[
                "JetBrains Mono",
                "Lilex",
                "Fira Code",
                "Noto Sans Mono",
                "DejaVu Sans Mono",
                "Liberation Mono",
                "Consolas",
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
