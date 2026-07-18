use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::path::PathBuf;

/// Diretório de dados do app (config do SO + subpasta do app). Criado se não existir.
pub fn app_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("sem diretório de config do SO")?;
    let dir = base.join("calendar-notifier");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn db_path() -> Result<PathBuf> {
    Ok(app_dir()?.join("app.db"))
}

fn conn() -> Result<Connection> {
    Ok(Connection::open(db_path()?)?)
}

/// Cria as tabelas se necessário. Chamado no setup do Tauri.
pub fn init() -> Result<()> {
    let c = conn()?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS accounts (
            email        TEXT PRIMARY KEY,
            display_name TEXT,
            created_at   INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

#[derive(Serialize, Clone)]
pub struct Account {
    pub email: String,
    pub display_name: String,
}

pub fn upsert_account(email: &str, display_name: &str) -> Result<()> {
    let c = conn()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    c.execute(
        "INSERT INTO accounts (email, display_name, created_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(email) DO UPDATE SET display_name = excluded.display_name",
        rusqlite::params![email, display_name, now],
    )?;
    Ok(())
}

pub fn list_accounts() -> Result<Vec<Account>> {
    let c = conn()?;
    let mut stmt = c.prepare("SELECT email, display_name FROM accounts ORDER BY created_at")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Account {
                email: r.get(0)?,
                display_name: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn delete_account(email: &str) -> Result<()> {
    let c = conn()?;
    c.execute(
        "DELETE FROM accounts WHERE email = ?1",
        rusqlite::params![email],
    )?;
    Ok(())
}
