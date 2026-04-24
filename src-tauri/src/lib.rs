mod backend;

use tauri::window::Color;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let background = Color(248, 247, 244, 255);
                let _ = window.set_background_color(Some(background));
            }
            let state = backend::AppState::new(&app.handle());
            backend::spawn_registry_watcher(app.handle().clone(), state.dirs.accounts_dir.clone());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend::get_app_snapshot,
            backend::run_codex_auth_status,
            backend::refresh_registry_snapshot,
            backend::switch_account,
            backend::remove_account,
            backend::set_account_alias,
            backend::import_auth_file,
            backend::import_auth_directory,
            backend::import_cpa,
            backend::rebuild_registry,
            backend::set_auto_switch,
            backend::set_usage_api_mode,
            backend::record_ui_event,
            backend::launch_login,
            backend::open_diagnostic_path
        ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
