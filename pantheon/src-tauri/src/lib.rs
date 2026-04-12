// Pantheon Tauri v2 entry point. Per mx-tauri-core: ALL builder logic lives
// in lib.rs (not main.rs) so the same code can compile for mobile targets.
// main.rs is just `pantheon_lib::run()`.

mod menu;
mod tray;

use tauri::{RunEvent, WindowEvent};
use tauri_plugin_prevent_default::Flags;

// Smoke-test command from the scaffold; kept so the frontend can verify the
// IPC bridge works during early development. Removed in T-020 once real
// daemon-client commands replace it.
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Build → setup → on_window_event → run.
    // We use the .build() then .run() variant (instead of the chained
    // `.run(generate_context!())`) so we can intercept Tauri's RunEvent
    // loop and call `api.prevent_exit()` when all windows hide. Without
    // that second half, hide-on-close still terminates the app when the
    // last visible window vanishes — the tray would be left holding a
    // dead app handle. Per mx-tauri-window Level 2 hide-on-close pattern.
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // T-014: install the menubar tray icon at startup. The tray
            // outlives every window — closing the window hides it (see
            // the on_window_event handler below) and the tray is what
            // brings it back via left-click.
            tray::init_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // T-014 close-to-tray (half 1 of 2): intercept the user clicking
            // the red close button on the main window. Hide the window
            // instead of letting it propagate to a destroy.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![greet, tray::update_daemon_state])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| {
        // T-014 close-to-tray (half 2 of 2): when all windows are hidden,
        // Tauri's run loop fires ExitRequested. We block it so the tray
        // icon keeps the app alive. The tray's Quit menu item still works
        // because it calls `app.exit(0)`, which is a forced exit and
        // bypasses ExitRequested entirely.
        if let RunEvent::ExitRequested { api, .. } = event {
            api.prevent_exit();
        }
    });
}
