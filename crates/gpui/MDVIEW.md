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

## 5. Initial directory for the open-file dialog

`platform.rs`, `platform/linux/platform.rs`, `platform/mac/platform.rs`,
`platform/windows/platform.rs`

Upstream `PathPromptOptions` has no field for a starting directory, so
`prompt_for_paths` leaves the OS file picker to choose its own default folder
(often `/` or `~` on Linux portals). `PathPromptOptions` now carries
`directory: Option<PathBuf>`, wired to `OpenFileRequest::current_folder` on
Linux, `NSOpenPanel::setDirectoryURL` on macOS, and `IFileOpenDialog::SetFolder`
on Windows (matching `prompt_for_new_path`).

## 6. Hooks for smooth and kinetic scrolling

`platform/linux/wayland/client.rs`, `elements/list.rs`

Wayland leaves kinetic (fling) scrolling to clients, but upstream ignores
`wl_pointer.axis_stop`, so an app can't tell when the fingers left the
touchpad. A finger `axis_stop` on the vertical axis is now sent, after the
frame's other scroll events, as a `ScrollWheelEvent` with a zero `Pixels`
delta and `TouchPhase::Ended`. A horizontal stop is ignored: in a diagonal
swipe it can come while the fingers still scroll vertically.

`List::without_scroll_wheel()` skips the list's own wheel listener, so a view
can animate the scroll itself with `ListState::scroll_by`.
`ListState::measured_before(n)` returns how many of the first `n` items are
measured and their height, so a scrollbar can estimate the rest. No startup
cost: nothing runs until a scroll event or a scrollbar is drawn.

## 7. Clamp a stale offset into the scroll-top item

`elements/list.rs`

`ListState::scroll_to` keeps any `offset_in_item`, and `layout_items` drew the
item at the scroll top shifted up by it, even when the item turned out
shorter than the offset: the item was then above the viewport although it
was still the scroll top. That happens when an offset is restored after a
`reset` (mdview reloading a file whose paragraph at the top got shorter), or
after the item shrank (a width change). When the scroll-top item is measured
and the offset is at or past its end, `layout_items` now starts at the item's
top. `scroll_to_reveal_item` also seeks with `Bias::Right` (matching
`StateInner::scroll`) so exact row boundaries normalize to the next item at
offset 0 instead of the previous item at `offset == height`. Offsets reached by
scrolling are always inside their item, so they're unaffected. Tested in
mdview's `src/viewer/app_tests.rs`
(`list_starts_at_the_top_of_an_item_shorter_than_the_offset`).

## 8. Take an asset out of the cache

`app.rs`

`App::remove_asset` drops a cached asset without returning it, so an image
removed from the cache (to read a changed file again) kept its texture in
the sprite atlas. `App::take_asset` removes it and returns it if it had
finished loading, without starting a load the way `fetch_asset` would, so
the caller can free the texture with `App::drop_image`.

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
