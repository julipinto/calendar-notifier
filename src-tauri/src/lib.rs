mod auth;
mod commands;
mod config;
mod google;
mod scheduler;
mod secrets;
mod store;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(commands::AuthState::default())
        .setup(|app| {
            store::init().expect("falha ao inicializar o banco de dados");
            // tray não-fatal: no WSLg pode não haver host de bandeja
            if let Err(e) = tray::build(&app.handle().clone()) {
                eprintln!("[tray] não foi possível criar o tray: {e}");
            }
            scheduler::start(app.handle().clone()); // loop de notificações
            scheduler::start_poller(app.handle().clone()); // sync periódico
            Ok(())
        })
        .on_window_event(|window, event| {
            // fechar a janela esconde na bandeja em vez de encerrar o app
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_auth,
            commands::finish_auth_manual,
            commands::list_accounts,
            commands::remove_account,
            commands::test_account,
            commands::refresh_calendars,
            commands::account_calendars,
            commands::set_calendar_selected,
            commands::sync_now,
            commands::list_events,
            commands::get_lead_minutes,
            commands::set_lead_minutes,
            commands::test_notification,
            commands::get_poll_minutes,
            commands::set_poll_minutes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
