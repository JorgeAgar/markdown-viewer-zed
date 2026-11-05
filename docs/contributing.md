# Contributing

This guide is for people and AI agents. Agents: also read
[`AGENTS.md`](../AGENTS.md).

## Setup

You need:

- **Rust**, recent stable (the code uses edition 2024 and `let` chains).
- **Linux system libraries** for GPUI: Wayland and/or X11 client libraries,
  `libxkbcommon`, `fontconfig`, `freetype2`, and a Vulkan loader. Arch:
  `sudo pacman -S wayland libxkbcommon libxcb fontconfig freetype2 vulkan-icd-loader`.
- **A Vulkan driver** for your GPU: `vulkan-intel`, `vulkan-radeon`, or the
  NVIDIA driver. Without one, mdview falls back to lavapipe (software
  rendering). It works, but it's slow.
- **A desktop file portal** (`xdg-desktop-portal` plus a backend such as
  `-gtk` or `-gnome`). The `Ctrl+O` file picker needs it.

Windows builds need the MSVC Rust toolchain, Visual Studio C++ build tools and
the Windows SDK (including `fxc.exe`, used to compile GPUI shaders at build time).
Build with `cargo build --locked --release`. Run `cargo test --locked --workspace
--profile ci` and clippy natively; Linux retains the dependency/size guards.
Inno Setup 6 builds the installer from `packaging/windows/mdview.iss`.
Native CI exercises installation, upgrade and uninstallation. Actual GPU/UI
and startup validation still requires a Windows desktop.

## Build, run, test

```sh
cargo build --release                 # full LTO, ~1 min for mdview itself; the first build compiles GPUI (several minutes)
./target/release/mdview examples/demo.md examples/second.md
./target/release/mdview -n FILE       # separate instance (doesn't use or create the socket)
scripts/check.sh                      # fmt, guards, clippy and every test; must pass before each commit
cargo test -p mdview-core             # just the parser tests (seconds, no GPUI)
cargo install --path .                # installs `mdview` into ~/.cargo/bin
```

`MDVIEW_TRACE_STARTUP=1` prints the time to the first frame. See
[startup.md](startup.md). What the tests cover, and how to update snapshots
and golden images after an intended change, is in [testing.md](testing.md).
Run `scripts/install-hooks.sh` once to run `scripts/check.sh --quick` before
every commit.

### Manual test checklist

Run this before you call a UI change done:

- [ ] `examples/demo.md` renders correctly in **dark and light** mode
- [ ] Tabs: switch with a click and with `Ctrl+Tab`; close with `×`, middle
      click and `Ctrl+W`
- [ ] Running `mdview other.md` while a window is open adds a tab, and the
      command exits immediately
- [ ] Links: `#anchor` scrolls, the link to `second.md` opens a tab, `https://`
      opens the browser
- [ ] Editing an open file and saving reloads it, keeping the scroll position
- [ ] Startup time has not regressed (`MDVIEW_TRACE_STARTUP=1`, take the
      median of 5 runs, compare with `main`)

## Performance rules

Startup is the product, so these rules come first:

1. **Nothing slow before the socket check in `main()`.** The warm path must
   stay a few milliseconds.
2. **No new dependencies without a measured reason.** Each new crate adds build
   time and binary size, and usually some startup cost too. Explain the
   tradeoff in the PR.
3. **No blocking I/O or subprocesses at startup.** For example, no
   `fc-match`, no D-Bus calls on the critical path, and no network.
4. **Parse once, render many times.** Expensive work belongs in
   `markdown::parse` (which runs once per load), not in `render_block` (which
   runs every frame for the visible rows). Work that's too slow even for
   parsing (typesetting math) goes on the background executor and into a
   cache, like `graphics.rs` does.
5. **Back up performance claims with numbers.** Use the method in
   [startup.md](startup.md), compare medians, and put the numbers in the
   commit message.

## Code conventions

- `cargo fmt` before committing. Fix any new `cargo clippy` warnings in
  `src/`.
- Keep the module boundaries described in
  [architecture.md](architecture.md). In particular, `crates/mdview-core`
  (`markdown.rs`, `highlight.rs`, `latex.rs`) must not depend on GPUI.
- Comments explain **why**, not what.
- Changes to `crates/gpui` follow [gpui-fork.md](gpui-fork.md): a
  `// mdview:` comment, a regenerated patch, and updated `MDVIEW.md`.
- When you change behavior, update the docs in the same commit (the README
  key table, `docs/`, the diagrams).

## Git workflow

- **Commit every change.** Each logical change gets its own commit. Don't
  leave work uncommitted.
- Commit messages: an imperative subject (≤ 72 characters), a blank line, then
  a body that explains *why* and includes any measurements.
- Work on a branch, not directly on `main`. Open a pull request against
  `main`.
- Never commit the temporary profiling marks (`tmark`) or anything under
  `target/`.
