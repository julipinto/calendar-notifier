use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;

use crate::{auth, config, google, scheduler, secrets, store};

/// True se o erro é de rede (sem internet / timeout).
fn is_network_err(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<reqwest::Error>()
            .map(|re| re.is_connect() || re.is_timeout())
            .unwrap_or(false)
    })
}

/// Traduz erros técnicos em mensagens amigáveis (sem internet, token, etc.).
pub(crate) fn friendly_err(e: &anyhow::Error) -> String {
    if is_network_err(e) {
        return "Sem conexão com a internet (não consegui falar com o Google). \
                Vou tentar de novo na próxima sincronização."
            .to_string();
    }
    for cause in e.chain() {
        if let Some(re) = cause.downcast_ref::<reqwest::Error>() {
            if let Some(s) = re.status() {
                if matches!(s.as_u16(), 400 | 401 | 403) {
                    return "Autorização expirada ou revogada. Reconecte a conta.".to_string();
                }
            }
        }
    }
    e.to_string()
}

/// Obtém um access_token novo para a conta (via refresh_token). Se o refresh
/// falhar por motivo que não seja rede, marca a conta como "precisa reconectar".
async fn access_token_for(email: &str) -> Result<String, String> {
    let creds = config::client_creds();
    let rt = secrets::get_refresh_token(email)
        .map_err(|e| e.to_string())?
        .ok_or("conta sem refresh token — reconecte")?;
    match auth::refresh_access_token(&creds, &rt).await {
        Ok((at, _)) => {
            let _ = store::set_account_reauth(email, false);
            Ok(at)
        }
        Err(e) => {
            if !is_network_err(&e) {
                let _ = store::set_account_reauth(email, true);
            }
            Err(friendly_err(&e))
        }
    }
}

/// Busca os calendários da conta e os salva. Calendários novos: só o principal
/// já vem marcado para acompanhar (`selected`); a escolha do usuário é preservada.
async fn sync_calendars_for(email: &str) -> Result<(), String> {
    let at = access_token_for(email).await?;
    let cals = google::list_calendars(&at)
        .await
        .map_err(|e| friendly_err(&e))?;
    for c in cals {
        store::upsert_calendar(email, &c.id, &c.summary, c.primary, c.primary, &c.color)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Janela de sincronização: de 30 dias atrás até +30 dias, em RFC3339.
/// (o passado alimenta a visão de mês; a lista/notificações usam só o futuro)
fn sync_window() -> (String, String) {
    let now = chrono::Utc::now();
    let from = now - chrono::Duration::days(30);
    let to = now + chrono::Duration::days(30);
    (from.to_rfc3339(), to.to_rfc3339())
}

#[derive(Serialize, Clone)]
pub struct AccountInfo {
    pub email: String,
    pub display_name: String,
    #[serde(default)]
    pub needs_reauth: bool,
}

/// Contexto PKCE da autorização em andamento, guardado entre `start_auth` e a
/// conclusão manual (`finish_auth_manual`).
#[derive(Clone)]
struct PendingCtx {
    verifier: String,
    state: String,
    redirect_uri: String,
}

#[derive(Default)]
pub struct AuthState(Mutex<Option<PendingCtx>>);

fn save_connected(c: &auth::Connected) -> Result<(), String> {
    secrets::save_refresh_token(&c.email, &c.refresh_token).map_err(|e| e.to_string())?;
    store::upsert_account(&c.email, &c.display_name).map_err(|e| e.to_string())?;
    Ok(())
}

/// Inicia o fluxo OAuth. Retorna a URL de consentimento (a UI mostra/abre).
/// Em paralelo, escuta o loopback (caminho automático — funciona no Windows);
/// no sucesso emite `account-connected`, no erro `auth-error`.
#[tauri::command]
pub async fn start_auth(
    app: AppHandle,
    auth_state: State<'_, AuthState>,
) -> Result<String, String> {
    let creds = config::client_creds();
    let pending = auth::begin(&creds).await.map_err(|e| e.to_string())?;
    let url = pending.auth_url.clone();

    // guarda o contexto p/ a conclusão manual (fallback do WSL)
    *auth_state.0.lock().unwrap() = Some(PendingCtx {
        verifier: pending.verifier.clone(),
        state: pending.state.clone(),
        redirect_uri: pending.redirect_uri.clone(),
    });

    let _ = open::that(&url); // best-effort (não funciona no WSL)

    // caminho automático: escuta o loopback em background
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let creds = config::client_creds();
        match auth::finish(&creds, pending).await {
            Ok(c) => {
                if let Err(e) = save_connected(&c) {
                    let _ = app2.emit("auth-error", e);
                    return;
                }
                let _ = sync_calendars_for(&c.email).await; // busca calendários
                let _ = app2.emit(
                    "account-connected",
                    AccountInfo {
                        email: c.email,
                        display_name: c.display_name,
                        needs_reauth: false,
                    },
                );
            }
            Err(e) => {
                // timeout no WSL é esperado — a UI oferece a conclusão manual.
                eprintln!("[auth] loopback não concluiu: {e}");
            }
        }
    });

    Ok(url)
}

/// Conclusão manual: recebe a URL de redirect completa (colada da barra de
/// endereço) e finaliza a troca de tokens. Fallback confiável no WSL.
#[tauri::command]
pub async fn finish_auth_manual(
    redirect_url: String,
    auth_state: State<'_, AuthState>,
) -> Result<AccountInfo, String> {
    let ctx = auth_state
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or("nenhuma autorização em andamento — clique em Conectar primeiro")?;

    let parsed = url::Url::parse(redirect_url.trim())
        .map_err(|_| "URL inválida — cole a URL completa (começa com http://127.0.0.1)")?;
    let (mut code, mut st) = (None, None);
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.to_string()),
            "state" => st = Some(v.to_string()),
            _ => {}
        }
    }
    let code =
        code.ok_or("a URL não tem 'code' — copie a URL de redirect (127.0.0.1/?code=...)")?;
    if st.as_deref() != Some(ctx.state.as_str()) {
        return Err("state não confere — reinicie a conexão".into());
    }

    let creds = config::client_creds();
    let c = auth::exchange(&creds, &code, &ctx.verifier, &ctx.redirect_uri)
        .await
        .map_err(|e| e.to_string())?;
    save_connected(&c)?;
    let _ = sync_calendars_for(&c.email).await;

    Ok(AccountInfo {
        email: c.email,
        display_name: c.display_name,
        needs_reauth: false,
    })
}

#[tauri::command]
pub fn list_accounts() -> Result<Vec<AccountInfo>, String> {
    let accs = store::list_accounts().map_err(|e| e.to_string())?;
    Ok(accs
        .into_iter()
        .map(|a| AccountInfo {
            email: a.email,
            display_name: a.display_name,
            needs_reauth: a.needs_reauth,
        })
        .collect())
}

#[tauri::command]
pub fn remove_account(email: String) -> Result<(), String> {
    secrets::delete_refresh_token(&email).map_err(|e| e.to_string())?;
    store::delete_account(&email).map_err(|e| e.to_string())?;
    Ok(())
}

/// Validação: faz refresh do token e conta os calendários acessíveis.
#[tauri::command]
pub async fn test_account(email: String) -> Result<String, String> {
    let creds = config::client_creds();
    let rt = secrets::get_refresh_token(&email)
        .map_err(|e| e.to_string())?
        .ok_or("conta sem refresh token — reconecte")?;

    let (access, _exp) = auth::refresh_access_token(&creds, &rt)
        .await
        .map_err(|e| e.to_string())?;

    let client = reqwest::Client::new();
    let v: serde_json::Value = client
        .get("https://www.googleapis.com/calendar/v3/users/me/calendarList")
        .bearer_auth(&access)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let n = v
        .get("items")
        .and_then(|i| i.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    Ok(format!("{n} calendário(s) acessível(is)"))
}

// ---------- Fase 2: calendários e eventos ----------

/// Rebusca os calendários da conta no Google e devolve a lista atualizada.
#[tauri::command]
pub async fn refresh_calendars(email: String) -> Result<Vec<store::Calendar>, String> {
    sync_calendars_for(&email).await?;
    store::list_calendars(&email).map_err(|e| e.to_string())
}

/// Lista os calendários de uma conta (do cache local).
#[tauri::command]
pub fn account_calendars(email: String) -> Result<Vec<store::Calendar>, String> {
    store::list_calendars(&email).map_err(|e| e.to_string())
}

/// Marca/desmarca um calendário para acompanhar. Ao desmarcar, remove os
/// eventos dele do cache (para não notificar de calendário que não acompanho).
#[tauri::command]
pub fn set_calendar_selected(
    email: String,
    calendar_id: String,
    selected: bool,
) -> Result<(), String> {
    store::set_calendar_selected(&email, &calendar_id, selected).map_err(|e| e.to_string())?;
    if !selected {
        store::delete_events_for_calendar(&email, &calendar_id).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- Fase 3: configurações e notificações ----------

// ---------- avisos (múltiplos, minutos antes) ----------

fn parse_minutes_csv(s: &str) -> Vec<i64> {
    // aceita qualquer separador (vírgula, espaço, etc.) — extrai os números
    let mut v: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|x| !x.is_empty())
        .filter_map(|x| x.parse::<i64>().ok())
        .map(|m| m.clamp(0, 1440))
        .collect();
    v.sort_unstable_by(|a, b| b.cmp(a)); // maiores antes (10, depois 2…)
    v.dedup();
    v
}

/// Avisos globais (lista de minutos antes do evento). Ex.: [10, 2].
#[tauri::command]
pub fn get_reminders() -> Result<Vec<i64>, String> {
    let s = store::get_setting("lead_minutes", scheduler::DEFAULT_LEAD).map_err(|e| e.to_string())?;
    let v = parse_minutes_csv(&s);
    Ok(if v.is_empty() { vec![10] } else { v })
}

#[tauri::command]
pub fn set_reminders(minutes: Vec<i64>) -> Result<(), String> {
    let v = if minutes.is_empty() { vec![10] } else { minutes };
    let clamped: Vec<String> = v.iter().map(|m| (*m).clamp(0, 1440).to_string()).collect();
    store::set_setting("lead_minutes", &clamped.join(",")).map_err(|e| e.to_string())
}

/// Avisos específicos de uma conta (None/vazio = herda os globais).
#[tauri::command]
pub fn get_account_reminders(email: String) -> Result<Option<Vec<i64>>, String> {
    let s = store::get_setting(&format!("lead:{email}"), "").map_err(|e| e.to_string())?;
    let v = parse_minutes_csv(&s);
    Ok(if v.is_empty() { None } else { Some(v) })
}

#[tauri::command]
pub fn set_account_reminders(email: String, minutes: Option<Vec<i64>>) -> Result<(), String> {
    let val = match minutes {
        Some(m) if !m.is_empty() => m
            .iter()
            .map(|x| (*x).clamp(0, 1440).to_string())
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(), // vazio = herda os globais
    };
    store::set_setting(&format!("lead:{email}"), &val).map_err(|e| e.to_string())
}

/// Dispara uma notificação de teste (para validar o canal do SO).
#[tauri::command]
pub fn test_notification(app: AppHandle) -> Result<(), String> {
    let sound_on = store::get_setting("sound_enabled", "true")
        .map(|v| v != "false")
        .unwrap_or(true);
    let mut b = app
        .notification()
        .builder()
        .title("Calendar Notifier")
        .body("Notificação de teste ✓");
    if sound_on {
        b = b.sound(scheduler::NOTIF_SOUND);
    }
    b.show().map_err(|e| e.to_string())
}

/// Liga/desliga o som das notificações.
#[tauri::command]
pub fn get_sound_enabled() -> Result<bool, String> {
    Ok(store::get_setting("sound_enabled", "true").map_err(|e| e.to_string())? != "false")
}

#[tauri::command]
pub fn set_sound_enabled(enabled: bool) -> Result<(), String> {
    store::set_setting("sound_enabled", if enabled { "true" } else { "false" })
        .map_err(|e| e.to_string())
}

/// Iniciar o app junto com o sistema operacional (autostart no login).
#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let m = app.autolaunch();
    if enabled {
        m.enable().map_err(|e| e.to_string())
    } else {
        m.disable().map_err(|e| e.to_string())
    }
}

/// Núcleo da sincronização (janela de 30d) de todos os calendários marcados.
/// Reusado pelo comando `sync_now` e pelo poller automático.
pub(crate) async fn do_sync() -> Result<u32, String> {
    let cals = store::selected_calendars().map_err(|e| e.to_string())?;
    let mut by_acct: HashMap<String, Vec<store::Calendar>> = HashMap::new();
    for c in cals {
        by_acct.entry(c.account_email.clone()).or_default().push(c);
    }

    let (tmin, tmax) = sync_window();
    let mut total = 0u32;
    for (email, cals) in by_acct {
        let at = access_token_for(&email).await?;
        for c in cals {
            let evs = google::fetch_events(&at, &c.id, &tmin, &tmax)
                .await
                .map_err(|e| friendly_err(&e))?;
            let mapped: Vec<store::Event> = evs
                .into_iter()
                .map(|e| store::Event {
                    id: e.id,
                    calendar_id: c.id.clone(),
                    account_email: email.clone(),
                    title: e.title,
                    start_ts: e.start_ts,
                    end_ts: e.end_ts,
                    all_day: e.all_day,
                    status: e.status,
                    html_link: e.html_link,
                    declined: e.declined,
                })
                .collect();
            total += mapped.len() as u32;
            store::replace_events(&email, &c.id, &mapped).map_err(|e| e.to_string())?;
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = store::set_setting("last_sync_ts", &now.to_string());
    Ok(total)
}

/// Timestamp (epoch segundos) da última sincronização bem-sucedida; 0 se nunca.
#[tauri::command]
pub fn get_last_sync() -> Result<i64, String> {
    store::get_setting("last_sync_ts", "0")
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|_| "valor inválido".to_string())
}

/// Sincroniza os eventos de todos os calendários marcados (acionado pela UI).
#[tauri::command]
pub async fn sync_now() -> Result<u32, String> {
    do_sync().await
}

/// Intervalo do polling automático (minutos).
#[tauri::command]
pub fn get_poll_minutes() -> Result<i64, String> {
    store::get_setting("poll_minutes", scheduler::DEFAULT_POLL)
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|_| "intervalo inválido".to_string())
}

#[tauri::command]
pub fn set_poll_minutes(minutes: i64) -> Result<(), String> {
    let m = minutes.clamp(1, 1440);
    store::set_setting("poll_minutes", &m.to_string()).map_err(|e| e.to_string())
}

/// Eventos em cache (passados + futuros, janela de ±30d), com os filtros.
/// A lista mostra só os futuros; a visão de mês usa todos (filtro no front).
#[tauri::command]
pub fn list_events() -> Result<Vec<store::UpcomingEvent>, String> {
    let ignore_declined = pref_bool("ignore_declined", true);
    let ignore_all_day = pref_bool("ignore_all_day", false);
    let evs = store::all_events(2000).map_err(|e| e.to_string())?;
    Ok(evs
        .into_iter()
        .filter(|e| !(ignore_declined && e.declined))
        .filter(|e| !(ignore_all_day && e.all_day))
        .collect())
}

// ---------- preferências (bool) ----------

fn pref_bool(key: &str, default: bool) -> bool {
    store::get_setting(key, if default { "true" } else { "false" })
        .map(|v| v != "false")
        .unwrap_or(default)
}

#[tauri::command]
pub fn get_ignore_declined() -> Result<bool, String> {
    Ok(pref_bool("ignore_declined", true))
}
#[tauri::command]
pub fn set_ignore_declined(enabled: bool) -> Result<(), String> {
    store::set_setting("ignore_declined", if enabled { "true" } else { "false" })
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn get_ignore_all_day() -> Result<bool, String> {
    Ok(pref_bool("ignore_all_day", false))
}
#[tauri::command]
pub fn set_ignore_all_day(enabled: bool) -> Result<(), String> {
    store::set_setting("ignore_all_day", if enabled { "true" } else { "false" })
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn get_start_minimized() -> Result<bool, String> {
    Ok(pref_bool("start_minimized", true))
}
#[tauri::command]
pub fn set_start_minimized(enabled: bool) -> Result<(), String> {
    store::set_setting("start_minimized", if enabled { "true" } else { "false" })
        .map_err(|e| e.to_string())
}

// ---------- resumo diário ----------

#[tauri::command]
pub fn get_daily_summary_enabled() -> Result<bool, String> {
    Ok(pref_bool("daily_summary_enabled", false))
}
#[tauri::command]
pub fn set_daily_summary_enabled(enabled: bool) -> Result<(), String> {
    store::set_setting("daily_summary_enabled", if enabled { "true" } else { "false" })
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn get_daily_summary_time() -> Result<String, String> {
    store::get_setting("daily_summary_time", scheduler::DEFAULT_SUMMARY_TIME)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn set_daily_summary_time(time: String) -> Result<(), String> {
    let valid = time
        .split_once(':')
        .map(|(h, m)| {
            h.parse::<u32>().map(|h| h < 24).unwrap_or(false)
                && m.parse::<u32>().map(|m| m < 60).unwrap_or(false)
        })
        .unwrap_or(false);
    if !valid {
        return Err("horário inválido (use HH:MM)".into());
    }
    store::set_setting("daily_summary_time", &time).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_minutes_csv;

    #[test]
    fn csv_sorted_dedup_clamped() {
        assert_eq!(parse_minutes_csv("2,10,10"), vec![10, 2]);
        assert_eq!(parse_minutes_csv("99999"), vec![1440]);
        assert_eq!(parse_minutes_csv(""), Vec::<i64>::new());
    }
}
