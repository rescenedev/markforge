//! Persisted user settings (theme preference, zoom, recent files).
//!
//! Stored as JSON at `~/Library/Application Support/MarkForge/settings.json`.
//! Installed as a GPUI global so any view can read/update it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    /// Whether the file-explorer sidebar is shown (default: open).
    pub sidebar_open: bool,
    /// Inner padding (px) around the rendered preview. 0 = edge-to-edge.
    pub preview_padding: f32,
    /// Opacity of the window backdrop gradient (0.2 transparent – 1.0 solid).
    /// Sidebar and title bar derive slightly more transparent values from it.
    pub backdrop_opacity: f32,
    /// Auto-commit the file to its repository on every save (note-vault style).
    pub git_auto_commit: bool,
    /// Sidebar background in dark mode (hex, e.g. "#262B3C"); empty = default.
    pub sidebar_bg_dark: String,
    /// Sidebar section headers the user has collapsed (e.g. "FAVORITES").
    pub collapsed_sections: Vec<String>,
    pub recent: Vec<PathBuf>,
    /// Open counts per path, feeding the sidebar Favorites section.
    pub usage: HashMap<PathBuf, u32>,
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
            sidebar_open: true,
            preview_padding: 8.0,
            backdrop_opacity: 0.68,
            git_auto_commit: false,
            sidebar_bg_dark: String::new(),
            collapsed_sections: Vec::new(),
            recent: Vec::new(),
            usage: HashMap::new(),
        }
    }
}

impl gpui::Global for Settings {}

impl Settings {
    /// Load settings from disk, falling back to defaults on any error.
    /// Hand-edited values are clamped to sane ranges; recent entries that no
    /// longer exist on disk are dropped.
    pub fn load() -> Self {
        let mut settings: Self = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        settings.body_font_size = settings.body_font_size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        settings.backdrop_opacity = settings.backdrop_opacity.clamp(0.2, 1.0);
        settings.recent.retain(|p| p.exists());
        settings.usage.retain(|p, _| p.exists());
        settings
    }

    /// Count an open of `path`; the table is bounded to the heaviest hitters.
    pub fn bump_usage(&mut self, path: &Path) {
        *self.usage.entry(path.to_path_buf()).or_insert(0) += 1;
        if self.usage.len() > 200 {
            let mut entries: Vec<_> = std::mem::take(&mut self.usage).into_iter().collect();
            entries.sort_by_key(|e| std::cmp::Reverse(e.1));
            entries.truncate(100);
            self.usage = entries.into_iter().collect();
        }
    }

    /// The most-opened existing paths matching `pred`, busiest first.
    pub fn top_usage(&self, pred: impl Fn(&Path) -> bool, limit: usize) -> Vec<PathBuf> {
        let mut entries: Vec<(&PathBuf, &u32)> = self.usage.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1));
        entries
            .into_iter()
            .map(|(p, _)| p)
            .filter(|p| pred(p))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Persist to disk. Errors are logged but never propagated — settings are
    /// best-effort and must not interrupt the app.
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent()
            && std::fs::create_dir_all(dir).is_err() {
                return;
            }
        if let Ok(json) = serde_json::to_string_pretty(self)
            && let Err(err) = std::fs::write(&path, json) {
                eprintln!("markforge: failed to save settings: {err}");
            }
    }

    /// Move `path` to the front of the recent list (deduplicated, capped).
    pub fn push_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support/MarkForge")
                .join("settings.json"),
        )
    }

    /// Whether `path` is the settings file (editing it in-app applies live).
    pub fn is_settings_path(path: &Path) -> bool {
        Self::path().as_deref() == Some(path)
    }
}

/// Parse a `#RRGGBB` hex color.
pub fn parse_hex_color(s: &str) -> Option<gpui::Hsla> {
    let hex = s.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(gpui::rgb(v).into())
}

#[cfg(test)]
mod tests {
    use super::parse_hex_color;

    #[test]
    fn hex_valid_colors_parse() {
        assert!(parse_hex_color("#ffffff").is_some());
        assert!(parse_hex_color("#000000").is_some());
        assert!(parse_hex_color("#3182f6").is_some());
        assert!(parse_hex_color("  #1a1d29  ").is_some(), "should trim whitespace");
    }

    #[test]
    fn hex_invalid_colors_rejected() {
        assert!(parse_hex_color("#fff").is_none(), "3-digit not supported");
        assert!(parse_hex_color("3182f6").is_none(), "missing #");
        assert!(parse_hex_color("#gggggg").is_none(), "non-hex digits");
        assert!(parse_hex_color("#12345").is_none(), "wrong length");
        assert!(parse_hex_color("").is_none());
    }

    // ---- Settings (de)serialization ----
    use super::Settings;

    #[test]
    fn settings_serde_roundtrip_is_stable() {
        let json1 = serde_json::to_string(&Settings::default()).unwrap();
        let back: Settings = serde_json::from_str(&json1).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        assert_eq!(json1, json2, "round-trip must not lose or reshape data");
    }

    #[test]
    fn settings_fills_defaults_for_missing_fields() {
        // #[serde(default)] lets a partial config (only `zoom`) still load.
        let s: Settings = serde_json::from_str(r#"{"zoom": 1.5}"#).unwrap();
        assert_eq!(s.zoom, 1.5_f32);
        assert_eq!(s.body_font_size, Settings::default().body_font_size);
        assert_eq!(s.sidebar_open, Settings::default().sidebar_open);
    }

    #[test]
    fn settings_ignores_unknown_fields() {
        // A field removed in a later version must not break loading old configs.
        let s: Settings = serde_json::from_str(r#"{"zoom": 1.0, "obsolete": true}"#).unwrap();
        assert_eq!(s.zoom, 1.0_f32);
    }
}
