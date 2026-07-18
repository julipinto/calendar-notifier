//! Loop leve que verifica periodicamente os eventos entrando na janela de aviso
//! e dispara notificações do sistema (uma vez por evento).
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::store;

const TICK: Duration = Duration::from_secs(30);
pub const DEFAULT_LEAD: &str = "10";

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

    for ev in store::due_notifications(lead)? {
        let mins = ((ev.start_ts - now()) as f64 / 60.0).ceil().max(0.0) as i64;
        let body = if mins <= 1 {
            "Começa em instantes".to_string()
        } else {
            format!("Começa em {mins} min")
        };
        let _ = app
            .notification()
            .builder()
            .title(&ev.title)
            .body(&body)
            .show();
        store::mark_notified(&ev.account_email, &ev.calendar_id, &ev.id)?;
    }
    Ok(())
}
