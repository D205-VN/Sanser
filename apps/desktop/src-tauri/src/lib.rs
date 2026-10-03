mod commands;
mod connection_report;
mod device_identity;
mod engine;
mod engine_output;
mod error;
mod models;
mod relay;
mod storage;
mod tray;

use engine::EngineManager;
use tauri::Manager;

/// Starts the native Sanser desktop shell and blocks until its event loop exits.
///
/// # Errors
///
/// Returns an error when Tauri cannot initialize or run the application.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut context = tauri::generate_context!();
    if cfg!(target_os = "windows") {
        tray::configure_background_timers(context.config_mut());
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(EngineManager::default())
        .manage(commands::P2pSessionManager::default())
        .manage(relay::RelayManager::default())
        .setup(|app| {
            if cfg!(target_os = "windows") {
                tray::install(app)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if cfg!(target_os = "windows")
                && window.label() == "main"
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                // Keep the webview, host polling and native engines alive. The
                // tray is installed before this handler can hide the window.
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("Unable to hide Sanser in the system tray: {error}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime_status,
            device_identity::get_device_identity,
            commands::load_preferences,
            commands::save_preferences,
            commands::secure_get,
            commands::secure_set,
            commands::secure_delete,
            commands::get_local_route_address,
            commands::launch_engine,
            commands::stop_engine,
            commands::get_engine_status,
            commands::export_diagnostics,
            commands::p2p_stop,
            commands::p2p_gather,
            commands::p2p_punch,
            relay::relay_start,
            relay::relay_stop
        ])
        .build(context)?
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                // Explicit exit (including an updater restart) must stop
                // sidecars before Tauri terminates the process. Drop alone is
                // not sufficient when the event loop calls process::exit.
                for kind in [models::EngineKind::Host, models::EngineKind::Client] {
                    app.state::<relay::RelayManager>().stop_for_engine(kind);
                    if let Err(error) = app.state::<EngineManager>().stop(kind) {
                        eprintln!("Unable to stop Sanser engine on exit: {error}");
                    }
                }
            }
        });
    Ok(())
}
