# AGENTS.md

Instructions for AI coding agents working on this repository. People should
read [docs/contributing.md](docs/contributing.md), which covers the same rules
in more detail.

## What this is

mdview is a **read-only Markdown viewer with tabs**, built on GPUI (Zed's UI
framework) and optimized for **startup time**. The full docs are in
[`docs/`](docs/README.md). Start with
[docs/architecture.md](docs/architecture.md).

## Hard rules

1. **Commit every change.** When a task is done (or at each logical step of a
   larger one), commit it. Never leave work uncommitted. End commit messages
   with the attribution your harness requires, if any.
2. **Startup time is the product.** Don't add work before the socket check in
   `main()`, blocking I/O or subprocesses at startup, or new dependencies
   without measured numbers. See the performance rules in
   [docs/contributing.md](docs/contributing.md#performance-rules).
3. **Respect the scope.** No editing, no showing the Markdown source, no
   settings UI. If a request conflicts with
   [the goals](docs/README.md#what-the-project-is-and-isnt), ask first.
4. **`markdown.rs` must not depend on GPUI.** Parse once in `markdown.rs`;
   only read the result in `render.rs`.
5. **Changes to `crates/gpui`** need a `// mdview:` comment, a regenerated
   `patches/gpui-0.2.2-startup.patch`, and updated `crates/gpui/MDVIEW.md`.
   See [docs/gpui-fork.md](docs/gpui-fork.md).
6. **Keep the docs in sync.** If you change behavior, keys, modules or data
   types, update the README and the relevant `docs/` page (including the
   Mermaid diagrams) in the same commit.

## Commands

```sh
cargo build --release                                    # build (full LTO)
cargo test --release                                     # parser tests
./target/release/mdview -n examples/demo.md              # run a separate instance
MDVIEW_TRACE_STARTUP=quit ./target/release/mdview -n examples/demo.md   # time to first frame, then exit
cargo fmt
```

## Verifying your work

- Run `cargo build --release` and `cargo test --release`. Both must pass.
- For UI changes, actually run the app and look at it, in light and dark mode
  if colors are involved. On Wayland you can take a screenshot with `grim`
  while the app runs.
- For startup changes, compare the median of at least 5 runs of
  `MDVIEW_TRACE_STARTUP=quit … -n` before and after, and put the numbers in
  the commit message. For a breakdown per phase, use the method in
  [docs/startup.md](docs/startup.md), on a copy **outside the repo**. Never
  commit timing marks.

## Pitfalls

- If `cargo update` breaks the build with `cannot find value ENOATTR`, run
  `cargo update -p libc --precise 0.2.189`. Details are in
  [docs/gpui-fork.md](docs/gpui-fork.md).
- Without `-n`, launching mdview while another instance runs just sends the
  paths to that instance and exits. Use `-n` for testing.
- `InteractiveText` and scrollable divs need unique `ElementId`s within a list
  row. Use the row's `IdGen` in `render.rs`.
- The first release build compiles GPUI and takes several minutes. Later
  builds take about 1 minute, because of full LTO.
