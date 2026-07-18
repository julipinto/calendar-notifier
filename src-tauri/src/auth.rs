//! Fluxo OAuth 2.0 "Desktop app" com PKCE e loopback local — sem servidor.
//!
//! 1. `begin` sobe um TcpListener em 127.0.0.1:<porta aleatória> e monta a URL
//!    de consentimento (com code_challenge PKCE + state).
//! 2. O chamador abre a URL no navegador (e/ou mostra o link na UI).
//! 3. `finish` espera o redirect no loopback, valida o state, troca o code por
//!    tokens e busca email/nome via userinfo.
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::config::ClientCreds;

const SCOPES: &str = "openid email https://www.googleapis.com/auth/calendar.readonly";
const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
/// Quanto tempo esperamos o usuário concluir a autorização no navegador.
const AUTH_TIMEOUT: Duration = Duration::from_secs(300);

/// Resultado de uma conexão bem-sucedida.
pub struct Connected {
    pub email: String,
    pub display_name: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_in: i64,
}

/// Estado intermediário entre `begin` e `finish`.
pub struct PendingAuth {
    pub auth_url: String,
    pub listener: TcpListener,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_b64(n_bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; n_bytes];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow!("falha ao gerar aleatoriedade: {e}"))?;
    Ok(b64url(&buf))
}

fn pkce() -> Result<(String, String)> {
    let verifier = random_b64(48)?; // 64 chars base64url
    let challenge = b64url(Sha256::digest(verifier.as_bytes()).as_slice());
    Ok((verifier, challenge))
}

/// Prepara o fluxo: abre o loopback e monta a URL de consentimento.
pub async fn begin(creds: &ClientCreds) -> Result<PendingAuth> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("não consegui abrir o listener loopback")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    eprintln!("[auth] loopback ouvindo em {redirect_uri}");

    let (verifier, challenge) = pkce()?;
    let state = random_b64(24)?;

    let mut u = url::Url::parse(&creds.auth_uri)?;
    u.query_pairs_mut()
        .append_pair("client_id", &creds.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");

    Ok(PendingAuth {
        auth_url: u.to_string(),
        listener,
        verifier,
        state,
        redirect_uri,
    })
}

/// Caminho automático (Windows): espera o redirect no loopback, valida e troca.
pub async fn finish(creds: &ClientCreds, pending: PendingAuth) -> Result<Connected> {
    let (code, got_state) = tokio::time::timeout(AUTH_TIMEOUT, wait_for_code(&pending.listener))
        .await
        .map_err(|_| anyhow!("tempo esgotado esperando a autorização"))??;

    if got_state != pending.state {
        bail!("state inválido (possível CSRF) — tente conectar novamente");
    }
    exchange(creds, &code, &pending.verifier, &pending.redirect_uri).await
}

/// Troca o authorization code por tokens e busca email/nome. Usado tanto pelo
/// caminho automático (loopback) quanto pela conclusão manual (colar URL).
pub async fn exchange(
    creds: &ClientCreds,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Connected> {
    let client = reqwest::Client::new();
    let token_res: TokenResponse = client
        .post(&creds.token_uri)
        .form(&[
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await?
        .error_for_status()
        .context("troca do code por token falhou")?
        .json()
        .await?;

    let refresh_token = token_res.refresh_token.ok_or_else(|| {
        anyhow!(
            "Google não retornou refresh_token. Revogue o acesso do app em \
             myaccount.google.com/permissions e conecte novamente."
        )
    })?;

    let userinfo: UserInfo = client
        .get(USERINFO_URL)
        .bearer_auth(&token_res.access_token)
        .send()
        .await?
        .error_for_status()
        .context("userinfo falhou")?
        .json()
        .await?;

    let email = userinfo.email;
    Ok(Connected {
        display_name: userinfo.name.unwrap_or_else(|| email.clone()),
        email,
        refresh_token,
        access_token: token_res.access_token,
        expires_in: token_res.expires_in,
    })
}

/// Troca um refresh_token por um novo access_token. Retorna (access_token, expires_in).
pub async fn refresh_access_token(creds: &ClientCreds, refresh_token: &str) -> Result<(String, i64)> {
    let client = reqwest::Client::new();
    let res: TokenResponse = client
        .post(&creds.token_uri)
        .form(&[
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?
        .error_for_status()
        .context("refresh do access_token falhou")?
        .json()
        .await?;
    Ok((res.access_token, res.expires_in))
}

/// Aceita conexões no loopback até chegar uma com `code`+`state` (ou `error`).
/// Responde ao navegador com uma página simples e ignora requests laterais
/// (ex.: /favicon.ico).
async fn wait_for_code(listener: &TcpListener) -> Result<(String, String)> {
    loop {
        let (mut socket, peer) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        eprintln!("[auth] conexão de {peer} → path: {path}");

        let parsed = url::Url::parse(&format!("http://127.0.0.1{path}"));
        let (mut code, mut state, mut err) = (None, None, None);
        if let Ok(ref p) = parsed {
            for (k, v) in p.query_pairs() {
                match k.as_ref() {
                    "code" => code = Some(v.to_string()),
                    "state" => state = Some(v.to_string()),
                    "error" => err = Some(v.to_string()),
                    _ => {}
                }
            }
        }

        let done = err.is_some() || (code.is_some() && state.is_some());
        let body = if done {
            "<html><body style='font-family:sans-serif;text-align:center;padding-top:15vh'>\
             <h2>Pode fechar esta aba ✓</h2><p>Volte ao Calendar Notifier.</p></body></html>"
        } else {
            "<html><body></body></html>"
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(resp.as_bytes()).await;
        let _ = socket.flush().await;

        if let Some(e) = err {
            bail!("autorização negada: {e}");
        }
        if let (Some(c), Some(s)) = (code, state) {
            return Ok((c, s));
        }
        // request lateral (favicon etc.): continua esperando o redirect real.
    }
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(serde::Deserialize)]
struct UserInfo {
    email: String,
    #[serde(default)]
    name: Option<String>,
}
