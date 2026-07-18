//! Armazenamento dos refresh tokens.
//!
//! Fase 1 (dev/WSL): arquivo JSON em `app_dir/tokens.json` com permissão 0600
//! no Unix. É suficiente e portável para desenvolver.
//!
//! TODO (build Windows / Fase 5): trocar por keychain nativo (Credential Manager
//! no Windows, Keychain no macOS) via crate `keyring`, mantendo esta mesma API.
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::store::app_dir;

fn tokens_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("tokens.json"))
}

fn load_all() -> Result<HashMap<String, String>> {
    let p = tokens_path()?;
    if !p.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&p)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn save_all(map: &HashMap<String, String>) -> Result<()> {
    let p = tokens_path()?;
    std::fs::write(&p, serde_json::to_string_pretty(map)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn save_refresh_token(email: &str, token: &str) -> Result<()> {
    let mut all = load_all()?;
    all.insert(email.to_string(), token.to_string());
    save_all(&all)
}

pub fn get_refresh_token(email: &str) -> Result<Option<String>> {
    Ok(load_all()?.get(email).cloned())
}

pub fn delete_refresh_token(email: &str) -> Result<()> {
    let mut all = load_all()?;
    all.remove(email);
    save_all(&all)
}
