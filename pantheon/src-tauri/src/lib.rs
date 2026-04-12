// Pantheon Tauri v2 entry point. Per mx-tauri-core: ALL builder logic lives
// in lib.rs (not main.rs) so the same code can compile for mobile targets.
// main.rs is just `pantheon_lib::run()`.

mod daemon_client;
mod menu;
mod pty;
mod tray;

use tauri::{RunEvent, WindowEvent};
// T-020 crash-loop diagnostic: tauri_plugin_prevent_default disabled.
// Gemini's diagnosis (9/10 confidence) identifies this plugin's script
// injection as the likely cause of a WKWebView WebProcess crash loop.
// We prove or disprove by ripping it out in one step.
// use tauri_plugin_prevent_default::Flags;

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
        // T-016: register the PtyState singleton so the pty_* commands
        // can atomically manage the shared PTY handle. Default constructs
        // an empty `Option<PtyHandle>` — the first pty_spawn invoke fills
        // it in.
        .manage(pty::PtyState::default())
        .plugin(tauri_plugin_opener::init())
        // T-020 crash-loop diagnostic: plugin disabled. See note at the
        // top of the file. Restore by re-enabling the import and the
        // `.plugin(tauri_plugin_prevent_default::Builder...)` call below.
        //
        // .plugin(
        //     tauri_plugin_prevent_default::Builder::new()
        //         .with_flags(Flags::FIND)
        //         .build(),
        // )
        // T-015: install the custom native menu (no Find/Replace).
        .menu(|handle| menu::build_menu(handle))
        .setup(|app| {
            // T-014: install the menubar tray icon at startup. The tray
            // outlives every window — closing the window hides it (see
            // the on_window_event handler below) and the tray is what
            // brings it back via left-click.
            tray::init_tray(app)?;

            // T-020: spawn the daemon client background task. Connects
            // to the v3.9.0 daemon over REST + /ws/v2, maintains the
            // 4-state health machine, and drives the tray icon via
            // `tray::update_daemon_state`. Failure to load the token
            // file (daemon not running) is non-fatal — we log it and
            // let the UI show Disconnected until the user starts the
            // daemon, at which point the user can manually restart
            // Pantheon. Wave 7 adds an in-app "Start daemon" button.
            if let Err(err) = daemon_client::spawn(app.handle().clone()) {
                tracing::warn!(error = %err, "daemon_client failed to spawn");
            }
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
        .invoke_handler(tauri::generate_handler![
            greet,
            tray::update_daemon_state,
            // T-016: PTY host commands for the xterm.js terminal panel.
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_kill,
        ])
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
