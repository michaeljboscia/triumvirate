// T-014 (REQ-003, REQ-019) — Pantheon menubar tray icon.
//
// Builds a system tray icon at app startup, attaches a right-click menu
// with Quit, handles left-click to show/focus the main window, and
// exposes an `update_daemon_state` Tauri command for the frontend to
// call when the daemon's health state changes — which swaps the tray
// icon to one of four template images:
//
//   starting     — pulsing/dim icon while the daemon is being launched
//   ready        — filled icon, daemon is connected and healthy
//   degraded     — outlined icon, daemon connected but reporting issues
//   disconnected — empty/X icon, daemon is unreachable
//
// Real PNG assets for the four states are deferred to a polish pass —
// for now we use the bundled default window icon for all four slots so
// the state-swap MECHANISM is provable end-to-end. The frontend can
// already call `update_daemon_state(state)` from any onMount + the daemon
// reconnect loop in T-020, and the icon will swap; replacing the four
// PNGs with real template images is a drop-in change that doesn't require
// touching this file.
//
// Template images on macOS: filenames ending in `Template.png` are
// auto-inverted by the OS for dark/light mode. When the real assets land
// they should be named `IconStartingTemplate.png` etc. For now we just
// reuse `app.default_window_icon()`.

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

/// Stable id used both as the tray's own identifier and the way the
/// frontend's `update_daemon_state` command looks the tray up at runtime.
pub const TRAY_ID: &str = "pantheon-main";

/// The four daemon states the tray icon mirrors. Frontend sends one of
/// these strings via the `update_daemon_state` command. Anything else
/// falls back to `Disconnected`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonTrayState {
    Starting,
    Ready,
    Degraded,
    Disconnected,
}

impl DaemonTrayState {
    fn from_str(s: &str) -> Self {
        match s {
            "starting" => Self::Starting,
            "ready" => Self::Ready,
            "degraded" => Self::Degraded,
            _ => Self::Disconnected,
        }
    }
}

/// Build the tray icon and menu, register click + menu handlers. Called
/// from `lib.rs::run`'s setup hook.
pub fn init_tray<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    // Right-click menu — just Quit for now. Wave 7 (T-027) adds a
    // Preferences entry. The id "quit" is matched in on_menu_event below.
    let quit_item = MenuItem::with_id(app, "quit", "Quit Pantheon", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit_item])?;

    // Default window icon as the placeholder for all four states until
    // real template assets land. unwrap is safe — Tauri always bundles
    // a default icon and panics at app startup if it can't load it.
    let default_icon = app
        .default_window_icon()
        .expect("Tauri must provide a default window icon")
        .clone();

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(default_icon)
        // Disable left-click → menu so the on_tray_icon_event handler
        // below receives the MouseButton::Left event and can show/focus
        // the main window. Right-click still opens the menu.
        .show_menu_on_left_click(false)
        .menu(&menu)
        .on_menu_event(|app, event| {
            if event.id.as_ref() == "quit" {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click released = show + focus the main window. The
            // window may have been hidden by the close-to-tray handler
            // in lib.rs::run.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Tauri command — frontend calls this when the daemon health state
/// changes (initially in T-020 when DaemonClient lands; today it's
/// callable for testing). Looks up the tray by id and swaps the icon.
///
/// Until real PNG assets land, every state uses the same default window
/// icon. The `update_daemon_state` command IS wired and reaches the right
/// tray handle, so swapping in real PNGs is a localized change here:
/// branch on `_tray_state` and call `Image::from_bytes(include_bytes!(
/// "../icons/StateXTemplate.png"))?` per arm.
#[tauri::command]
pub fn update_daemon_state(app: AppHandle, state: String) -> Result<(), String> {
    let _tray_state = DaemonTrayState::from_str(&state);
    let tray = app
        .tray_by_id(TRAY_ID)
        .ok_or_else(|| format!("tray '{TRAY_ID}' not found"))?;

    // Inline the icon clone — extracting this into a helper function
    // returning Image<'static> tripped a lifetime error because
    // default_window_icon() returns Option<&Image<'_>> with a borrowed
    // lifetime tied to the AppHandle. Cloning + handing directly to
    // set_icon avoids the function-boundary lifetime constraint entirely.
    let icon = app
        .default_window_icon()
        .ok_or_else(|| "no default window icon".to_string())?
        .clone();

    tray.set_icon(Some(icon))
        .map_err(|e| format!("set_icon failed: {e}"))?;
    Ok(())
}
