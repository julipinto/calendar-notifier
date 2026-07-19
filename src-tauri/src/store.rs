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
            created_at   INTEGER NOT NULL,
            needs_reauth INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS calendars (
            id            TEXT NOT NULL,
            account_email TEXT NOT NULL,
            summary       TEXT,
            selected      INTEGER NOT NULL DEFAULT 0,
            is_primary    INTEGER NOT NULL DEFAULT 0,
            color         TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (account_email, id)
        );
        CREATE TABLE IF NOT EXISTS events (
            id            TEXT NOT NULL,
            calendar_id   TEXT NOT NULL,
            account_email TEXT NOT NULL,
            title         TEXT,
            start_ts      INTEGER NOT NULL,
            end_ts        INTEGER NOT NULL,
            all_day       INTEGER NOT NULL DEFAULT 0,
            status        TEXT,
            html_link     TEXT,
            notified      INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (account_email, calendar_id, id)
        );
        CREATE INDEX IF NOT EXISTS idx_events_start ON events(start_ts);
        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    // migrações p/ bancos antigos (ignora se a coluna já existe)
    let _ = c.execute(
        "ALTER TABLE events ADD COLUMN notified INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE calendars ADD COLUMN color TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE accounts ADD COLUMN needs_reauth INTEGER NOT NULL DEFAULT 0",
        [],
    );
    Ok(())
}

// ---------- settings ----------

pub fn get_setting(key: &str, default: &str) -> Result<String> {
    let c = conn()?;
    let v: Option<String> = c
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .ok();
    Ok(v.unwrap_or_else(|| default.to_string()))
}

pub fn set_setting(key: &str, value: &str) -> Result<()> {
    let c = conn()?;
    c.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

#[derive(Serialize, Clone)]
pub struct Account {
    pub email: String,
    pub display_name: String,
    pub needs_reauth: bool,
}

pub fn upsert_account(email: &str, display_name: &str) -> Result<()> {
    let c = conn()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // conectar/reconectar sempre zera o needs_reauth (token novo em mãos)
    c.execute(
        "INSERT INTO accounts (email, display_name, created_at, needs_reauth)
         VALUES (?1, ?2, ?3, 0)
         ON CONFLICT(email) DO UPDATE SET
            display_name = excluded.display_name,
            needs_reauth = 0",
        rusqlite::params![email, display_name, now],
    )?;
    Ok(())
}

/// Marca/desmarca a conta como precisando reconectar (token inválido/revogado).
pub fn set_account_reauth(email: &str, needs: bool) -> Result<()> {
    let c = conn()?;
    c.execute(
        "UPDATE accounts SET needs_reauth = ?1 WHERE email = ?2",
        rusqlite::params![needs as i64, email],
    )?;
    Ok(())
}

pub fn list_accounts() -> Result<Vec<Account>> {
    let c = conn()?;
    let mut stmt =
        c.prepare("SELECT email, display_name, needs_reauth FROM accounts ORDER BY created_at")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Account {
                email: r.get(0)?,
                display_name: r.get(1)?,
                needs_reauth: r.get::<_, i64>(2)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn delete_account(email: &str) -> Result<()> {
    let c = conn()?;
    c.execute("DELETE FROM events WHERE account_email = ?1", [email])?;
    c.execute("DELETE FROM calendars WHERE account_email = ?1", [email])?;
    c.execute("DELETE FROM accounts WHERE email = ?1", [email])?;
    Ok(())
}

// ---------- calendários ----------

#[derive(Serialize, Clone)]
pub struct Calendar {
    pub id: String,
    pub account_email: String,
    pub summary: String,
    pub selected: bool,
    pub is_primary: bool,
    pub color: String,
}

/// Insere/atualiza um calendário preservando a escolha do usuário (`selected`).
/// Em calendários novos, `default_selected` define o estado inicial.
pub fn upsert_calendar(
    account_email: &str,
    id: &str,
    summary: &str,
    is_primary: bool,
    default_selected: bool,
    color: &str,
) -> Result<()> {
    let c = conn()?;
    c.execute(
        "INSERT INTO calendars (id, account_email, summary, selected, is_primary, color)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(account_email, id) DO UPDATE SET
            summary = excluded.summary,
            is_primary = excluded.is_primary,
            color = excluded.color",
        rusqlite::params![
            id,
            account_email,
            summary,
            default_selected as i64,
            is_primary as i64,
            color
        ],
    )?;
    Ok(())
}

pub fn set_calendar_selected(account_email: &str, id: &str, selected: bool) -> Result<()> {
    let c = conn()?;
    c.execute(
        "UPDATE calendars SET selected = ?1 WHERE account_email = ?2 AND id = ?3",
        rusqlite::params![selected as i64, account_email, id],
    )?;
    Ok(())
}

pub fn list_calendars(account_email: &str) -> Result<Vec<Calendar>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, account_email, summary, selected, is_primary, color
         FROM calendars WHERE account_email = ?1
         ORDER BY is_primary DESC, summary",
    )?;
    let rows = stmt
        .query_map([account_email], row_to_calendar)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Todos os calendários marcados para acompanhar (de todas as contas).
pub fn selected_calendars() -> Result<Vec<Calendar>> {
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT id, account_email, summary, selected, is_primary, color
         FROM calendars WHERE selected = 1",
    )?;
    let rows = stmt
        .query_map([], row_to_calendar)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_calendar(r: &rusqlite::Row) -> rusqlite::Result<Calendar> {
    Ok(Calendar {
        id: r.get(0)?,
        account_email: r.get(1)?,
        summary: r.get(2)?,
        selected: r.get::<_, i64>(3)? != 0,
        is_primary: r.get::<_, i64>(4)? != 0,
        color: r.get(5)?,
    })
}

// ---------- eventos ----------

#[derive(Serialize, Clone)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub account_email: String,
    pub title: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub all_day: bool,
    pub status: String,
    pub html_link: String,
}

/// Sincroniza os eventos de um calendário: faz upsert dos recém-buscados
/// (preservando o flag `notified`, mas resetando-o se o horário mudou) e
/// remove os que não vieram mais (evento apagado/movido pra fora da janela).
/// Tudo numa transação.
pub fn replace_events(account_email: &str, calendar_id: &str, events: &[Event]) -> Result<()> {
    let mut c = conn()?;
    let tx = c.transaction()?;
    {
        let mut up = tx.prepare(
            "INSERT INTO events
             (id, calendar_id, account_email, title, start_ts, end_ts, all_day, status, html_link, notified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)
             ON CONFLICT(account_email, calendar_id, id) DO UPDATE SET
                title = excluded.title,
                end_ts = excluded.end_ts,
                all_day = excluded.all_day,
                status = excluded.status,
                html_link = excluded.html_link,
                -- se o horário de início mudou, volta a poder notificar
                notified = CASE WHEN events.start_ts != excluded.start_ts THEN 0 ELSE events.notified END,
                start_ts = excluded.start_ts",
        )?;
        for e in events {
            up.execute(rusqlite::params![
                e.id,
                calendar_id,
                account_email,
                e.title,
                e.start_ts,
                e.end_ts,
                e.all_day as i64,
                e.status,
                e.html_link,
            ])?;
        }
        // remove eventos que não vieram nesta sincronização
        let keep: Vec<String> = events.iter().map(|e| e.id.clone()).collect();
        let placeholders = if keep.is_empty() {
            "''".to_string()
        } else {
            keep.iter().map(|_| "?").collect::<Vec<_>>().join(",")
        };
        let sql = format!(
            "DELETE FROM events WHERE account_email = ? AND calendar_id = ? AND id NOT IN ({placeholders})"
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&account_email, &calendar_id];
        for k in &keep {
            params.push(k);
        }
        tx.execute(&sql, params.as_slice())?;
    }
    tx.commit()?;
    Ok(())
}

/// Remove todos os eventos de um calendário (usado ao desmarcar).
pub fn delete_events_for_calendar(account_email: &str, calendar_id: &str) -> Result<()> {
    let c = conn()?;
    c.execute(
        "DELETE FROM events WHERE account_email = ?1 AND calendar_id = ?2",
        rusqlite::params![account_email, calendar_id],
    )?;
    Ok(())
}

/// Evento pronto para notificar.
#[derive(Clone)]
pub struct DueEvent {
    pub account_email: String,
    pub calendar_id: String,
    pub id: String,
    pub title: String,
    pub start_ts: i64,
}

/// Eventos (não dia-inteiro, ainda não notificados) cuja janela de aviso já
/// chegou: `start - lead <= agora < start`.
pub fn due_notifications(lead_minutes: i64) -> Result<Vec<DueEvent>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let lead = lead_minutes * 60;
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT account_email, calendar_id, id, title, start_ts
         FROM events
         WHERE all_day = 0 AND notified = 0
           AND (start_ts - ?1) <= ?2 AND start_ts > ?2
         ORDER BY start_ts",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![lead, now], |r| {
            Ok(DueEvent {
                account_email: r.get(0)?,
                calendar_id: r.get(1)?,
                id: r.get(2)?,
                title: r.get(3)?,
                start_ts: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn mark_notified(account_email: &str, calendar_id: &str, id: &str) -> Result<()> {
    let c = conn()?;
    c.execute(
        "UPDATE events SET notified = 1
         WHERE account_email = ?1 AND calendar_id = ?2 AND id = ?3",
        rusqlite::params![account_email, calendar_id, id],
    )?;
    Ok(())
}

/// Evento enriquecido para a UI (com cor e nome do calendário de origem).
#[derive(Serialize, Clone)]
pub struct UpcomingEvent {
    pub id: String,
    pub account_email: String,
    pub title: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub all_day: bool,
    pub html_link: String,
    pub color: String,
    pub calendar_summary: String,
}

/// Próximos eventos (a partir de agora), ordenados por início, com a cor/nome
/// do calendário via JOIN. `limit` opcional.
pub fn upcoming_events(limit: i64) -> Result<Vec<UpcomingEvent>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let c = conn()?;
    let mut stmt = c.prepare(
        "SELECT e.id, e.account_email, e.title, e.start_ts, e.end_ts, e.all_day,
                e.html_link, COALESCE(c.color, ''), COALESCE(c.summary, '')
         FROM events e
         LEFT JOIN calendars c
           ON c.account_email = e.account_email AND c.id = e.calendar_id
         WHERE e.start_ts >= ?1
         ORDER BY e.start_ts LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![now, limit], |r| {
            Ok(UpcomingEvent {
                id: r.get(0)?,
                account_email: r.get(1)?,
                title: r.get(2)?,
                start_ts: r.get(3)?,
                end_ts: r.get(4)?,
                all_day: r.get::<_, i64>(5)? != 0,
                html_link: r.get(6)?,
                color: r.get(7)?,
                calendar_summary: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
