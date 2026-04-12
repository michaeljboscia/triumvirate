// Pantheon Tauri v2 entry point. The thin-main pattern from create-tauri-app
// puts setup logic here in lib.rs so the same code can compile for mobile
// targets in the future. main.rs is just `pantheon_lib::run()`.

mod tray;

use tauri::{Manager, WindowEvent};

// Smoke-test command from the scaffold; kept so the frontend can verify the
// IPC bridge works during early development. Will be removed once real
// daemon-client commands replace it in T-020.
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // T-014: install the menubar tray icon at startup. The tray
            // outlives every window — closing the window hides it (see the
            // close-to-tray handler below) and the tray is what brings it
            // back via left-click.
            tray::init_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // T-014 close-to-tray: intercept the user clicking the red "close"
            // button on the main window. Instead of exiting the app, hide the
            // window — the tray icon stays alive and the user can click it
            // (or run `pantheon` from the CLI in T-025) to bring the window
            // back. Quit happens explicitly via the tray's Quit menu item or
            // Cmd+Q (which routes through Tauri's app handle).
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![greet, tray::update_daemon_state])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
