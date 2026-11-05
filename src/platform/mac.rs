//! LaunchServices delivers file-open events separately from argv on macOS.
use std::path::PathBuf;

pub fn finder_paths(urls: Vec<String>) -> Vec<PathBuf> {
    urls.into_iter()
        .filter_map(|url| reqwest::Url::parse(&url).ok()?.to_file_path().ok())
        .collect()
}

#[cfg(target_os = "macos")]
pub fn configure(
    app: &gpui::Application,
    tx: futures::channel::mpsc::UnboundedSender<Vec<PathBuf>>,
) {
    let reopen = tx.clone();
    app.on_open_urls(move |urls| {
        let paths = finder_paths(urls);
        if !paths.is_empty() {
            let _ = tx.unbounded_send(paths);
        }
    });
    app.on_reopen(move |_| {
        let _ = reopen.unbounded_send(Vec::new());
    });
}

#[cfg(target_os = "macos")]
pub fn install_menus(cx: &mut gpui::App) {
    use crate::viewer::{CloseTab, NextTab, Open, PrevTab, Quit, Reload};
    use gpui::{KeyBinding, Menu, MenuItem, SystemMenuType};
    cx.bind_keys([
        KeyBinding::new("cmd-o", Open, None),
        KeyBinding::new("cmd-w", CloseTab, None),
        KeyBinding::new("cmd-r", Reload, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("cmd-shift-[", PrevTab, None),
    ]);
    cx.set_menus(vec![
        Menu {
            name: "mdview".into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit mdview", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Open…", Open),
                MenuItem::action("Close Tab", CloseTab),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Reload", Reload),
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PrevTab),
            ],
        },
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finder_decodes_files_and_ignores_other_urls() {
        assert_eq!(
            finder_paths(vec![
                "file:///tmp/a%20b%23.md".into(),
                "https://example.com/a.md".into(),
                "not a URL".into(),
                "file:///tmp/%E6%97%A5%E6%9C%AC%E8%AA%9E.md".into(),
            ]),
            vec![
                PathBuf::from("/tmp/a b#.md"),
                PathBuf::from("/tmp/日本語.md")
            ]
        );
        assert!(finder_paths(vec![]).is_empty());
    }
}
