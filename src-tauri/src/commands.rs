use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::{auth, config, secrets, store};

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
