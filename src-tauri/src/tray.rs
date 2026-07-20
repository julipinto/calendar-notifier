//! Ícone na bandeja (tray): mostra o próximo evento no tooltip e um menu
//! (sincronizar / abrir / sair).
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

use crate::store;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let sync_i = MenuItem::with_id(app, "sync", "Sincronizar agora", true, None::<&str>)?;
    let show_i = MenuItem::with_id(app, "show", "Abrir", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&sync_i, &show_i, &quit_i])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Calendar Notifier")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "sync" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::commands::do_sync().await;
                    let _ = app.emit("events-updated", 0);
                    update_tray(&app);
                });
            }
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Atualiza o tooltip do tray com o próximo evento.
pub fn update_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        let tip = match store::upcoming_events(1) {
            Ok(evs) if !evs.is_empty() => format!("Próximo: {}", evs[0].title),
            _ => "Calendar Notifier — sem eventos".to_string(),
        };
        let _ = tray.set_tooltip(Some(&tip));
    }
}

/// Mostra e foca a janela principal. Se ela foi destruída (para liberar memória
/// em background), recria — o WebView sobe de novo.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    } else {
        match tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
            .title("Calendar Notifier")
            .inner_size(800.0, 600.0)
            .build()
        {
            Ok(w) => {
                let _ = w.set_focus();
            }
            Err(e) => eprintln!("[tray] falha ao recriar a janela: {e}"),
        }
    }
}
