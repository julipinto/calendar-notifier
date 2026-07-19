//! Loop leve que verifica periodicamente os eventos entrando na janela de aviso
//! e dispara notificações do sistema (uma vez por evento).
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::store;

const TICK: Duration = Duration::from_secs(30);
pub const DEFAULT_LEAD: &str = "10";
pub const DEFAULT_POLL: &str = "60";

/// Inicia o loop de notificações em background.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&app) {
                eprintln!("[scheduler] erro no tick: {e}");
            }
        }
    });
}

/// Inicia o polling periódico: sincroniza logo ao subir e depois a cada
/// `poll_minutes` (padrão 5). Emite `events-updated` e atualiza o tray.
pub fn start_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            match crate::commands::do_sync().await {
                Ok(n) => {
                    let _ = app.emit("events-updated", n);
                    crate::tray::update_tray(&app);
                }
                Err(e) => eprintln!("[poller] sync falhou: {e}"),
            }
            let mins: u64 = store::get_setting("poll_minutes", DEFAULT_POLL)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5)
                .max(1);
            tokio::time::sleep(Duration::from_secs(mins * 60)).await;
        }
    });
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn tick(app: &AppHandle) -> anyhow::Result<()> {
    let lead: i64 = store::get_setting("lead_minutes", DEFAULT_LEAD)?
        .parse()
        .unwrap_or(10);

    let sound_on = store::get_setting("sound_enabled", "true")
        .map(|v| v != "false")
        .unwrap_or(true);

    for ev in store::due_notifications(lead)? {
        let mins = ((ev.start_ts - now()) as f64 / 60.0).ceil().max(0.0) as i64;
        let body = if mins <= 1 {
            "Começa em instantes".to_string()
        } else {
            format!("Começa em {mins} min")
        };
        let mut b = app.notification().builder().title(&ev.title).body(&body);
        if sound_on {
            b = b.sound("Default");
        }
        let _ = b.show();
        store::mark_notified(&ev.account_email, &ev.calendar_id, &ev.id)?;
    }
    Ok(())
}
