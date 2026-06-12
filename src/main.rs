//! MarkForge — a native macOS Markdown viewer & editor.
//!
//! Built on GPUI (Zed's GPU-accelerated UI framework) and the shadcn-style
//! `gpui-component` library. This entry point wires up the application:
//! actions, the macOS menu bar, key bindings, persisted settings, and the window.

mod app;
mod file_tree;
mod git;
mod import;
mod pdf;
mod rem_scaled;
mod settings;
mod syntax_theme;

use app::MarkForge;
use settings::Settings;
use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    Action, App, Bounds, Focusable, KeyBinding, Menu, MenuItem, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions, actions, px, size,
};
use gpui_component::{Root, TitleBar};
use gpui_component_assets::Assets;
use serde::Deserialize;

// Unit actions. Document-related ones are handled by the view (they need state);
// `Quit` is handled globally on the `App`.
actions!(
    markforge,
    [
        OpenFile,
        OpenFolder,
        Reload,
        Save,
        ToggleEdit,
        ToggleSidebar,
        ToggleTheme,
        ToggleSettings,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        TreeUp,
        TreeDown,
        TreeLeft,
        TreeRight,
        TreeConfirm,
        ToggleDiff,
        CommitAll,
        GitPush,
        GitPull,
        DiscardChanges,
        CloseWindow,
        Quit
    ]
);

/// Open a specific file from the "Open Recent" menu. Carries the path as a string.
#[derive(Clone, PartialEq, Eq, Action, Deserialize)]
#[action(namespace = markforge, no_json)]
pub struct OpenRecent(pub String);

/// Set the theme preference: "system", "light", or "dark".
#[derive(Clone, PartialEq, Eq, Action, Deserialize)]
#[action(namespace = markforge, no_json)]
pub struct SetTheme(pub String);

/// Set the syntax-highlighting preset (display name; "" = built-in default).
#[derive(Clone, PartialEq, Eq, Action, Deserialize)]
#[action(namespace = markforge, no_json)]
pub struct SetSyntaxTheme(pub String);

/// Check out the named git branch in the open folder's repository.
#[derive(Clone, PartialEq, Eq, Action, Deserialize)]
#[action(namespace = markforge, no_json)]
pub struct CheckoutBranch(pub String);

fn main() {
    let application = gpui_platform::application().with_assets(Assets);

    application.run(move |cx| {
        // Required before using any gpui-component feature.
        gpui_component::init(cx);

        // Load persisted settings and install them as a global.
        cx.set_global(Settings::load());

        register_global_actions(cx);
        bind_keys(cx);
        set_menus(cx);
        cx.activate(true);

        // Single-window app: once the last window is gone there is no way to
        // open a new one, so quit instead of lingering in the Dock.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        // Allow `markforge path/to/file.md` to open a document on launch.
        let initial_path = std::env::args().nth(1).map(PathBuf::from);
        open_main_window(initial_path, cx);
    });
}

/// Global (App-level) action handlers. Theme/zoom/document actions are handled by
/// the view so they have access to the window and document state.
fn register_global_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenFile, None),
        KeyBinding::new("cmd-shift-o", OpenFolder, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-r", Reload, None),
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("cmd-e", ToggleEdit, None),
        KeyBinding::new("cmd-,", ToggleSettings, None),
        KeyBinding::new("cmd-shift-l", ToggleTheme, None),
        KeyBinding::new("cmd-d", ToggleDiff, None),
        // Zoom. Accept both the bare `=`/`+` keys so ⌘+ works with or without Shift.
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ZoomReset, None),
        // Sidebar tree navigation. The editor/search inputs bind arrows in
        // their own (deeper) key context, so these only fire elsewhere.
        KeyBinding::new("up", TreeUp, None),
        KeyBinding::new("down", TreeDown, None),
        KeyBinding::new("left", TreeLeft, None),
        KeyBinding::new("right", TreeRight, None),
        KeyBinding::new("enter", TreeConfirm, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
}

/// (Re)build the macOS menu bar. Reads the current recent-files list from the
/// global settings, so call this again whenever that list changes.
pub fn set_menus(cx: &mut App) {
    let recent = cx
        .try_global::<Settings>()
        .map(|s| s.recent.clone())
        .unwrap_or_default();

    let recent_items: Vec<MenuItem> = if recent.is_empty() {
        vec![MenuItem::action("No Recent Files", OpenRecent(String::new()))]
    } else {
        recent
            .iter()
            .map(|p| {
                let label = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                MenuItem::action(label, OpenRecent(p.to_string_lossy().to_string()))
            })
            .collect()
    };

    cx.set_menus(vec![
        Menu {
            name: "MarkForge".into(),
            items: vec![
                MenuItem::action("Settings…", ToggleSettings),
                MenuItem::Submenu(Menu {
                    name: "Appearance".into(),
                    items: vec![
                        MenuItem::action("System", SetTheme("system".into())),
                        MenuItem::action("Light", SetTheme("light".into())),
                        MenuItem::action("Dark", SetTheme("dark".into())),
                    ],
                    disabled: false,
                }),
                MenuItem::separator(),
                MenuItem::action("Quit MarkForge", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Open…", OpenFile),
                MenuItem::action("Open Folder…", OpenFolder),
                MenuItem::Submenu(Menu {
                    name: "Open Recent".into(),
                    items: recent_items,
                    disabled: false,
                }),
                MenuItem::separator(),
                MenuItem::action("Save", Save),
                MenuItem::action("Reload", Reload),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindow),
            ],
            disabled: false,
        },
        Menu {
            name: "Git".into(),
            items: vec![
                MenuItem::action("Show Diff", ToggleDiff),
                MenuItem::action("Discard File Changes…", DiscardChanges),
                MenuItem::separator(),
                MenuItem::action("Push", GitPush),
                MenuItem::action("Pull", GitPull),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Toggle Editor", ToggleEdit),
                MenuItem::Submenu(Menu {
                    name: "Syntax Theme".into(),
                    items: std::iter::once(MenuItem::action(
                        "Default",
                        SetSyntaxTheme(String::new()),
                    ))
                    .chain(syntax_theme::PRESETS.iter().map(|&name| {
                        MenuItem::action(name, SetSyntaxTheme(name.to_string()))
                    }))
                    .collect(),
                    disabled: false,
                }),
                MenuItem::separator(),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::action("Actual Size", ZoomReset),
            ],
            disabled: false,
        },
    ]);
}

fn open_main_window(initial_path: Option<PathBuf>, cx: &mut App) {
    let window_size = size(px(1100.), px(820.));
    let bounds = Bounds::centered(None, window_size, cx);

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitleBar::title_bar_options()),
        window_min_size: Some(size(px(480.), px(360.))),
        kind: WindowKind::Normal,
        // The root view paints a slightly translucent gradient, so what's
        // behind the window shows through as a faint blur (macOS vibrancy).
        window_background: WindowBackgroundAppearance::Blurred,
        ..Default::default()
    };

    let window = cx
        .open_window(options, |window, cx| {
            let view = cx.new(|cx| MarkForge::new(initial_path.clone(), window, cx));

            // Focus the view so its key-bound actions dispatch.
            let focus_handle = view.focus_handle(cx);
            window.defer(cx, move |window, cx| focus_handle.focus(window, cx));

            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");

    window
        .update(cx, |_, window, _| window.activate_window())
        .ok();
}
