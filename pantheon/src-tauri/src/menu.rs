// T-015 (REQ-028, REQ-029) — Pantheon custom native menu.
//
// Tauri v2 installs a default macOS menu that includes an Edit submenu with
// Find/Find Next/Find Previous and Replace. Those trigger WKWebView's native
// find bar, which is confusing inside a terminal app — ⌘F in a terminal is
// not a document search, it's a shell keybinding. We rebuild the menu from
// scratch with only the submenus we want (App, Edit without Find, Window,
// Help) so the offending items simply do not exist.
//
// Note on macOS ordering: the FIRST submenu added via MenuBuilder becomes
// the "App" menu (named after the app, regardless of the label we pass to
// SubmenuBuilder). That's why `app_menu` is built first below — macOS
// treats position 0 as the application menu and names it from CFBundleName.
//
// Cmd+F is ALSO suppressed at the WKWebView level by tauri-plugin-prevent-
// default in lib.rs, because removing the menu item doesn't stop WebKit
// from intercepting the chord on its own. The plugin + the menu rebuild
// together give full suppression.

use tauri::{
    menu::{Menu, MenuBuilder, PredefinedMenuItem, SubmenuBuilder},
    AppHandle, Runtime,
};

/// Build Pantheon's native menu. Called from `lib.rs::run()`'s builder via
/// `.menu(|handle| menu::build_menu(handle))`. Returns a fully-assembled
/// `Menu` ready for the Tauri runtime to install as the process-wide menu.
pub fn build_menu<R: Runtime>(handle: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    // App menu (position 0 → becomes the application menu on macOS).
    // PredefinedMenuItem gives us the OS-native chords + behaviors for free:
    // Quit handles ⌘Q, Hide handles ⌘H, etc.
    let app_menu = SubmenuBuilder::new(handle, "Pantheon")
        .item(&PredefinedMenuItem::about(handle, None, None)?)
        .separator()
        .item(&PredefinedMenuItem::services(handle, None)?)
        .separator()
        .item(&PredefinedMenuItem::hide(handle, None)?)
        .item(&PredefinedMenuItem::hide_others(handle, None)?)
        .item(&PredefinedMenuItem::show_all(handle, None)?)
        .separator()
        .item(&PredefinedMenuItem::quit(handle, None)?)
        .build()?;

    // Edit menu — deliberately omits Find, Find Next, Find Previous, Replace.
    // SubmenuBuilder exposes .undo/.redo/.cut/.copy/.paste/.select_all as
    // direct methods that attach the predefined items with their native
    // chords and OS integration.
    let edit_menu = SubmenuBuilder::new(handle, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    // Window menu — macOS convention: Minimize, Zoom, CloseWindow. The
    // close_window item respects our close-to-tray interceptor in lib.rs
    // because it fires the same WindowEvent::CloseRequested we trap there.
    let window_menu = SubmenuBuilder::new(handle, "Window")
        .item(&PredefinedMenuItem::minimize(handle, None)?)
        .item(&PredefinedMenuItem::maximize(handle, None)?)
        .separator()
        .item(&PredefinedMenuItem::close_window(handle, None)?)
        .build()?;

    MenuBuilder::new(handle)
        .items(&[&app_menu, &edit_menu, &window_menu])
        .build()
}
