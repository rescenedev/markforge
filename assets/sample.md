# MarkForge

A **native macOS Markdown viewer** built with [GPUI](https://github.com/zed-industries/zed) and [gpui-component](https://github.com/longbridge/gpui-component) — the same rendering stack that powers the Zed editor.

> Open a `.md` file with **⌘O**, or just drag one onto the window.
> Edit it in any other app and MarkForge live-reloads it for you.

---

## Features

- 🖥️ **Native** — GPU-accelerated, no web view, no Electron.
- 👀 **Viewer first** — clean, readable typography out of the box.
- ♻️ **Live reload** — external edits show up automatically.
- 🌗 **Light & dark** — toggle with **⌘⇧L** (or the title-bar button).
- 🧩 **Editor next** — a split-pane editor is on the roadmap.

## Typography

You get all the usual inline styling: *italic*, **bold**, ***bold italic***,
`inline code`, ~~strikethrough~~, and [links](https://example.com).

### Lists

1. Ordered items
2. Nested content
   - Unordered child
   - Another child
3. Back to the top level

- [x] Render Markdown
- [x] Live reload
- [ ] In-app editor

## Code

```rust
fn main() {
    println!("Hello from MarkForge!");
}
```

```python
def greet(name: str) -> str:
    return f"Hello, {name}!"
```

## Tables

| Feature      | Viewer | Editor |
| ------------ | :----: | :----: |
| Render       |   ✅   |   ✅   |
| Live reload  |   ✅   |   🚧   |
| Syntax HL    |   ✅   |   🚧   |

## Quote

> "The best way to predict the future is to invent it."
> — Alan Kay

---

Press **⌘O** to open your own Markdown file and start reading.
