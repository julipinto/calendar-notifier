//! Loop leve que verifica periodicamente os eventos entrando na janela de aviso
//! e dispara notificações do sistema (uma vez por evento).
use chrono::{Local, TimeZone};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::store;

const TICK: Duration = Duration::from_secs(30);
pub const DEFAULT_LEAD: &str = "10";
pub const DEFAULT_POLL: &str = "60";
pub const DEFAULT_SUMMARY_TIME: &str = "08:00";

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
                Err(e) => {
                    eprintln!("[poller] sync falhou: {e}");
                    let _ = app.emit("sync-error", e);
                }
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

/// Interpreta uma lista de minutos "10,2" / "10 2" → [10, 2] (desc, sem dup).
/// Aceita qualquer separador (extrai os números).
pub fn parse_reminders(s: &str) -> Vec<i64> {
    let mut v: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|x| !x.is_empty())
        .filter_map(|x| x.parse::<i64>().ok())
        .collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.dedup();
    v
}

/// Avisos (lista de minutos) da conta: override por conta, senão os globais.
pub fn account_reminders(email: &str, global: &[i64]) -> Vec<i64> {
    let s = store::get_setting(&format!("lead:{email}"), "").unwrap_or_default();
    let v = parse_reminders(&s);
    if v.is_empty() {
        global.to_vec()
    } else {
        v
    }
}

fn tick(app: &AppHandle) -> anyhow::Result<()> {
    let mut global = parse_reminders(&store::get_setting("lead_minutes", DEFAULT_LEAD)?);
    if global.is_empty() {
        global = vec![10];
    }
    let sound_on = store::get_setting("sound_enabled", "true")
        .map(|v| v != "false")
        .unwrap_or(true);
    let ignore_declined = store::get_setting("ignore_declined", "true")
        .map(|v| v != "false")
        .unwrap_or(true);
    let now = now();

    for ev in store::pending_notifications()? {
        if ignore_declined && ev.declined {
            continue;
        }
        let leads = account_reminders(&ev.account_email, &global);
        let fired: std::collections::HashSet<i64> = ev
            .notified_leads
            .split(',')
            .filter_map(|x| x.trim().parse::<i64>().ok())
            .collect();

        for lead in leads {
            if fired.contains(&lead) {
                continue;
            }
            // janela do aviso alcançada? (start - lead <= agora); start > agora garantido
            if ev.start_ts - lead * 60 > now {
                continue;
            }
            let mins = ((ev.start_ts - now) as f64 / 60.0).ceil().max(0.0) as i64;
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
            store::add_notified_lead(&ev.account_email, &ev.calendar_id, &ev.id, lead)?;
        }
    }

    let _ = maybe_daily_summary(app, sound_on);
    Ok(())
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    Some((h.trim().parse().ok()?, m.trim().parse().ok()?))
}

/// Dia do evento (igual à UI): all-day usa data UTC; com horário usa data local.
fn event_day(start_ts: i64, all_day: bool) -> chrono::NaiveDate {
    let dt = chrono::DateTime::from_timestamp(start_ts, 0).unwrap_or_default();
    if all_day {
        dt.date_naive()
    } else {
        dt.with_timezone(&Local).date_naive()
    }
}

/// Uma vez por dia, no horário configurado, notifica o resumo dos eventos de hoje
/// (todos os tipos). Só dispara se houver eventos. Marca o dia como enviado.
fn maybe_daily_summary(app: &AppHandle, sound_on: bool) -> anyhow::Result<()> {
    if store::get_setting("daily_summary_enabled", "false")? == "false" {
        return Ok(());
    }
    let (hh, mm) = parse_hhmm(&store::get_setting(
        "daily_summary_time",
        DEFAULT_SUMMARY_TIME,
    )?)
    .unwrap_or((8, 0));

    let now = Local::now();
    let today = now.date_naive();
    let scheduled = match today.and_hms_opt(hh, mm, 0) {
        Some(t) => t,
        None => return Ok(()),
    };
    if now.naive_local() < scheduled {
        return Ok(()); // ainda não chegou a hora hoje
    }
    let today_str = today.to_string();
    if store::get_setting("daily_summary_last", "")? == today_str {
        return Ok(()); // já enviado hoje
    }

    // janela ampla (ontem→depois de amanhã) e filtra pelos que são "hoje"
    let from = today
        .pred_opt()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|n| Local.from_local_datetime(&n).single())
        .map(|d| d.timestamp())
        .unwrap_or(0);
    let to = today
        .succ_opt()
        .and_then(|d| d.succ_opt())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|n| Local.from_local_datetime(&n).single())
        .map(|d| d.timestamp())
        .unwrap_or(i64::MAX);

    let items: Vec<store::SummaryItem> = store::events_in_range(from, to)?
        .into_iter()
        .filter(|it| event_day(it.start_ts, it.all_day) == today)
        .collect();

    // marca como processado hoje (mesmo sem eventos, p/ não reprocessar o dia)
    store::set_setting("daily_summary_last", &today_str)?;
    if items.is_empty() {
        return Ok(()); // "caso tenha" — nada hoje, não notifica
    }

    let mut lines: Vec<String> = Vec::new();
    for it in items.iter().take(10) {
        if it.all_day {
            lines.push(format!("• {}", it.title));
        } else {
            let hm = chrono::DateTime::from_timestamp(it.start_ts, 0)
                .unwrap_or_default()
                .with_timezone(&Local)
                .format("%H:%M");
            lines.push(format!("{hm} {}", it.title));
        }
    }
    if items.len() > 10 {
        lines.push(format!("+{} mais", items.len() - 10));
    }
    let title = format!("Resumo de hoje — {} evento(s)", items.len());
    let mut b = app.notification().builder().title(&title).body(&lines.join("\n"));
    if sound_on {
        b = b.sound("Default");
    }
    let _ = b.show();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_reminders;

    #[test]
    fn reminders_sorted_desc_dedup_and_parse() {
        assert_eq!(parse_reminders("2,10,10,5"), vec![10, 5, 2]);
        assert_eq!(parse_reminders("10"), vec![10]);
        assert_eq!(parse_reminders(""), Vec::<i64>::new());
        assert_eq!(parse_reminders(" 3 , x , 1 "), vec![3, 1]);
    }
}
