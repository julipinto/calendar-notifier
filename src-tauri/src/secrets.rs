//! Armazenamento dos refresh tokens.
//!
//! - Windows/macOS: keychain nativo (Credential Manager / Keychain) via `keyring`.
//! - Linux (inclui WSL, que não tem Secret Service): arquivo JSON `0600` no
//!   diretório de dados do app.
//!
//! A API pública (`save/get/delete_refresh_token`) é a mesma nos dois casos.

pub use backend::{delete_refresh_token, get_refresh_token, save_refresh_token};

// ---------- Windows / macOS: keychain nativo ----------
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod backend {
    use anyhow::Result;

    const SERVICE: &str = "calendar-notifier";

    fn entry(email: &str) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(SERVICE, email)?)
    }

    pub fn save_refresh_token(email: &str, token: &str) -> Result<()> {
        entry(email)?.set_password(token)?;
        Ok(())
    }

    pub fn get_refresh_token(email: &str) -> Result<Option<String>> {
        match entry(email)?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_refresh_token(email: &str) -> Result<()> {
        match entry(email)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

// ---------- Linux / outros: arquivo 0600 ----------
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod backend {
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
        Ok(serde_json::from_str(&std::fs::read_to_string(&p)?).unwrap_or_default())
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
}
