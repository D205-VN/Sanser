mod commands;
mod engine;
mod error;
mod models;
mod storage;

use engine::EngineManager;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
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
