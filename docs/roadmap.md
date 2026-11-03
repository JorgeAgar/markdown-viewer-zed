# Roadmap

Ideas and known gaps, roughly ordered by value. Before you start something big,
open an issue (or ask the maintainer) so the scope gets agreed first.

## Integration

- [ ] **`.desktop` file + MIME association**, so `.md` files can be opened from
      file managers with a double click. Add an install target or a small
      script.
- [ ] **App icon.** Install `assets/icon.svg` under the `mdview` app id.
- [ ] **Packaging:** an AUR `PKGBUILD`, maybe Flatpak.

## Rendering

- [ ] **Text selection and copy.** GPUI has selection support in Zed's editor
      crates, but not in plain `StyledText`. This needs a custom element or a
      selectable-text component.
- [ ] **Syntax highlighting in code blocks.** It has to be cheap: no
      tree-sitter grammars bundled by default. Options: `syntect` with a
      minimal set of syntaxes, loaded lazily on first use; or
      highlight on a background thread and patch the result in. Must not
      affect startup.
- [ ] **Remote images.** This needs an HTTP client (GPUI has a pluggable
      `HttpClient`). Load lazily and never block startup.
- [ ] **Raw HTML.** At least `<img>`, `<br>`, `<details>`, and
      `<p align="center">`, which are common in READMEs.
- [ ] **Mermaid diagrams**, so these docs render in mdview too.
- [ ] **Math** (`$…$`). It's parsed today but shown as code.
- [ ] Split very large blocks (huge tables or code) into several list rows, so
      virtualization still helps.
- [ ] Find in page (`Ctrl+F`).

## UX

- [ ] Show a scrollbar, or at least a scroll position indicator.
- [ ] Zoom (`Ctrl+=` / `Ctrl+-`) for the content font size.
- [ ] `Alt+1…9` to jump to a tab; reorder tabs by dragging.
- [ ] Remember open tabs between sessions (optional, and must not slow down
      startup).
- [ ] A table of contents / outline sidebar, toggled with a key.

## Startup

See [startup.md](startup.md) for the current breakdown.

- [ ] **Warm up glyphs while Vulkan initializes.** The main thread waits about
      5–7 ms for the GPU thread. It could load the main fonts and rasterize
      common ASCII glyphs in that time, shortening the first layout (~22 ms).
- [ ] **Load only the needed fonts at startup.** Scanning all ~670 system
      font faces takes ~16 ms. Loading only the chosen families takes 0.2 ms
      (measured). The full database could load on a background thread for
      fallback characters (emoji, CJK).
- [ ] **Figure out the ~25 ms first-present gap.** It's probably waiting on
      the compositor's configure event or for a swapchain image. Check with
      timing marks in `BladeRenderer::draw` around `acquire_frame`.
- [ ] Send the GPUI fixes upstream to Zed (fix 1 is a plain bug).
- [ ] A "daemon" mode: keep an instance running with no window, so the first
      window opens almost instantly.

## Done

- [x] Tabs, drag & drop, the `Ctrl+O` picker, live reload, links between
      documents, anchors
- [x] Single instance over a Unix socket (~4 ms warm path)
- [x] GPUI startup fixes: no double pipeline build, parallel pipelines,
      Vulkan init in parallel with fonts
