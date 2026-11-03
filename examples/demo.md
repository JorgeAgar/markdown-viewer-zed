# mdview demo

A **tiny** Markdown viewer, rendered on the GPU with *GPUI* — the same UI framework as [Zed](https://zed.dev). It supports `inline code`, ~~strikethrough~~, and [internal links](#tables).

## Lists

- First item
- Second item with **bold** text
  - Nested item
  - Another nested one
- [x] Done task
- [ ] Pending task

1. One
2. Two
3. Three

## Quotes and callouts

> Simplicity is prerequisite for reliability.
> — Edsger W. Dijkstra

> [!TIP]
> Open several files at once: `mdview a.md b.md`. Running `mdview` again adds tabs to the existing window.

> [!WARNING]
> This is a prototype.

## Code

```rust
fn main() {
    println!("Hello from mdview!");
}
```

## Tables

| Feature        | Status | Notes                    |
|----------------|:------:|-------------------------:|
| Tabs           |   ✓    | Ctrl+Tab to switch       |
| Live reload    |   ✓    | Watches file mtime       |
| Single instance|   ✓    | Unix socket, like Zed    |

---

Footnote reference[^1] and a [link to the other demo](second.md).

[^1]: Footnotes render at the end.
