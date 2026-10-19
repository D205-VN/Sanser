mod commands;
mod connection_report;
mod device_identity;
mod direct;
mod engine;
mod engine_output;
mod error;
mod fec_health;
mod models;
mod relay;
mod sharing_engine;
mod storage;
mod tray;
mod voice;
mod windows_uac;

use engine::EngineManager;
use tauri::Manager;

/// Starts the native Sanser desktop shell and blocks until its event loop exits.
///
/// # Errors
///
/// Returns an error when Tauri cannot initialize or run the application.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(debug_assertions)]
    eprintln!("Sanser lifecycle: process started");
    let mut context = tauri::generate_context!();
    if cfg!(target_os = "windows") {
        tray::configure_background_timers(context.config_mut());
    }
    let builder = tauri::Builder::default();
    // Register before every other plugin and before setup creates the tray or
    // webview. A desktop shortcut restores the existing background process.
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        #[cfg(debug_assertions)]
        eprintln!("Sanser lifecycle: second launch received");
        tray::show_main(app);
    }));
    builder
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(EngineManager::default())
        .manage(commands::P2pSessionManager::default())
        .manage(relay::RelayManager::default())
        .setup(|app| {
            if cfg!(target_os = "windows") {
                tray::install(app)?;
            }
            #[cfg(debug_assertions)]
            eprintln!("Sanser lifecycle: setup completed");
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(debug_assertions)]
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                eprintln!("Sanser lifecycle: close event label={}", window.label());
            }
            if cfg!(target_os = "windows")
                && window.label() == "main"
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                // Keep the webview, host polling and native engines alive. The
                // tray is installed before this handler can hide the window.
                api.prevent_close();
                #[cfg(debug_assertions)]
                eprintln!("Sanser lifecycle: hide requested");
                // Leave the native close callback before changing WebView2's
                // visibility, which can synchronously send more window events.
                let window = window.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = window.hide() {
                        eprintln!("Unable to hide Sanser in the system tray: {error}");
                    }
                    #[cfg(debug_assertions)]
                    eprintln!("Sanser lifecycle: hide queued");
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            windows_uac::windows_uac_status,
            windows_uac::configure_windows_uac,
            commands::get_runtime_status,
            device_identity::get_device_identity,
            device_identity::get_login_device,
            commands::load_preferences,
            commands::save_preferences,
            commands::secure_get,
            commands::secure_set,
            commands::secure_delete,
            commands::get_local_route_address,
            commands::launch_engine,
            commands::stop_engine,
            commands::set_viewer_audio,
            commands::set_group_voice,
            commands::get_group_voice_status,
            commands::sharing_engine_add_peer,
            commands::sharing_engine_remove_peer,
            commands::sharing_engine_peer_status,
            commands::get_engine_status,
            commands::route_recovery_plan,
            commands::export_diagnostics,
            commands::p2p_stop,
            commands::p2p_gather,
            commands::p2p_punch,
            relay::relay_start,
            relay::relay_peer_failure,
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
