use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_notification::NotificationExt;

use crate::{auth, config, google, scheduler, secrets, store};

/// Traduz erros técnicos em mensagens amigáveis (sem internet, timeout, etc.).
pub(crate) fn friendly_err(e: &anyhow::Error) -> String {
    // procura um erro de reqwest na cadeia de causas
    for cause in e.chain() {
        if let Some(re) = cause.downcast_ref::<reqwest::Error>() {
            if re.is_connect() || re.is_timeout() {
                return "Sem conexão com a internet (não consegui falar com o Google). \
                        Vou tentar de novo na próxima sincronização."
                    .to_string();
            }
            if re.is_status() {
                if let Some(s) = re.status() {
                    if s.as_u16() == 401 || s.as_u16() == 403 {
                        return "Autorização recusada pelo Google (token expirado ou \
                                permissão revogada). Reconecte a conta."
                            .to_string();
                    }
                }
            }
        }
    }
    e.to_string()
}

/// Obtém um access_token novo para a conta (via refresh_token).
async fn access_token_for(email: &str) -> Result<String, String> {
    let creds = config::client_creds();
    let rt = secrets::get_refresh_token(email)
        .map_err(|e| e.to_string())?
        .ok_or("conta sem refresh token — reconecte")?;
    let (at, _) = auth::refresh_access_token(&creds, &rt)
        .await
        .map_err(|e| friendly_err(&e))?;
    Ok(at)
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

/// Janela deslizante: de agora até +30 dias, em RFC3339.
fn window_30d() -> (String, String) {
    let now = chrono::Utc::now();
    let max = now + chrono::Duration::days(30);
    (now.to_rfc3339(), max.to_rfc3339())
}

#[derive(Serialize, Clone)]
pub struct AccountInfo {
    pub email: String,
    pub display_name: String,
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
pub async fn start_auth(app: AppHandle, auth_state: State<'_, AuthState>) -> Result<String, String> {
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
    let code = code.ok_or("a URL não tem 'code' — copie a URL de redirect (127.0.0.1/?code=...)")?;
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

/// Antecedência global das notificações (minutos antes do evento).
#[tauri::command]
pub fn get_lead_minutes() -> Result<i64, String> {
    store::get_setting("lead_minutes", scheduler::DEFAULT_LEAD)
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|_| "valor de antecedência inválido".to_string())
}

#[tauri::command]
pub fn set_lead_minutes(minutes: i64) -> Result<(), String> {
    let m = minutes.clamp(0, 1440);
    store::set_setting("lead_minutes", &m.to_string()).map_err(|e| e.to_string())
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
        b = b.sound("Default");
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

/// Núcleo da sincronização (janela de 30d) de todos os calendários marcados.
/// Reusado pelo comando `sync_now` e pelo poller automático.
pub(crate) async fn do_sync() -> Result<u32, String> {
    let cals = store::selected_calendars().map_err(|e| e.to_string())?;
    let mut by_acct: HashMap<String, Vec<store::Calendar>> = HashMap::new();
    for c in cals {
        by_acct.entry(c.account_email.clone()).or_default().push(c);
    }

    let (tmin, tmax) = window_30d();
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

/// Próximos eventos (para exibir na UI).
#[tauri::command]
pub fn list_events() -> Result<Vec<store::UpcomingEvent>, String> {
    store::upcoming_events(100).map_err(|e| e.to_string())
}
