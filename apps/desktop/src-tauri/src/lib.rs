mod commands;
mod engine;
mod error;
mod models;
mod storage;

use engine::EngineManager;

/// Starts the native Sanser desktop shell and blocks until its event loop exits.
///
/// # Errors
///
/// Returns an error when Tauri cannot initialize or run the application.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(EngineManager::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime_status,
            commands::load_preferences,
            commands::save_preferences,
            commands::secure_get,
            commands::secure_set,
            commands::secure_delete,
            commands::launch_engine,
            commands::stop_engine,
            commands::export_diagnostics
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
