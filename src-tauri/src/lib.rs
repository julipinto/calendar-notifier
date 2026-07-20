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
        // single-instance deve ser o primeiro plugin. 2ª instância → foca a janela.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        // autostart lança com "--minimized" p/ o app poder iniciar na bandeja
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--minimized"])
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(commands::AuthState::default())
        .setup(|app| {
            use tauri::Manager;
            store::init().expect("falha ao inicializar o banco de dados");
            // tray não-fatal: no WSLg pode não haver host de bandeja
            if let Err(e) = tray::build(&app.handle().clone()) {
                eprintln!("[tray] não foi possível criar o tray: {e}");
            }
            scheduler::start(app.handle().clone()); // loop de notificações
            scheduler::start_poller(app.handle().clone()); // sync periódico

            // iniciar em segundo plano? (autostart passou "--minimized" E a
            // preferência start_minimized está ligada). Senão, mostra a janela.
            let launched_minimized = std::env::args().any(|a| a == "--minimized");
            let want_minimized = store::get_setting("start_minimized", "true")
                .map(|v| v != "false")
                .unwrap_or(true);
            if !(launched_minimized && want_minimized) {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
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
            commands::test_notification,
            commands::get_poll_minutes,
            commands::set_poll_minutes,
            commands::get_sound_enabled,
            commands::set_sound_enabled,
            commands::get_last_sync,
            commands::get_autostart,
            commands::set_autostart,
            commands::get_reminders,
            commands::set_reminders,
            commands::get_account_reminders,
            commands::set_account_reminders,
            commands::get_ignore_declined,
            commands::set_ignore_declined,
            commands::get_ignore_all_day,
            commands::set_ignore_all_day,
            commands::get_start_minimized,
            commands::set_start_minimized,
            commands::get_daily_summary_enabled,
            commands::set_daily_summary_enabled,
            commands::get_daily_summary_time,
            commands::set_daily_summary_time,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
