//! The MarkForge main view.
//!
//! A `gpui-component` [`InputState`] (in Markdown code-editor mode) is the source
//! of truth for the document text. The preview renders a *cached* copy of that
//! text, refreshed on a short debounce so typing stays smooth on large files.
//!
//! Layouts: **Preview** (full-width rendered Markdown) and **Editor** (⌘E — a
//! resizable split with the source on the left and live preview on the right).
//! Content zoom (⌘+/⌘-/⌘0) scales both panes. Theme preference, zoom, and the
//! recent-files list are persisted via [`Settings`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gpui::prelude::*;
use gpui::{
    App, ExternalPaths, FocusHandle, Focusable, PathPromptOptions, PromptLevel, SharedString,
    StyleRefinement, Subscription, Task, Window, WindowAppearance, div, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, Theme, ThemeMode, TitleBar,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    highlighter::{HighlightTheme, Language},
    input::{Input, InputEvent, InputState},
    notification::NotificationType,
    resizable::{h_resizable, resizable_panel},
    text::{TextViewStyle, markdown},
    v_flex,
};

use crate::file_tree::FileTree;
use crate::import::{is_imported_doc, read_document};
use crate::rem_scaled::RemScaled;
use crate::settings::{FONT_SIZE_MAX, FONT_SIZE_MIN, Settings, ThemePref};
use crate::{
    CloseWindow, FontDec, FontInc, OpenFile, OpenFolder, OpenRecent, Quit, Reload, Save,
    SetSyntaxTheme, SetTheme, ToggleEdit, ToggleSettings, ToggleSidebar, ToggleTheme, ZoomIn,
    ZoomOut, ZoomReset,
};

/// Bundled showcase document, displayed on first launch.
const SAMPLE: &str = include_str!("../assets/sample.md");

const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.6;
const ZOOM_MAX: f32 = 2.6;

/// Step (px) for the base font-size control in Settings.
const FONT_STEP: f32 = 1.0;

/// How long to wait after the last keystroke before re-parsing the preview.
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);

/// Above this many bytes of document text, the *editor* skips syntax
/// highlighting — tokenizing megabytes of JSON on every edit is too slow.
const HIGHLIGHT_MAX_BYTES: usize = 1024 * 1024;

/// The preview renders at most this many bytes of a non-Markdown document.
/// The markdown TextView lays the whole code block out at once (no
/// virtualization), so a multi-megabyte document beachballs the UI;
/// a capped preview stays instant and the full text lives in the editor.
const PREVIEW_MAX_BYTES: usize = 64 * 1024;

/// Per-file size cap for background preloading (on-disk bytes).
const PRELOAD_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Total text budget for one directory preload pass.
const PRELOAD_BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// Hard cap on cached documents (stale entries self-heal via mtime check).
const DOC_CACHE_MAX_ENTRIES: usize = 256;

/// A document preloaded (read + pretty-printed) and ready to apply instantly.
struct CachedDoc {
    modified: Option<SystemTime>,
    text: SharedString,
}

/// What kind of document the buffer holds, driving highlighter and preview.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DocKind {
    Markdown,
    /// `highlight` is false for very large documents (see [`HIGHLIGHT_MAX_BYTES`]).
    Json { highlight: bool },
}

impl DocKind {
    fn editor_language(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Json { highlight: true } => "json",
            Self::Json { highlight: false } => "text",
        }
    }

    fn for_path(path: &std::path::Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("json" | "jsonc") => Self::Json { highlight: true },
            _ => Self::Markdown,
        }
    }
}

/// Decision from a single file-watch poll.
enum WatchOutcome {
    /// The file changed on disk and a reload should proceed.
    Reload,
    Noop,
    Stop,
}

pub struct MarkForge {
    focus_handle: FocusHandle,
    /// The document buffer (Markdown source). Source of truth for the text.
    input_state: gpui::Entity<InputState>,
    /// Debounced snapshot of the buffer used to render the preview. For
    /// non-Markdown documents this holds the fenced-code-block wrapped text.
    preview_text: SharedString,
    /// Kind of the current document (drives highlighter and preview treatment).
    doc_kind: DocKind,
    /// Path the current document was loaded from, if any.
    file_path: Option<PathBuf>,
    /// Last observed modification time, used to detect external edits.
    last_modified: Option<SystemTime>,
    /// Whether the split editor is shown.
    editing: bool,
    /// Whether the buffer has unsaved edits.
    dirty: bool,
    /// Converted document (docx/hwpx/pdf) — read-only; ⌘S saves a Markdown copy.
    imported: bool,
    /// Content zoom factor (1.0 == 100%).
    zoom: f32,
    /// File-explorer model (opened folder, expansion, listings).
    file_tree: FileTree,
    /// Whether the left sidebar (file explorer) is shown.
    sidebar_open: bool,
    /// Whether the Settings panel is open.
    show_settings: bool,
    /// Text field for the editor (monospace) font family.
    editor_font_input: gpui::Entity<InputState>,
    /// Text field for the preview font family.
    preview_font_input: gpui::Entity<InputState>,
    /// Debounce task that refreshes `preview_text`.
    preview_task: Option<Task<()>>,
    /// Background poller that live-reloads the current file.
    watch_task: Option<Task<()>>,
    /// Background-preloaded documents (read + pretty-printed), keyed by path.
    doc_cache: HashMap<PathBuf, CachedDoc>,
    /// Monotonic token so a stale async load can't clobber a newer one.
    load_seq: u64,
    _subscriptions: Vec<Subscription>,
}

impl MarkForge {
    pub fn new(initial: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<Settings>().clone();

        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(Language::Markdown)
                .line_number(true)
                .soft_wrap(true)
                .placeholder("Write Markdown here…")
        });

        let input_sub = cx.subscribe(&input_state, Self::on_input_event);

        // Font-family text fields for the Settings panel.
        let editor_font_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Editor font (e.g. Menlo) — empty for default")
                .default_value(settings.editor_font.clone())
        });
        let preview_font_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Preview font — empty for default")
                .default_value(settings.preview_font.clone())
        });

        let editor_font_sub =
            cx.subscribe(&editor_font_input, |_this, state, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.global_mut::<Settings>().editor_font = state.read(cx).value().to_string();
                    save_settings(cx);
                    cx.notify();
                }
            });
        let preview_font_sub =
            cx.subscribe(&preview_font_input, |_this, state, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.global_mut::<Settings>().preview_font = state.read(cx).value().to_string();
                    save_settings(cx);
                    cx.notify();
                }
            });

        // Apply the persisted theme preference (System resolves to the OS
        // appearance) and keep following the OS while in System mode.
        apply_theme(settings.theme, window, cx);
        apply_syntax_theme(cx);
        let appearance_sub = window.observe_window_appearance(|window, cx| {
            if cx.global::<Settings>().theme == ThemePref::System {
                apply_theme(ThemePref::System, window, cx);
                apply_syntax_theme(cx);
            }
        });

        // Intercept the red traffic-light close so unsaved edits get a prompt.
        // Returning `false` keeps the window; the guard re-closes it on confirm.
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(entity) = weak.upgrade() else {
                return true;
            };
            if !entity.read(cx).dirty {
                return true;
            }
            entity.update(cx, |this, cx| {
                this.guard_unsaved(window, cx, |_, window, _| window.remove_window());
            });
            false
        });

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            input_state,
            preview_text: SharedString::default(),
            doc_kind: DocKind::Markdown,
            file_path: None,
            last_modified: None,
            editing: false,
            dirty: false,
            imported: false,
            zoom: settings.zoom.clamp(ZOOM_MIN, ZOOM_MAX),
            file_tree: FileTree::new(),
            sidebar_open: settings.sidebar_open,
            show_settings: false,
            editor_font_input,
            preview_font_input,
            preview_task: None,
            watch_task: None,
            doc_cache: HashMap::new(),
            load_seq: 0,
            _subscriptions: vec![input_sub, editor_font_sub, preview_font_sub, appearance_sub],
        };

        match initial {
            // A directory argument opens the file-explorer sidebar.
            Some(path) if path.is_dir() => {
                this.open_folder(path, cx);
                this.restore_last_document(window, cx);
            }
            Some(path) => this.load_path(path, window, cx),
            None => this.restore_last_document(window, cx),
        }

        this
    }

    /// Reopen the most recently viewed file; the bundled sample document only
    /// greets a true first launch (empty recents).
    fn restore_last_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let last = cx
            .global::<Settings>()
            .recent
            .iter()
            .find(|p| p.is_file())
            .cloned();
        match last {
            Some(path) => self.load_path(path, window, cx),
            None => self.set_editor_text(SAMPLE, window, cx),
        }
    }

    /// React to edits: keep the editor responsive immediately, refresh the
    /// (expensive) preview on a debounce.
    fn on_input_event(
        &mut self,
        _state: gpui::Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.dirty = true;
            cx.notify(); // editor repaints now; preview keeps its cached text

            self.preview_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
                let _ = this.update(cx, |this, cx| {
                    let latest = this.wrap_preview(this.input_state.read(cx).value());
                    if this.preview_text != latest {
                        this.preview_text = latest;
                        cx.notify();
                    }
                });
            }));
        }
    }

    /// Replace the buffer text without emitting change events (programmatic load).
    fn set_editor_text(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut text = text.into();
        if matches!(self.doc_kind, DocKind::Json { .. }) {
            if let Some(pretty) = prettify_minified_json(&text) {
                text = pretty.into();
            }
            // Re-decide editor highlighting on the final (pretty-printed) size.
            let highlight = text.len() <= HIGHLIGHT_MAX_BYTES;
            self.set_doc_kind(DocKind::Json { highlight }, cx);
        }
        self.input_state
            .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
        self.preview_text = self.wrap_preview(text);
    }

    /// Markdown renders as-is; JSON is wrapped in a fenced code block so the
    /// preview shows a highlighted code view. Documents larger than
    /// [`PREVIEW_MAX_BYTES`] are truncated in the preview (the editor has the
    /// full text) so opening huge files stays fast.
    fn wrap_preview(&self, text: SharedString) -> SharedString {
        match self.doc_kind {
            DocKind::Markdown => text,
            DocKind::Json { .. } => {
                let cut = preview_cut(&text, PREVIEW_MAX_BYTES);
                // Four backticks so content containing ``` can't break the fence.
                if cut < text.len() {
                    let total_kb = text.len() / 1024;
                    format!(
                        "````json\n{}\n…\n````\n\n> ⚠️ **Preview truncated** — \
                         showing the first {} KB of {} KB. \
                         Open the editor (⌘E) for the full document.",
                        &text[..cut],
                        cut / 1024,
                        total_kb,
                    )
                    .into()
                } else {
                    format!("````json\n{text}\n````").into()
                }
            }
        }
    }

    /// Switch the document kind, updating the editor highlighter to match.
    fn set_doc_kind(&mut self, kind: DocKind, cx: &mut Context<Self>) {
        if self.doc_kind != kind {
            self.doc_kind = kind;
            self.input_state.update(cx, |state, cx| {
                state.set_highlighter(kind.editor_language(), cx)
            });
        }
    }

    /// The authoritative buffer contents (not the debounced snapshot).
    fn text(&self, cx: &App) -> SharedString {
        self.input_state.read(cx).value()
    }

    /// Load a file into the document. Preloaded (cached) files apply
    /// instantly; everything else is read + pretty-printed on the background
    /// executor so the UI thread never stalls on a large document.
    fn load_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.load_seq += 1;
        let seq = self.load_seq;

        // Cache hit with a fresh mtime → no disk read, no prettify, no wait.
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified.is_some() {
            if let Some(doc) = self.doc_cache.get(&path) {
                if doc.modified == modified {
                    let text = doc.text.clone();
                    self.apply_loaded(path, modified, text, window, cx);
                    return;
                }
            }
        }

        let read = {
            let path = path.clone();
            cx.background_executor().spawn(async move {
                let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                read_document(&path).map(|text| (modified, prepare_doc_text(&path, text)))
            })
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.load_seq != seq {
                    return; // a newer load superseded this one
                }
                match result {
                    Ok((modified, text)) => {
                        this.apply_loaded(path.clone(), modified, text.into(), window, cx)
                    }
                    Err(err) => this.show_load_error(path.clone(), &err, window, cx),
                }
            });
        })
        .detach();
    }

    /// Install loaded document text (already pretty-printed) as the current file.
    fn apply_loaded(
        &mut self,
        path: PathBuf,
        modified: Option<SystemTime>,
        text: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.last_modified = modified;
        self.set_doc_kind(DocKind::for_path(&path), cx);
        self.set_editor_text(text.clone(), window, cx);
        self.file_path = Some(path.clone());
        self.dirty = false;
        self.imported = is_imported_doc(&path);
        self.start_watch(path.clone(), window, cx);

        if self.doc_cache.len() < DOC_CACHE_MAX_ENTRIES {
            self.doc_cache.insert(path.clone(), CachedDoc { modified, text });
        }

        // Populate the sidebar with the file's folder if none is open.
        if !self.file_tree.is_open() {
            if let Some(parent) = path.parent() {
                self.file_tree.open(parent.to_path_buf());
                self.preload_dir(parent.to_path_buf(), cx);
            }
        }

        cx.global_mut::<Settings>().push_recent(path);
        save_settings(cx);
        crate::set_menus(cx);
        cx.notify();
    }

    fn show_load_error(
        &mut self,
        path: PathBuf,
        err: &std::io::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The error document is Markdown regardless of the target file.
        self.set_doc_kind(DocKind::Markdown, cx);
        self.set_editor_text(
            format!("# Couldn't open file\n\n`{}`\n\n```\n{err}\n```", path.display()),
            window,
            cx,
        );
        // Drop unreadable entries from Open Recent so they don't linger.
        cx.global_mut::<Settings>().recent.retain(|p| p != &path);
        save_settings(cx);
        crate::set_menus(cx);
        self.file_path = Some(path);
        self.last_modified = None;
        self.dirty = false;
        // Read-only semantics: never let ⌘S overwrite the unreadable original.
        self.imported = true;
        self.watch_task = None;
        cx.notify();
    }

    /// Read + pretty-print every supported document in `dir` on the
    /// background executor and stash the results in the cache, so clicking a
    /// file in the tree applies instantly.
    fn preload_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let read_all = cx.background_executor().spawn(async move {
            let mut docs: Vec<(PathBuf, Option<SystemTime>, String)> = Vec::new();
            let mut budget = PRELOAD_BUDGET_BYTES;
            let Ok(rd) = std::fs::read_dir(&dir) else {
                return docs;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || !is_supported_doc(&path) {
                    continue;
                }
                // PDFs convert on demand only — extraction is too costly to
                // burn on files that may never be clicked.
                if path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                if !meta.is_file() || meta.len() > PRELOAD_MAX_FILE_BYTES {
                    continue;
                }
                let Ok(text) = read_document(&path) else {
                    continue;
                };
                let modified = meta.modified().ok();
                let text = prepare_doc_text(&path, text);
                if text.len() > budget {
                    break;
                }
                budget -= text.len();
                docs.push((path, modified, text));
            }
            docs
        });

        cx.spawn(async move |this, cx| {
            let docs = read_all.await;
            let _ = this.update(cx, |this, _| {
                for (path, modified, text) in docs {
                    if this.doc_cache.len() >= DOC_CACHE_MAX_ENTRIES {
                        break;
                    }
                    this.doc_cache
                        .insert(path, CachedDoc { modified, text: text.into() });
                }
            });
        })
        .detach();
    }

    /// Spawn a background task that re-reads `path` whenever its mtime changes —
    /// but only while the user isn't actively editing unsaved changes.
    /// Stat and read both run on the background executor so a large file never
    /// stalls the UI thread.
    fn start_watch(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(700))
                    .await;

                let modified = {
                    let path = path.clone();
                    cx.background_executor()
                        .spawn(async move {
                            std::fs::metadata(&path).and_then(|m| m.modified()).ok()
                        })
                        .await
                };
                let Some(modified) = modified else { continue };

                // Cheap main-thread check: decide whether a reload is needed.
                let outcome = this.update(cx, |this, _| {
                    if this.file_path.as_deref() != Some(path.as_path()) {
                        WatchOutcome::Stop
                    } else if this.last_modified == Some(modified) {
                        WatchOutcome::Noop
                    } else if this.dirty {
                        // Don't clobber in-progress edits; just remember the new mtime.
                        this.last_modified = Some(modified);
                        WatchOutcome::Noop
                    } else {
                        WatchOutcome::Reload
                    }
                });
                match outcome {
                    Ok(WatchOutcome::Reload) => {}
                    Ok(WatchOutcome::Noop) => continue,
                    Ok(WatchOutcome::Stop) | Err(_) => break,
                }

                let text = {
                    let path = path.clone();
                    cx.background_executor()
                        .spawn(async move { read_document(&path) })
                        .await
                };
                let Ok(text) = text else { continue };

                // Re-check before applying: the user may have started editing
                // or switched files while the read was in flight.
                let applied = this.update_in(cx, |this, window, cx| {
                    if this.file_path.as_deref() == Some(path.as_path()) && !this.dirty {
                        this.set_editor_text(text, window, cx);
                        this.last_modified = Some(modified);
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        });

        self.watch_task = Some(task);
    }

    /// Run `then` immediately when the buffer is clean; otherwise show the
    /// standard macOS Save / Don't Save / Cancel sheet first. `then` only runs
    /// if the document was saved successfully or the user chose to discard.
    fn guard_unsaved(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.dirty {
            then(self, window, cx);
            return;
        }

        // "Save" needs a writable destination; untitled buffers and imported
        // (docx/hwpx/pdf) documents only get Discard/Cancel.
        let can_save = self.file_path.is_some() && !self.imported;
        let answers: &[&str] = if can_save {
            &["Save", "Don't Save", "Cancel"]
        } else {
            &["Discard Changes", "Cancel"]
        };
        let name = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "this document".to_string());

        let rx = window.prompt(
            PromptLevel::Warning,
            &format!("Do you want to save the changes made to {name}?"),
            Some("Your changes will be lost if you don't save them."),
            answers,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = rx.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                let proceed = if can_save {
                    match answer {
                        0 => this.save_in_place(window, cx),
                        1 => true,
                        _ => false,
                    }
                } else {
                    answer == 0
                };
                if proceed {
                    this.dirty = false;
                    then(this, window, cx);
                }
            });
        })
        .detach();
    }

    /// Open `path` in the sidebar file tree and record it as recent.
    fn open_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.file_tree.open(path.clone());
        self.sidebar_open = true;
        // New root: stale cache entries are useless now; preload the new dir.
        self.doc_cache.clear();
        self.preload_dir(path.clone(), cx);
        cx.global_mut::<Settings>().push_recent(path);
        save_settings(cx);
        crate::set_menus(cx);
        cx.notify();
    }

    fn on_open(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.guard_unsaved(window, cx, |this, window, cx| this.prompt_open(window, cx));
    }

    /// Show the native picker for a Markdown file or a folder.
    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            let path = rx.await.ok()?.ok()??.into_iter().next()?;
            this.update_in(cx, |this, window, cx| {
                if path.is_dir() {
                    this.open_folder(path, cx);
                } else {
                    this.load_path(path, window, cx);
                }
            })
            .ok()?;
            Some(())
        })
        .detach();
    }

    fn on_open_recent(&mut self, action: &OpenRecent, window: &mut Window, cx: &mut Context<Self>) {
        if action.0.is_empty() {
            return; // the "No Recent Files" placeholder
        }
        let path = PathBuf::from(action.0.clone());
        if path.is_dir() {
            self.open_folder(path, cx);
            return;
        }
        self.guard_unsaved(window, cx, move |this, window, cx| {
            this.load_path(path, window, cx)
        });
    }

    fn on_open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_open_folder(window, cx);
    }

    /// Show the native directory picker and open the chosen folder.
    fn prompt_open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Folder".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            let path = rx.await.ok()?.ok()??.into_iter().next()?;
            this.update_in(cx, |this, _window, cx| this.open_folder(path, cx))
                .ok()?;
            Some(())
        })
        .detach();
    }

    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.global_mut::<Settings>().sidebar_open = self.sidebar_open;
        save_settings(cx);
        // Opening the sidebar with no folder yet → jump straight to the picker.
        if self.sidebar_open && !self.file_tree.is_open() {
            self.prompt_open_folder(window, cx);
        }
        cx.notify();
    }

    /// Clicked a row in the file tree: toggle folders, open files.
    fn on_tree_entry(
        &mut self,
        path: PathBuf,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if is_dir {
            // Expanding a folder warms the cache for its documents.
            if self.file_tree.toggle(&path) {
                self.preload_dir(path, cx);
            }
            cx.notify();
        } else {
            self.guard_unsaved(window, cx, move |this, window, cx| {
                this.load_path(path, window, cx)
            });
        }
    }

    fn on_reload(&mut self, _: &Reload, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_path.is_some() {
            self.guard_unsaved(window, cx, |this, window, cx| {
                if let Some(path) = this.file_path.clone() {
                    this.last_modified = None; // force a re-read
                    this.load_path(path, window, cx);
                }
            });
        }
    }

    fn on_toggle_edit(&mut self, _: &ToggleEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = !self.editing;
        if self.editing {
            self.input_state.focus_handle(cx).focus(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_path.is_some() && !self.imported {
            self.save_in_place(window, cx);
            return;
        }

        // No writable backing file (untitled, or a converted docx/hwpx/pdf) —
        // prompt for a destination; imported docs default to a Markdown copy.
        let dir = self
            .file_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let default_name = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("{}.md", s.to_string_lossy()))
            .unwrap_or_else(|| "Untitled.md".to_string());
        let rx = cx.prompt_for_new_path(&dir, Some(&default_name));
        cx.spawn_in(window, async move |this, cx| {
            let path = rx.await.ok()?.ok()??;
            this.update_in(cx, |this, window, cx| {
                let text = this.text(cx).to_string();
                match std::fs::write(&path, &text) {
                    Ok(()) => {
                        this.last_modified =
                            std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                        this.file_path = Some(path.clone());
                        this.dirty = false;
                        this.imported = false;
                        this.set_doc_kind(DocKind::for_path(&path), cx);
                        this.preview_text = this.wrap_preview(this.text(cx));
                        this.start_watch(path.clone(), window, cx);
                        cx.global_mut::<Settings>().push_recent(path);
                        save_settings(cx);
                        crate::set_menus(cx);
                        cx.notify();
                    }
                    Err(err) => notify_save_error(&path, &err, window, cx),
                }
            })
            .ok()?;
            Some(())
        })
        .detach();
    }

    /// Write the buffer to its backing file. Reports failure via a notification
    /// and returns whether the write succeeded.
    fn save_in_place(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.imported {
            return false; // never overwrite a docx/hwpx/pdf original
        }
        let Some(path) = self.file_path.clone() else {
            return false;
        };
        let text = self.text(cx).to_string();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                self.last_modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                self.dirty = false;
                cx.notify();
                true
            }
            Err(err) => {
                notify_save_error(&path, &err, window, cx);
                false
            }
        }
    }

    fn on_close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.guard_unsaved(window, cx, |_, window, _| window.remove_window());
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.guard_unsaved(window, cx, |_, _, cx| cx.quit());
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let next = if cx.theme().mode.is_dark() {
            ThemePref::Light
        } else {
            ThemePref::Dark
        };
        self.set_theme_pref(next, window, cx);
    }

    fn on_set_theme(&mut self, action: &SetTheme, window: &mut Window, cx: &mut Context<Self>) {
        let pref = match action.0.as_str() {
            "dark" => ThemePref::Dark,
            "system" => ThemePref::System,
            _ => ThemePref::Light,
        };
        self.set_theme_pref(pref, window, cx);
    }

    fn on_toggle_settings(
        &mut self,
        _: &ToggleSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_settings = !self.show_settings;
        if self.show_settings {
            // Sync the font fields to the persisted values when opening.
            let settings = cx.global::<Settings>().clone();
            self.editor_font_input.update(cx, |state, cx| {
                state.set_value(settings.editor_font.clone(), window, cx)
            });
            self.preview_font_input.update(cx, |state, cx| {
                state.set_value(settings.preview_font.clone(), window, cx)
            });
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn on_font_inc(&mut self, _: &FontInc, _window: &mut Window, cx: &mut Context<Self>) {
        self.adjust_font_size(FONT_STEP, cx);
    }

    fn on_font_dec(&mut self, _: &FontDec, _window: &mut Window, cx: &mut Context<Self>) {
        self.adjust_font_size(-FONT_STEP, cx);
    }

    fn adjust_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        let current = cx.global::<Settings>().body_font_size;
        let next = (current + delta).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        if (next - current).abs() > f32::EPSILON {
            cx.global_mut::<Settings>().body_font_size = next;
            save_settings(cx);
            cx.notify();
        }
    }

    fn set_theme_pref(&mut self, pref: ThemePref, window: &mut Window, cx: &mut Context<Self>) {
        cx.global_mut::<Settings>().theme = pref;
        save_settings(cx);
        apply_theme(pref, window, cx);
        apply_syntax_theme(cx);
        cx.notify();
    }

    fn on_set_syntax_theme(
        &mut self,
        action: &SetSyntaxTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.global_mut::<Settings>().syntax_theme = action.0.clone();
        save_settings(cx);
        // Restore the built-in highlight theme, then override if a preset is set
        // (so picking "Default" reverts cleanly).
        let pref = cx.global::<Settings>().theme;
        apply_theme(pref, window, cx);
        apply_syntax_theme(cx);
        cx.notify();
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, _window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom + ZOOM_STEP, cx);
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, _window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom - ZOOM_STEP, cx);
    }

    fn on_zoom_reset(&mut self, _: &ZoomReset, _window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(1.0, cx);
    }

    fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        let zoom = ((zoom * 10.0).round() / 10.0).clamp(ZOOM_MIN, ZOOM_MAX);
        if (zoom - self.zoom).abs() > f32::EPSILON {
            self.zoom = zoom;
            cx.global_mut::<Settings>().zoom = zoom;
            save_settings(cx);
            cx.notify();
        }
    }

    /// Build the rendered Markdown element, scaled by zoom and the configured
    /// base font size / preview font family.
    fn styled_markdown(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let settings = cx.global::<Settings>();
        let zoom = self.zoom;
        let body = settings.body_font_size.max(1.0) * zoom;
        let preview_font = settings.preview_font.clone();

        // Scale fenced code blocks too (they otherwise use the fixed mono size).
        let mut code_block = StyleRefinement::default();
        code_block.text.font_size = Some((theme.mono_font_size * zoom).into());

        let style = TextViewStyle {
            heading_base_font_size: px(body),
            highlight_theme: theme.highlight_theme.clone(),
            is_dark: theme.mode.is_dark(),
            code_block,
            ..Default::default()
        };

        markdown(self.preview_text.clone())
            .style(style)
            .scrollable(true)
            .selectable(true)
            .size_full()
            .text_size(px(body))
            .px_8()
            .py_6()
            .when(!preview_font.is_empty(), |this| this.font_family(preview_font))
    }

    fn document_title(&self) -> SharedString {
        let name = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());

        if self.dirty {
            format!("{name} •").into()
        } else {
            name.into()
        }
    }

    fn render_title_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let is_dark = cx.theme().mode.is_dark();
        let theme_icon = if is_dark { IconName::Sun } else { IconName::Moon };
        let editing = self.editing;

        TitleBar::new().child(
            h_flex()
                .w_full()
                .pr_2()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Button::new("sidebar")
                                .icon(IconName::PanelLeft)
                                .ghost()
                                .small()
                                .tooltip("Toggle sidebar (⌘B)")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleSidebar), cx)
                                }),
                        )
                        .child(Icon::new(IconName::BookOpen))
                        .child(div().font_semibold().child(self.document_title())),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .when(self.zoom != 1.0, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{}%", (self.zoom * 100.0).round() as i32)),
                            )
                        })
                        .child(
                            Button::new("edit")
                                .icon(if editing {
                                    IconName::Eye
                                } else {
                                    IconName::PanelRight
                                })
                                .label(if editing { "Preview" } else { "Edit" })
                                .ghost()
                                .small()
                                .tooltip("Toggle editor (⌘E)")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleEdit), cx)
                                }),
                        )
                        .when(self.dirty, |this| {
                            this.child(
                                Button::new("save")
                                    .icon(IconName::HardDrive)
                                    .ghost()
                                    .small()
                                    .tooltip("Save (⌘S)")
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(Save), cx)
                                    }),
                            )
                        })
                        .child(
                            Button::new("open")
                                .icon(IconName::FolderOpen)
                                .label("Open")
                                .ghost()
                                .small()
                                .tooltip("Open a Markdown file (⌘O)")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(OpenFile), cx)
                                }),
                        )
                        .child(
                            Button::new("theme")
                                .icon(theme_icon)
                                .ghost()
                                .small()
                                .tooltip("Toggle light/dark (⌘⇧L)")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleTheme), cx)
                                }),
                        ),
                ),
        )
    }

    /// Full-width rendered Markdown.
    fn render_preview(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("doc")
            .size_full()
            .bg(cx.theme().background)
            .child(self.styled_markdown(cx))
    }

    /// Split view: live preview on the left, source editor on the right.
    fn render_split(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let settings = cx.global::<Settings>();

        let editor_font: SharedString = if settings.editor_font.is_empty() {
            theme.mono_font_family.clone()
        } else {
            settings.editor_font.clone().into()
        };

        // The code editor derives its font size *and* line height from `rem`, so
        // we zoom it by scaling `rem` for just this subtree (see `RemScaled`). The
        // editor input renders text at 0.875·rem, so divide to hit the target px.
        let editor_rem = px(settings.body_font_size.max(1.0) * self.zoom / 0.875);
        // Use the highlight theme's editor background (dark slate by default, or
        // the active preset's bg) so syntax colors read correctly.
        let editor_bg = theme
            .highlight_theme
            .style
            .editor_background
            .unwrap_or(theme.background);
        let editor = RemScaled::new(
            editor_rem,
            div()
                .id("editor")
                .size_full()
                .bg(editor_bg)
                .border_l_1()
                .border_color(theme.border)
                .font_family(editor_font)
                .child(
                    Input::new(&self.input_state)
                        .h_full()
                        .p_3()
                        .border_0()
                        .focus_bordered(false),
                ),
        );

        // Both panels are size-less → equal flex → an exact 50/50 split that
        // tracks the window size (still draggable afterwards).
        div().size_full().child(
            h_resizable("split")
                .child(
                    resizable_panel().child(
                        div()
                            .id("preview")
                            .size_full()
                            .bg(theme.background)
                            .child(self.styled_markdown(cx)),
                    ),
                )
                .child(resizable_panel().child(editor)),
        )
    }

    /// VSCode-style file-explorer sidebar.
    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let weak = cx.entity().downgrade();

        let header_title = self
            .file_tree
            .root()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_uppercase())
            .unwrap_or_else(|| "EXPLORER".to_string());

        let header = h_flex()
            .h(px(34.))
            .w_full()
            .px_2()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme.sidebar_border)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.sidebar_foreground)
                    .truncate()
                    .child(header_title),
            )
            .child(
                h_flex()
                    .gap_0p5()
                    .child(
                        Button::new("sb-open")
                            .icon(IconName::FolderOpen)
                            .ghost()
                            .xsmall()
                            .tooltip("Open Folder (⌘⇧O)")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenFolder), cx)
                            }),
                    )
                    .when(self.file_tree.is_open(), |this| {
                        let w1 = weak.clone();
                        let w2 = weak.clone();
                        this.child(
                            Button::new("sb-refresh")
                                .icon(IconName::Redo)
                                .ghost()
                                .xsmall()
                                .tooltip("Refresh")
                                .on_click(move |_, _, cx| {
                                    let _ = w1.update(cx, |this, cx| {
                                        this.file_tree.refresh();
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new("sb-collapse")
                                .icon(IconName::ChevronUp)
                                .ghost()
                                .xsmall()
                                .tooltip("Collapse All")
                                .on_click(move |_, _, cx| {
                                    let _ = w2.update(cx, |this, cx| {
                                        this.file_tree.collapse_all();
                                        cx.notify();
                                    });
                                }),
                        )
                    }),
            );

        let body = if !self.file_tree.is_open() {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_4()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("No folder opened"),
                )
                .child(
                    Button::new("sb-open-empty")
                        .primary()
                        .small()
                        .label("Open Folder")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(OpenFolder), cx)
                        }),
                )
                .into_any_element()
        } else {
            let rows = self.file_tree.rows();
            let current = self.file_path.clone();
            // macOS system accent blue (selectedContentBackgroundColor), white on top.
            let accent = gpui::hsla(0.586, 0.92, 0.52, 1.0);
            let on_accent = gpui::hsla(0., 0., 1., 1.);
            // A blue tint for hover — clearly visible, ties to the selection.
            let hover_bg = gpui::hsla(0.586, 0.92, 0.52, 0.24);
            v_flex()
                .id("tree-scroll")
                .size_full()
                .min_h(px(0.))
                .py_1()
                .px_1p5()
                .overflow_y_scroll()
                .children(rows.into_iter().map(move |row| {
                    let path = row.entry.path.clone();
                    let is_dir = row.entry.is_dir;
                    let selected = !is_dir && current.as_deref() == Some(path.as_path());
                    let id = SharedString::from(path.to_string_lossy().to_string());

                    let name_color = if selected { on_accent } else { theme.sidebar_foreground };
                    let chevron_color = if selected { on_accent } else { theme.muted_foreground };
                    let icon_color = if selected {
                        on_accent
                    } else if is_dir {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    };

                    // Leading chevron (folders) or spacer (files).
                    let lead = if is_dir {
                        Icon::new(if row.expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(14.))
                        .text_color(chevron_color)
                        .into_any_element()
                    } else {
                        div().w(px(14.)).into_any_element()
                    };

                    let type_icon = Icon::new(if !is_dir {
                        IconName::File
                    } else if row.expanded {
                        IconName::FolderOpen
                    } else {
                        IconName::Folder
                    })
                    .size(px(15.))
                    .text_color(icon_color);

                    let weak = weak.clone();
                    div()
                        .id(id)
                        .flex()
                        .items_center()
                        .gap_1()
                        .h(px(26.))
                        .rounded_md()
                        .pl(px(6. + row.depth as f32 * 14.))
                        .pr_2()
                        .text_sm()
                        .text_color(name_color)
                        .cursor_pointer()
                        .when(selected, |this| this.bg(accent).font_medium())
                        .when(!selected, |this| this.hover(|this| this.bg(hover_bg)))
                        .child(lead)
                        .child(type_icon)
                        .child(div().truncate().child(row.entry.name.clone()))
                        .on_click(move |_, window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.on_tree_entry(path.clone(), is_dir, window, cx)
                            });
                        })
                }))
                .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(header)
            .child(body)
    }

    /// Modal Settings panel (appearance, text size, fonts).
    fn render_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let settings = cx.global::<Settings>();
        let pref = settings.theme;
        let font_size = settings.body_font_size.round() as i32;
        let syntax = settings.syntax_theme.clone();

        div()
            .id("settings-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::hsla(0., 0., 0., 0.45))
            .child(
                v_flex()
                    .w(px(440.))
                    .gap_4()
                    .p_5()
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(theme.radius)
                    .shadow_lg()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(div().text_lg().font_semibold().child("Settings"))
                            .child(
                                Button::new("close-settings")
                                    .icon(IconName::Close)
                                    .ghost()
                                    .small()
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(ToggleSettings), cx)
                                    }),
                            ),
                    )
                    .child(settings_section(
                        "Appearance",
                        h_flex()
                            .gap_2()
                            .child(theme_pref_button("System", "system", pref == ThemePref::System))
                            .child(theme_pref_button("Light", "light", pref == ThemePref::Light))
                            .child(theme_pref_button("Dark", "dark", pref == ThemePref::Dark)),
                    ))
                    .child(settings_section(
                        "Text size",
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("font-dec")
                                    .icon(IconName::Minus)
                                    .outline()
                                    .small()
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(FontDec), cx)
                                    }),
                            )
                            .child(
                                div()
                                    .w(px(64.))
                                    .text_center()
                                    .child(format!("{font_size} px")),
                            )
                            .child(
                                Button::new("font-inc")
                                    .icon(IconName::Plus)
                                    .outline()
                                    .small()
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(FontInc), cx)
                                    }),
                            ),
                    ))
                    .child(settings_section(
                        "Preview font",
                        Input::new(&self.preview_font_input).small().w_full(),
                    ))
                    .child(settings_section(
                        "Editor font",
                        Input::new(&self.editor_font_input).small().w_full(),
                    ))
                    .child(settings_section(
                        "Syntax theme (dark)",
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(syntax_theme_button("Default", "", syntax.is_empty()))
                            .children(crate::syntax_theme::PRESETS.iter().map(|&name| {
                                syntax_theme_button(name, name, syntax == name)
                            })),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Zoom with ⌘+ / ⌘- · changes are saved automatically"),
                    ),
            )
    }
}

impl Focusable for MarkForge {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MarkForge {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let main_body = if self.editing {
            self.render_split(cx).into_any_element()
        } else {
            self.render_preview(cx).into_any_element()
        };

        // Optional left sidebar (file explorer), resizable like VSCode.
        let workspace = if self.sidebar_open {
            h_resizable("workspace")
                .child(
                    resizable_panel()
                        .size(px(180.))
                        .size_range(px(140.)..px(400.))
                        .child(self.render_sidebar(cx)),
                )
                .child(resizable_panel().child(div().size_full().child(main_body)))
                .into_any_element()
        } else {
            main_body
        };

        v_flex()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_open_recent))
            .on_action(cx.listener(Self::on_reload))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_toggle_edit))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_set_theme))
            .on_action(cx.listener(Self::on_set_syntax_theme))
            .on_action(cx.listener(Self::on_toggle_settings))
            .on_action(cx.listener(Self::on_font_inc))
            .on_action(cx.listener(Self::on_font_dec))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_quit))
            .on_drop::<ExternalPaths>(move |paths, window, cx| {
                let Some(path) = first_markdown(paths.paths()) else {
                    return;
                };
                let _ = weak.update(cx, |this, cx| {
                    // A dropped folder opens in the sidebar instead of erroring.
                    if path.is_dir() {
                        this.open_folder(path.clone(), cx);
                    } else {
                        this.guard_unsaved(window, cx, move |this, window, cx| {
                            this.load_path(path, window, cx)
                        });
                    }
                });
            })
            .child(self.render_title_bar(cx))
            .child(div().id("body").flex_1().min_h(px(0.)).w_full().child(workspace))
            .when(self.show_settings, |this| this.child(self.render_settings(cx)))
    }
}

/// Apply a theme preference, resolving `System` against the OS appearance.
fn apply_theme(pref: ThemePref, window: &mut Window, cx: &mut App) {
    let mode = match pref {
        ThemePref::Light => ThemeMode::Light,
        ThemePref::Dark => ThemeMode::Dark,
        ThemePref::System => {
            if is_dark_appearance(window.appearance()) {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            }
        }
    };
    Theme::change(mode, Some(window), cx);
}

/// Set the global highlight theme: the chosen syntax preset if any, otherwise
/// the built-in theme matching the current mode. Always setting it explicitly
/// matters — `Theme::change` keeps the previous highlight theme when the new
/// mode's config carries none, which left light syntax colors (near-invisible
/// `#333` keys) on a dark background.
fn apply_syntax_theme(cx: &mut App) {
    let name = cx
        .try_global::<Settings>()
        .map(|s| s.syntax_theme.clone())
        .unwrap_or_default();
    let theme = crate::syntax_theme::load(&name).unwrap_or_else(|| {
        if Theme::global(cx).mode.is_dark() {
            HighlightTheme::default_dark()
        } else {
            HighlightTheme::default_light()
        }
    });
    Theme::global_mut(cx).highlight_theme = theme;
}

fn is_dark_appearance(appearance: WindowAppearance) -> bool {
    matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

fn save_settings(cx: &App) {
    if let Some(settings) = cx.try_global::<Settings>() {
        settings.save();
    }
}

/// Document extensions MarkForge knows how to display.
fn is_supported_doc(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "md" | "markdown"
                    | "mdown"
                    | "mkd"
                    | "text"
                    | "txt"
                    | "json"
                    | "jsonc"
                    | "docx"
                    | "hwpx"
                    | "pdf"
            )
        })
        .unwrap_or(false)
}

/// Make raw file text ready for the buffer: minified JSON gets pretty-printed,
/// everything else passes through. Safe to run on the background executor.
fn prepare_doc_text(path: &Path, text: String) -> String {
    if matches!(DocKind::for_path(path), DocKind::Json { .. }) {
        prettify_minified_json(&text).unwrap_or(text)
    } else {
        text
    }
}

/// Largest byte index ≤ `max` that falls on a line (or char) boundary, so a
/// truncated preview never cuts mid-line or mid-codepoint.
fn preview_cut(text: &str, max: usize) -> usize {
    if text.len() <= max {
        return text.len();
    }
    if let Some(nl) = text.as_bytes()[..max].iter().rposition(|&b| b == b'\n') {
        return nl;
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// Pretty-print JSON that was squeezed onto a single line (no newline in the
/// first 4 KiB). Already-formatted documents pass through untouched, and
/// anything that doesn't parse (e.g. JSONC comments) stays as-is.
fn prettify_minified_json(text: &str) -> Option<String> {
    if text.as_bytes().iter().take(4096).any(|&b| b == b'\n') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// Surface a failed write as an in-window error notification.
fn notify_save_error(path: &PathBuf, err: &std::io::Error, window: &mut Window, cx: &mut App) {
    window.push_notification(
        (
            NotificationType::Error,
            format!("Couldn't save {}: {err}", path.display()),
        ),
        cx,
    );
}

/// A labelled section for the Settings panel.
fn settings_section(label: &'static str, content: impl IntoElement) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(div().text_sm().font_semibold().child(label))
        .child(content)
}

/// One of the Appearance choices; `value` is "system" | "light" | "dark".
fn theme_pref_button(label: &'static str, value: &'static str, active: bool) -> Button {
    let button = Button::new(value)
        .label(label)
        .small()
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(SetTheme(value.to_string())), cx)
        });
    if active {
        button.primary()
    } else {
        button.outline()
    }
}

/// A syntax-theme choice; `value` is the preset display name ("" = Default).
fn syntax_theme_button(label: &'static str, value: &'static str, active: bool) -> Button {
    let button = Button::new(SharedString::from(format!("syntax-{value}")))
        .label(label)
        .xsmall()
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(SetSyntaxTheme(value.to_string())), cx)
        });
    if active {
        button.primary()
    } else {
        button.outline()
    }
}

/// Pick the best path from a drop: a supported document if present, else the first.
fn first_markdown(paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .find(|p| is_supported_doc(p))
        .or_else(|| paths.first())
        .cloned()
}
