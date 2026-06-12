# MarkForge

A **native macOS Markdown viewer** (with an editor on the roadmap), built in Rust
on [GPUI](https://github.com/zed-industries/zed) — Zed's GPU-accelerated UI
framework — and the [gpui-component](https://github.com/longbridge/gpui-component)
widget library.

No web view, no Electron. The Markdown is rendered by the same engine that powers
Zed's own preview.

## Features

- **Native & fast** — GPU-accelerated rendering, instant startup.
- **File explorer** — VSCode-style sidebar (⌘B). Open a folder (⌘⇧O), browse the
  tree, click any file to open it.
- **Viewer + editor** — read full-width, or ⌘E into a resizable split with a
  syntax-highlighted Markdown source editor and live preview.
- **JSON too** — open `.json`/`.jsonc` files: minified JSON is pretty-printed,
  the editor switches to JSON highlighting, and the preview shows a
  highlighted code view.
- **docx · hwpx · pdf** — read-only import: Word/HWPX documents convert to
  Markdown (headings, emphasis, lists, tables), PDFs to extracted text.
  ⌘S saves a Markdown copy — originals are never overwritten.
- **Live reload** — edit the file in any other app and MarkForge updates it
  automatically (mtime-polled in the background); debounced so typing stays smooth.
- **Zoom** — ⌘+/⌘-/⌘0 scales both editor and preview (body, headings, code).
- **Theme** — light (default), dark, or follow the macOS system appearance.
- **Recent files** — File ▸ Open Recent.
- **Persisted settings** — theme, zoom, and recent files are remembered across launches.
- **Open anywhere** — file dialog, drag-and-drop, or a command-line argument.

**Website:** https://rescenedev.github.io/markforge/

## Install

```bash
# Homebrew (recommended)
brew install --cask rescenedev/tap/markforge
```

Or grab `MarkForge.app` from the [latest release](https://github.com/rescenedev/markforge/releases/latest).

## Usage

```bash
# Run with the bundled sample document
cargo run --release

# Open a specific file
cargo run --release -- path/to/notes.md

# Build a distributable macOS .app bundle (→ target/release/MarkForge.app)
./scripts/bundle.sh
```

### Shortcuts

| Action               | Key      |
| -------------------- | -------- |
| Open file            | ⌘O       |
| Open folder          | ⌘⇧O      |
| Toggle sidebar       | ⌘B       |
| Save                 | ⌘S       |
| Settings             | ⌘,       |
| Toggle editor        | ⌘E       |
| Zoom in / out / reset| ⌘+ / ⌘- / ⌘0 |
| Reload               | ⌘R       |
| Toggle light/dark    | ⌘⇧L      |
| Navigate file tree   | ↑ ↓ (move) · ← → (collapse/expand) · ⏎ (open) |
| Close window         | ⌘W       |
| Quit                 | ⌘Q       |

You can also **drag a `.md` file onto the window** to open it (or drag a
folder to open it in the sidebar). Closing, quitting, opening another file, or
reloading with unsaved edits asks to save first.

Settings live at `~/Library/Application Support/MarkForge/settings.json` —
⌘, opens that file *in MarkForge itself*; save (⌘S) and the changes apply
instantly. Keys include `theme`, `zoom`, `body_font_size`, `preview_font`,
`editor_font`, `syntax_theme`, `sidebar_open`, `preview_padding`, and
`sidebar_bg_dark`.

## Building

Requires a recent stable Rust toolchain (edition 2024 → Rust ≥ 1.85) and the
Xcode Command Line Tools.

```bash
cargo build --release
```

> The first build compiles GPUI and a large dependency graph from source, so it
> takes a while. Subsequent builds are incremental and fast.

## Roadmap

- [x] Markdown viewer
- [x] Live reload
- [x] Drag-and-drop + file dialog
- [x] Split-pane **editor** with live preview
- [x] Syntax-highlighted source editing
- [x] Content zoom
- [x] Light / dark / system theme + persisted settings
- [x] Recent files
- [x] File-explorer sidebar (open folder, tree navigation)
- [ ] Tabs / multiple open documents
- [ ] `.app` bundle + Finder file association
- [ ] Outline (table of contents) sidebar
- [ ] Export to PDF / HTML

## Project layout

```
src/
  main.rs     # app bootstrap: actions, menus, key bindings, window
  app.rs      # MarkForge view: title bar, sidebar, editor/preview, zoom, watcher
  file_tree.rs# file-explorer model (open folder, expansion, listings)
  rem_scaled.rs # element wrapper that scales `rem` for the editor zoom
  settings.rs # persisted settings (theme, zoom, fonts, recent files)
assets/
  sample.md   # bundled showcase document
```

## License

MIT.
