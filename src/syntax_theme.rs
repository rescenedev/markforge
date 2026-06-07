//! Built-in syntax-highlighting themes (Zed-compatible JSON).
//!
//! VS Code color-theme files can't be loaded directly (they use TextMate scopes;
//! this engine uses tree-sitter capture names), so popular palettes are bundled
//! here. Selecting one overrides `cx.theme().highlight_theme`, which the code
//! editor and preview code blocks both read live.

use std::sync::Arc;

use gpui_component::highlighter::HighlightTheme;

/// Display names of the bundled presets (besides the built-in "Default").
pub const PRESETS: &[&str] = &["VS Code Dark+", "Dracula", "Nord", "Monokai"];

/// Parse a preset by display name. Returns `None` for the built-in default.
pub fn load(name: &str) -> Option<Arc<HighlightTheme>> {
    let json = match name {
        "VS Code Dark+" => VSCODE_DARK,
        "Dracula" => DRACULA,
        "Nord" => NORD,
        "Monokai" => MONOKAI,
        _ => return None,
    };
    serde_json::from_str::<HighlightTheme>(json)
        .ok()
        .map(Arc::new)
}

const VSCODE_DARK: &str = r##"{
  "name": "VS Code Dark+",
  "appearance": "dark",
  "style": {
    "editor.foreground": "#D4D4D4",
    "editor.background": "#1E1E1E",
    "editor.active_line.background": "#2A2D2E",
    "editor.line_number": "#858585",
    "editor.active_line_number": "#C6C6C6",
    "syntax": {
      "keyword": { "color": "#569CD6" },
      "string": { "color": "#CE9178" },
      "string.escape": { "color": "#D7BA7D" },
      "number": { "color": "#B5CEA8" },
      "boolean": { "color": "#569CD6" },
      "constant": { "color": "#4FC1FF" },
      "function": { "color": "#DCDCAA" },
      "type": { "color": "#4EC9B0" },
      "constructor": { "color": "#4EC9B0" },
      "property": { "color": "#9CDCFE" },
      "variable.special": { "color": "#9CDCFE" },
      "comment": { "color": "#6A9955", "font_style": "italic" },
      "comment.doc": { "color": "#6A9955", "font_style": "italic" },
      "tag": { "color": "#569CD6" },
      "attribute": { "color": "#9CDCFE" },
      "operator": { "color": "#D4D4D4" },
      "punctuation": { "color": "#D4D4D4" },
      "title": { "color": "#DCDCAA", "font_weight": 600 },
      "text.literal": { "color": "#CE9178" },
      "text.code.span": { "color": "#CE9178" }
    }
  }
}"##;

const DRACULA: &str = r##"{
  "name": "Dracula",
  "appearance": "dark",
  "style": {
    "editor.foreground": "#F8F8F2",
    "editor.background": "#282A36",
    "editor.active_line.background": "#383B4C",
    "editor.line_number": "#6272A4",
    "editor.active_line_number": "#F8F8F2",
    "syntax": {
      "keyword": { "color": "#FF79C6" },
      "string": { "color": "#F1FA8C" },
      "string.escape": { "color": "#FF79C6" },
      "number": { "color": "#BD93F9" },
      "boolean": { "color": "#BD93F9" },
      "constant": { "color": "#BD93F9" },
      "function": { "color": "#50FA7B" },
      "type": { "color": "#8BE9FD", "font_style": "italic" },
      "constructor": { "color": "#8BE9FD" },
      "property": { "color": "#8BE9FD" },
      "variable.special": { "color": "#BD93F9" },
      "comment": { "color": "#6272A4", "font_style": "italic" },
      "comment.doc": { "color": "#6272A4", "font_style": "italic" },
      "tag": { "color": "#FF79C6" },
      "attribute": { "color": "#50FA7B" },
      "operator": { "color": "#FF79C6" },
      "punctuation": { "color": "#F8F8F2" },
      "title": { "color": "#50FA7B", "font_weight": 600 },
      "text.literal": { "color": "#F1FA8C" },
      "text.code.span": { "color": "#F1FA8C" }
    }
  }
}"##;

const NORD: &str = r##"{
  "name": "Nord",
  "appearance": "dark",
  "style": {
    "editor.foreground": "#D8DEE9",
    "editor.background": "#2E3440",
    "editor.active_line.background": "#3B4252",
    "editor.line_number": "#4C566A",
    "editor.active_line_number": "#D8DEE9",
    "syntax": {
      "keyword": { "color": "#81A1C1" },
      "string": { "color": "#A3BE8C" },
      "string.escape": { "color": "#EBCB8B" },
      "number": { "color": "#B48EAD" },
      "boolean": { "color": "#81A1C1" },
      "constant": { "color": "#B48EAD" },
      "function": { "color": "#88C0D0" },
      "type": { "color": "#8FBCBB" },
      "constructor": { "color": "#8FBCBB" },
      "property": { "color": "#8FBCBB" },
      "variable.special": { "color": "#B48EAD" },
      "comment": { "color": "#616E88", "font_style": "italic" },
      "comment.doc": { "color": "#616E88", "font_style": "italic" },
      "tag": { "color": "#81A1C1" },
      "attribute": { "color": "#8FBCBB" },
      "operator": { "color": "#81A1C1" },
      "punctuation": { "color": "#ECEFF4" },
      "title": { "color": "#88C0D0", "font_weight": 600 },
      "text.literal": { "color": "#A3BE8C" },
      "text.code.span": { "color": "#A3BE8C" }
    }
  }
}"##;

const MONOKAI: &str = r##"{
  "name": "Monokai",
  "appearance": "dark",
  "style": {
    "editor.foreground": "#F8F8F2",
    "editor.background": "#272822",
    "editor.active_line.background": "#3E3D32",
    "editor.line_number": "#90908A",
    "editor.active_line_number": "#F8F8F2",
    "syntax": {
      "keyword": { "color": "#F92672" },
      "string": { "color": "#E6DB74" },
      "string.escape": { "color": "#AE81FF" },
      "number": { "color": "#AE81FF" },
      "boolean": { "color": "#AE81FF" },
      "constant": { "color": "#AE81FF" },
      "function": { "color": "#A6E22E" },
      "type": { "color": "#66D9EF", "font_style": "italic" },
      "constructor": { "color": "#A6E22E" },
      "property": { "color": "#F8F8F2" },
      "variable.special": { "color": "#FD971F" },
      "comment": { "color": "#75715E", "font_style": "italic" },
      "comment.doc": { "color": "#75715E", "font_style": "italic" },
      "tag": { "color": "#F92672" },
      "attribute": { "color": "#A6E22E" },
      "operator": { "color": "#F92672" },
      "punctuation": { "color": "#F8F8F2" },
      "title": { "color": "#A6E22E", "font_weight": 600 },
      "text.literal": { "color": "#E6DB74" },
      "text.code.span": { "color": "#E6DB74" }
    }
  }
}"##;
