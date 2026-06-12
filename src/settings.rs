//! Persisted user settings (theme preference, zoom, recent files).
//!
//! Stored as JSON at `~/Library/Application Support/MarkForge/settings.json`.
//! Installed as a GPUI global so any view can read/update it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How the app decides between light and dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePref {
    /// Follow the macOS system appearance.
    System,
    /// Always light. This is the default.
    #[default]
    Light,
    /// Always dark.
    Dark,
}

const MAX_RECENT: usize = 10;

/// Bounds for the configurable base body font size (px).
pub const FONT_SIZE_MIN: f32 = 11.0;
pub const FONT_SIZE_MAX: f32 = 28.0;
pub const FONT_SIZE_DEFAULT: f32 = 15.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemePref,
    pub zoom: f32,
    /// Base body font size in px (before zoom).
    pub body_font_size: f32,
    /// Preview font family; empty = library/theme default.
    pub preview_font: String,
    /// Editor (monospace) font family; empty = theme default.
    pub editor_font: String,
    /// Syntax-highlighting preset name; empty = the built-in default.
    pub syntax_theme: String,
    pub recent: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePref::Light,
            zoom: 1.0,
            body_font_size: FONT_SIZE_DEFAULT,
            preview_font: String::new(),
            editor_font: String::new(),
            syntax_theme: String::new(),
            recent: Vec::new(),
        }
    }
}

impl gpui::Global for Settings {}

impl Settings {
    /// Load settings from disk, falling back to defaults on any error.
    /// Recent entries that no longer exist on disk are dropped.
    pub fn load() -> Self {
        let mut settings: Self = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        settings.recent.retain(|p| p.exists());
        settings
    }

    /// Persist to disk. Errors are logged but never propagated — settings are
    /// best-effort and must not interrupt the app.
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return;
            }
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(err) = std::fs::write(&path, json) {
                eprintln!("markforge: failed to save settings: {err}");
            }
        }
    }

    /// Move `path` to the front of the recent list (deduplicated, capped).
    pub fn push_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
    }

    fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support/MarkForge")
                .join("settings.json"),
        )
    }
}
