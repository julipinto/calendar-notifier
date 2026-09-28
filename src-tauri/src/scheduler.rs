//! Loop leve que verifica periodicamente os eventos entrando na janela de aviso
//! e dispara notificações do sistema (uma vez por evento).
use chrono::{Local, TimeZone};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;

use crate::store;

const TICK: Duration = Duration::from_secs(30);
pub const DEFAULT_LEAD: &str = "10";

// nome do som da notificação por plataforma:
// - Windows/macOS: "Default" (som padrão do SO)
// - Linux: nome do freedesktop sound theme (o "Default" não existe lá)
#[cfg(target_os = "linux")]
pub const NOTIF_SOUND: &str = "message-new-instant";
#[cfg(not(target_os = "linux"))]
pub const NOTIF_SOUND: &str = "Default";
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
            match crate::commands::do_sync(&app).await {
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

/// Dispara uma notificação. No Linux usa `notify-rust` (permite clique → abrir
/// o evento e notificação "fixa"/urgente); nas demais plataformas usa o plugin
/// do Tauri (que não expõe clique/urgência no desktop).
#[allow(unused_variables)]
fn notify(app: &AppHandle, title: &str, body: &str, sound_on: bool, click_url: Option<String>, sticky: bool) {
    #[cfg(target_os = "linux")]
    {
        let (title, body) = (title.to_string(), body.to_string());
        // thread própria: wait_for_action bloqueia até clicar/fechar
        std::thread::spawn(move || {
            let mut n = notify_rust::Notification::new();
            n.summary(&title).body(&body);
            if sound_on {
                n.sound_name("message-new-instant");
            }
            if sticky {
                n.hint(notify_rust::Hint::Urgency(notify_rust::Urgency::Critical));
                n.timeout(notify_rust::Timeout::Never);
            }
            if click_url.is_some() {
                n.action("default", "Abrir");
            }
            match n.show() {
                Ok(handle) => {
                    if let Some(url) = click_url {
                        handle.wait_for_action(|action| {
                            if action == "default" {
                                let _ = open::that(&url);
                            }
                        });
                    }
                }
                Err(e) => eprintln!("[notif] falha: {e}"),
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut b = app.notification().builder().title(title).body(body);
        if sound_on {
            b = b.sound(NOTIF_SOUND);
        }
        let _ = b.show();
    }
}

/// Monta o link do evento na conta certa (authuser evita cair na conta logada).
fn event_click_url(html_link: &str, account_email: &str) -> Option<String> {
    if html_link.is_empty() {
        return None;
    }
    let sep = if html_link.contains('?') { "&" } else { "?" };
    Some(format!(
        "{html_link}{sep}authuser={}",
        urlencoding::encode(account_email)
    ))
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
            let click = event_click_url(&ev.html_link, &ev.account_email);
            notify(app, &ev.title, &body, sound_on, click, false);
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

fn hhmm(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .unwrap_or_default()
        .with_timezone(&Local)
        .format("%H:%M")
        .to_string()
}

/// "09:00–10:00" (só o início se não houver fim válido).
fn time_range(start_ts: i64, end_ts: i64) -> String {
    if end_ts > start_ts {
        format!("{}–{}", hhmm(start_ts), hhmm(end_ts))
    } else {
        hhmm(start_ts)
    }
}

/// Descreve quando o evento acontece, relativo a `today`: "14:00–15:00",
/// "30/09 às 14:00–15:00", "dia inteiro", "30/09 (dia inteiro)".
fn when_label(ev: &store::Event, today: chrono::NaiveDate) -> String {
    let day = event_day(ev.start_ts, ev.all_day);
    let time = if ev.all_day {
        "dia inteiro".to_string()
    } else {
        time_range(ev.start_ts, ev.end_ts)
    };
    if day == today {
        time
    } else if ev.all_day {
        format!("{} ({time})", day.format("%d/%m"))
    } else {
        format!("{} às {time}", day.format("%d/%m"))
    }
}

/// Mudança detectada na sincronização envolvendo um evento de hoje.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncChange {
    pub title: String,
    pub body: String,
    pub click_url: Option<String>,
}

/// Compara o cache anterior (`old`) com o recém-buscado (`new`) de um calendário
/// e devolve as mudanças que envolvem `today` (antes ou depois): evento novo,
/// excluído, com horário alterado ou movido para/de outro dia.
pub fn diff_today(
    old: &[store::Event],
    new: &[store::Event],
    today: chrono::NaiveDate,
    ignore_declined: bool,
    ignore_all_day: bool,
) -> Vec<SyncChange> {
    use std::collections::HashMap;
    let is_today = |e: &store::Event| event_day(e.start_ts, e.all_day) == today;
    let skip = |e: &store::Event| (ignore_declined && e.declined) || (ignore_all_day && e.all_day);
    let change = |e: &store::Event, body: String| SyncChange {
        title: format!("[Sync] {}", e.title),
        body,
        click_url: event_click_url(&e.html_link, &e.account_email),
    };

    let old_by_id: HashMap<&str, &store::Event> = old.iter().map(|e| (e.id.as_str(), e)).collect();
    let new_ids: std::collections::HashSet<&str> = new.iter().map(|e| e.id.as_str()).collect();
    let mut out = Vec::new();

    for n in new {
        match old_by_id.get(n.id.as_str()) {
            None => {
                if is_today(n) && !skip(n) {
                    out.push(change(n, format!("Novo evento hoje: {}", when_label(n, today))));
                }
            }
            Some(o) => {
                if !(is_today(o) || is_today(n)) || skip(n) {
                    continue;
                }
                if o.start_ts != n.start_ts || o.all_day != n.all_day {
                    out.push(change(
                        n,
                        format!(
                            "Migrou para {} (era {})",
                            when_label(n, today),
                            when_label(o, today)
                        ),
                    ));
                } else if o.end_ts != n.end_ts && !n.all_day {
                    out.push(change(
                        n,
                        format!("Agora vai até {} (era até {})", hhmm(n.end_ts), hhmm(o.end_ts)),
                    ));
                }
            }
        }
    }
    for o in old {
        if !new_ids.contains(o.id.as_str()) && is_today(o) && !skip(o) {
            out.push(SyncChange {
                title: format!("[Sync] {}", o.title),
                body: format!("Foi excluído (era {})", when_label(o, today)),
                click_url: None,
            });
        }
    }
    out
}

/// Notifica cada mudança detectada na sincronização.
pub fn notify_sync_changes(app: &AppHandle, changes: &[SyncChange]) {
    if changes.is_empty() {
        return;
    }
    let sound_on = store::get_setting("sound_enabled", "true")
        .map(|v| v != "false")
        .unwrap_or(true);
    for c in changes {
        notify(app, &c.title, &c.body, sound_on, c.click_url.clone(), false);
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
            lines.push(format!("{} {}", time_range(it.start_ts, it.end_ts), it.title));
        }
    }
    if items.len() > 10 {
        lines.push(format!("+{} mais", items.len() - 10));
    }
    let title = format!("Resumo de hoje — {} evento(s)", items.len());
    // resumo "fixo" (urgente) no Linux p/ não sumir rápido; Windows vai p/ a Central
    notify(app, &title, &lines.join("\n"), sound_on, None, true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{diff_today, parse_reminders};
    use crate::store::Event;
    use chrono::{Local, NaiveDate, TimeZone};

    fn ts(d: NaiveDate, h: u32, m: u32) -> i64 {
        Local
            .from_local_datetime(&d.and_hms_opt(h, m, 0).unwrap())
            .single()
            .unwrap()
            .timestamp()
    }

    fn ev(id: &str, title: &str, start: i64, end: i64) -> Event {
        Event {
            id: id.into(),
            calendar_id: "c".into(),
            account_email: "a@x.com".into(),
            title: title.into(),
            start_ts: start,
            end_ts: end,
            all_day: false,
            status: "confirmed".into(),
            html_link: String::new(),
            declined: false,
            meet_link: String::new(),
        }
    }

    #[test]
    fn diff_today_detects_moves_deletes_and_new() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let tomorrow = today.succ_opt().unwrap();
        let old = vec![
            ev("1", "Cupom de desconto", ts(today, 10, 0), ts(today, 11, 0)),
            ev("2", "Daily", ts(today, 9, 0), ts(today, 9, 15)),
            ev("3", "1:1", ts(today, 15, 0), ts(today, 15, 30)),
            ev("4", "Sem mudança", ts(today, 16, 0), ts(today, 17, 0)),
            ev("5", "Amanhã", ts(tomorrow, 10, 0), ts(tomorrow, 11, 0)),
        ];
        let new = vec![
            ev("1", "Cupom de desconto", ts(today, 14, 0), ts(today, 15, 0)),
            ev("3", "1:1", ts(tomorrow, 15, 0), ts(tomorrow, 15, 30)),
            ev("4", "Sem mudança", ts(today, 16, 0), ts(today, 17, 0)),
            ev("5", "Amanhã", ts(tomorrow, 11, 0), ts(tomorrow, 12, 0)),
            ev("6", "Novo", ts(today, 18, 0), ts(today, 18, 30)),
        ];
        let got: Vec<(String, String)> = diff_today(&old, &new, today, true, false)
            .into_iter()
            .map(|c| (c.title, c.body))
            .collect();
        assert_eq!(
            got,
            vec![
                ("[Sync] Cupom de desconto".into(), "Migrou para 14:00–15:00 (era 10:00–11:00)".into()),
                ("[Sync] 1:1".into(), "Migrou para 29/09 às 15:00–15:30 (era 15:00–15:30)".into()),
                ("[Sync] Novo".into(), "Novo evento hoje: 18:00–18:30".into()),
                ("[Sync] Daily".into(), "Foi excluído (era 09:00–09:15)".into()),
            ]
        );
    }

    #[test]
    fn reminders_sorted_desc_dedup_and_parse() {
        assert_eq!(parse_reminders("2,10,10,5"), vec![10, 5, 2]);
        assert_eq!(parse_reminders("10"), vec![10]);
        assert_eq!(parse_reminders(""), Vec::<i64>::new());
        assert_eq!(parse_reminders(" 3 , x , 1 "), vec![3, 1]);
    }
}
