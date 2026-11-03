<img src="assets/icon.svg" width="64" height="64" alt="">

# mdview

A tiny, read-only Markdown viewer that starts fast. It's built on
[GPUI](https://gpui.rs), the GPU-accelerated UI framework behind Zed. There's no
editor and no source view. It opens Markdown files, renders them, and shows them
in tabs.

## Usage

```sh
cargo build --release
./target/release/mdview README.md docs/*.md
```

| Key                                  | Action                                    |
| ------------------------------------ | ----------------------------------------- |
| `Ctrl+O`                             | Open files (portal file picker)           |
| `Ctrl+W`                             | Close tab (closes the window if empty)    |
| `Ctrl+Tab` / `Ctrl+Shift+Tab`        | Next / previous tab                       |
| `Ctrl+PgDn` / `Ctrl+PgUp`            | Next / previous tab                       |
| `Ctrl+R` / `F5`                      | Reload                                    |
| `Ctrl+Q`                             | Quit                                      |
| `↑ ↓` `j k` `PgUp PgDn` `Space` `Home End` `g G` | Scroll                       |
| Middle click on a tab                | Close it                                  |

You can also drag files onto the window. Clicking a link to a local `.md` file
opens it in a new tab, and `#anchor` links scroll to the matching heading. Open
files reload automatically when they change on disk.

`-n` / `--new` starts a separate instance. `MDVIEW_FONT` and `MDVIEW_MONO_FONT`
override the fonts.

## Why it starts fast

- **Single instance (the same approach as Zed's CLI).** The first `mdview`
  listens on `$XDG_RUNTIME_DIR/mdview.sock`. Later runs send their paths over
  that socket and exit right away, so they skip GPU, window and font setup.
  The paths open as tabs in the window that's already running.
- **Files are parsed in parallel with startup.** A worker thread reads and
  parses the files (`pulldown-cmark`) while GPUI connects to Wayland/X11, loads
  the fonts and creates the GPU context.
- **Each file is parsed once.** The parser turns a file into a compact list of
  blocks, and frames are drawn from that list. The raw Markdown isn't parsed
  again.
- **Virtualized list.** As in Zed's Markdown preview, only the blocks on
  screen get laid out.
- **Patched GPUI.** GPUI is vendored in `crates/gpui` with three startup
  fixes: it no longer compiles the render pipelines twice, it compiles them in
  parallel, and it initializes Vulkan while the fonts load. Time to the first
  frame dropped from ~105 to ~89 ms on an Intel GPU, and from ~159 to ~103 ms
  on lavapipe. Details are in [crates/gpui/MDVIEW.md](crates/gpui/MDVIEW.md).
- **Small dependency footprint.** The only dependencies are GPUI,
  `pulldown-cmark` and `futures`. Syntax highlighting is a small built-in
  lexer that runs while parsing, so it loads nothing at startup. There's no
  HTTP stack and no editor. The release build uses full LTO, `panic=abort` and
  is stripped.

## Documentation

The architecture, startup performance, rendering pipeline, the GPUI fork, how
to contribute, and the roadmap are documented, with diagrams, in
[`docs/`](docs/README.md). AI agents: see [`AGENTS.md`](AGENTS.md).

## Layout

```
src/main.rs      startup, single-instance socket, key bindings, font pick
src/viewer.rs    window: tabs, actions, live reload, drag & drop
src/markdown.rs  Markdown -> block model (pulldown-cmark)
src/highlight.rs syntax highlighting for code blocks (built-in lexers)
src/render.rs    block model -> GPUI elements
src/theme.rs     light/dark palettes (follows the system appearance)
crates/gpui/     vendored GPUI 0.2.2 with startup fixes (see MDVIEW.md)
patches/         those fixes as a diff against upstream gpui 0.2.2
docs/            documentation for contributors (with diagrams)
```

## Not supported yet

- Remote (`http://`) images. They show up as links.
- Raw HTML blocks. They're skipped.
- Text selection and copy.
