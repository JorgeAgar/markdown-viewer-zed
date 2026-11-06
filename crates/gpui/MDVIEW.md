# Vendored GPUI 0.2.2 — mdview changes

This is `gpui` 0.2.2 from crates.io (Apache-2.0, © Zed Industries). mdview
uses it through `[patch.crates-io]` in the top-level `Cargo.toml`. The full
diff against upstream is in `patches/gpui-0.2.2-startup.patch`. Every change is
marked in the code with a `// mdview:` comment.

## 1. Don't build the render pipelines twice

`platform/linux/wayland/window.rs`, `platform.rs`, `window.rs`,
`platform/mac/window.rs`

Upstream always creates the Wayland renderer with `transparent: true`. While
the window is being set up, `request_decorations` / `set_background_appearance`
call `update_window`. For an opaque window with server-side decorations, that
switches the surface to opaque, and `BladeRenderer::update_transparency` throws
every pipeline away and compiles them all again.

`WindowParams` now carries `window_decorations` and `window_background`, so the
renderer starts with the transparency it will end up with.

## 2. Compile the pipelines in parallel

`platform/blade/blade_renderer.rs`

`BladePipelines::new` builds its 8 render pipelines on scoped threads.
Pipeline creation in Vulkan is thread-safe, and it's the most expensive part of
opening a window when the shader cache is cold (and every time on software
rasterizers).

## 3. Overlap Vulkan init with font loading

`platform/linux/wayland/client.rs`, `platform/linux/x11/client.rs`

`BladeContext::new()` (Vulkan instance and device) runs on its own thread from
the start of client construction. Meanwhile the main thread connects to the
display and builds the font database (`cosmic_text::FontSystem::new()` scans
every system font). The two are joined where the context used to be created.

## 4. Build fix: typed float literals

`taffy.rs`

`to_grid_repeat` passed `0.0` and `1.0` where the type is only inferred
through `From<f32>`. rustc warns that this fallback to `f32` is being
phased out and will become a hard error
([#154024](https://github.com/rust-lang/rust/issues/154024)), so the
literals are now `0.0_f32` and `1.0_f32`. No behavior change.

## 5. X11: don't sleep on events libxcb already queued

`platform/linux/x11/client.rs`

The X11 connection is a level-triggered fd source in the calloop event loop,
so `process_x11_events` runs only when the socket is readable. But any reply
wait on the connection (GPUI's own requests, or Mesa's Vulkan WSI, which
shares the `xcb_connection_t`) reads pending events off the socket into
libxcb's internal queue. When `MapNotify` landed there while the first frame
was being drawn, the socket stayed empty, the refresh loop (started by
`MapNotify`) never started, and the window never presented: `epoll_wait`
blocked until some unrelated event arrived. That reliably hung the 8 MB
end-to-end test under XWayland
([#12](https://github.com/JorgeAgar/markdown-viewer-zed/issues/12)).

`X11Client::run` now drains queued events in calloop's per-iteration
callback, which runs after the sources and idle callbacks and right before
the loop blocks again. When the queue is empty that's one non-blocking read.

## Measurements

Time from `main()` to the first frame submitted and presented. Medians;
release build, Intel Iris Xe, niri.

| Vulkan driver        | upstream | with these changes |
| -------------------- | -------: | -----------------: |
| Intel (ANV)          |  ~105 ms |             ~89 ms |
| lavapipe (software)  |  ~159 ms |            ~103 ms |

Breakdown with all three changes on ANV:

| Phase                                      | Time      |
| ------------------------------------------ | --------- |
| Font database, overlapped with Vulkan init | ~16 ms    |
| Remaining wait for Vulkan                  | ~5–7 ms   |
| Surface + pipelines                        | ~9–13 ms  |
| First layout (text shaping + glyphs)       | ~20–25 ms |
| First GPU frame acquire + present          | ~25 ms    |
