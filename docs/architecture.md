# Architecture

## Repository layout

```
.
├── src/
│   ├── main.rs        Entry point: arguments, single-instance socket, key bindings, font choice, window creation
│   ├── viewer.rs      The `Viewer` view: tabs, actions, live reload, drag & drop, file loading
│   ├── markdown.rs    Markdown → block model (pulldown-cmark), plus parser tests
│   ├── highlight.rs   Syntax highlighting for code blocks (hand-written lexers, no GPUI)
│   ├── render.rs      Block model → GPUI elements; link handling
│   └── theme.rs       Light and dark palettes
├── crates/gpui/       Vendored GPUI 0.2.2 with startup fixes (see gpui-fork.md)
├── patches/           Those fixes as a diff against upstream GPUI
├── examples/          Sample Markdown files for manual testing
└── docs/              This documentation
```

## Modules and dependencies

```mermaid
flowchart TD
    main["main.rs<br/>startup, IPC, key bindings"]
    viewer["viewer.rs<br/>Viewer: tabs, actions, reload"]
    markdown["markdown.rs<br/>parse() → Document"]
    highlight["highlight.rs<br/>highlight() → Vec of Highlight"]
    render["render.rs<br/>top_block() → AnyElement"]
    theme["theme.rs<br/>Theme"]
    gpui[("crates/gpui<br/>GPUI (vendored)")]
    cmark[("pulldown-cmark")]

    main --> viewer
    main --> gpui
    viewer --> markdown
    viewer --> render
    viewer --> theme
    render --> markdown
    render --> theme
    render --> gpui
    viewer --> gpui
    markdown --> cmark
    markdown --> highlight
    theme --> highlight
```

Rules that keep this structure simple:

- **`markdown.rs` and `highlight.rs` know nothing about GPUI.** It turns text into plain data
  (`Document`). That keeps it fast, easy to unit-test and safe to run on any
  thread.
- **`render.rs` only reads that data.** It never parses Markdown again. It's
  the only place where Markdown concepts become visual elements.
- **`viewer.rs` owns all state**: tabs, which tab is active, scroll positions
  and file modification times.

## Data model

The parser output is a tree of `Block`s. Inline formatting is flattened into
`RichText`: one `String` plus non-overlapping style spans.

```mermaid
classDiagram
    class Document {
        blocks: Vec~Block~
    }
    class Block {
        <<enum>>
        Heading(level, RichText, slug)
        Paragraph(RichText)
        Images(Vec~Image~)
        Code(lang, code, highlights)
        Quote(Option~Callout~, Vec~Block~)
        List(start, Vec~ListItem~)
        Table(aligns, header, rows)
        Footnote(label, Vec~Block~)
        Rule
    }
    class ListItem {
        checked: Option~bool~
        blocks: Vec~Block~
    }
    class RichText {
        text: String
        spans: Vec~Span~
        links: Vec~String~
    }
    class Span {
        range: Range~usize~
        style: u8  BOLD|ITALIC|STRIKE|CODE|LINK
        link: Option~u32~
    }
    Document --> Block
    Block --> ListItem
    Block --> RichText
    ListItem --> Block
    RichText --> Span
```

Each **top-level** block becomes one row of the virtualized list (see
[rendering.md](rendering.md)). Blocks nested inside a quote or list are drawn
as part of their parent's row.

## Runtime state

```mermaid
classDiagram
    class Viewer {
        tabs: Vec~Tab~
        active: usize
        focus: FocusHandle
        fonts: Fonts
        window_title: SharedString
    }
    class Tab {
        path: PathBuf  canonical
        title: SharedString
        doc: Arc~Document~
        mtime: Option~SystemTime~
        list: ListState  scroll + measured heights
    }
    Viewer "1" --> "*" Tab
```

- Paths are canonicalized (`normalize`), so opening the same file twice just
  switches to its tab.
- `Tab::replace` swaps in a newly parsed document and keeps the scroll
  position. Live reload and `Ctrl+R` both use it.

## Threads

```mermaid
flowchart LR
    subgraph Main["Main thread (GPUI foreground)"]
        UI["Event loop, layout, rendering, Viewer state"]
    end
    subgraph Startup["Startup only"]
        P["preload: read + parse initial files"]
        G["gpu_init: Vulkan instance + device"]
        PL["8 threads: one render pipeline each"]
    end
    subgraph Long["Long-lived"]
        IPC["IPC listener: accept() on the Unix socket"]
        BG["GPUI background executor<br/>(sized to the number of CPUs)"]
    end
    IPC -->|"mpsc channel: Vec of PathBuf"| UI
    P -->|"join() before the window opens"| UI
    G -->|"join() inside the platform client"| UI
```

- The startup threads are created once and joined. Their number doesn't depend
  on the machine; the operating system spreads them over however many cores
  there are.
- All `Viewer` state lives on the main thread. Work on other threads talks to
  it only through a channel (`futures::channel::mpsc`), or through
  `WeakEntity::update` from an async task.
- Live reload is an async task on the main thread: a 750 ms timer, then
  `stat()` on each open file. It doesn't use a file-watcher thread.

## Single-instance protocol

The socket lives at `$XDG_RUNTIME_DIR/mdview.sock`, or
`$TMPDIR/mdview-$USER.sock` when `XDG_RUNTIME_DIR` isn't set. The protocol is
deliberately trivial: the client writes absolute paths separated by `\n` and
closes the connection. An empty message just raises the window.

```mermaid
sequenceDiagram
    participant C as mdview b.md (client)
    participant S as Socket
    participant L as IPC listener thread
    participant UI as Main thread (Viewer)

    C->>S: connect()
    alt An instance is running
        C->>S: write("/abs/path/b.md")
        C-->>C: exit(0) (~4 ms total)
        S->>L: accept() + read_to_string
        L->>UI: unbounded_send(paths)
        UI->>UI: open_paths() → new tab, activate_window()
    else Nothing is listening (or a stale socket)
        C->>C: remove the stale file, bind(), become the server
    end
```

`-n` / `--new` skips the protocol and doesn't bind the socket. That's useful
for benchmarks and debugging.

## Links

`render::open_link` decides what clicking a link does:

| Link | Action |
| --- | --- |
| `#anchor` | Scroll the current tab to the heading whose slug matches |
| `http(s)://…`, `mailto:` | Open with the system handler (`cx.open_url`) |
| Relative or absolute `*.md` (with an optional `#anchor`) | Open in a tab (or switch to it), then scroll to the anchor |
| Any other local file | Open with the system handler (`file://…`) |

Anchor slugs follow GitHub's rules (`markdown::slugify`). Only top-level
headings can be anchor targets.
