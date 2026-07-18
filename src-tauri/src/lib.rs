mod auth;
mod commands;
mod config;
mod secrets;
mod store;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(commands::AuthState::default())
        .setup(|_app| {
            store::init().expect("falha ao inicializar o banco de dados");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_auth,
            commands::finish_auth_manual,
            commands::list_accounts,
            commands::remove_account,
            commands::test_account,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
