//! Cliente mínimo da Google Calendar API (somente leitura).
use anyhow::Result;
use serde::Deserialize;

const CAL_BASE: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Debug, Clone)]
pub struct CalendarItem {
    pub id: String,
    pub summary: String,
    pub primary: bool,
}

/// Lista os calendários visíveis pela conta (calendarList).
pub async fn list_calendars(access_token: &str) -> Result<Vec<CalendarItem>> {
    let client = reqwest::Client::new();
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut req = client
            .get(format!("{CAL_BASE}/users/me/calendarList"))
            .bearer_auth(access_token)
            .query(&[("maxResults", "250")]);
        if let Some(t) = &page_token {
            req = req.query(&[("pageToken", t.as_str())]);
        }
        let v: CalendarListResp = req.send().await?.error_for_status()?.json().await?;
        for it in v.items {
            out.push(CalendarItem {
                id: it.id,
                summary: it.summary.unwrap_or_default(),
                primary: it.primary.unwrap_or(false),
            });
        }
        match v.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct EventItem {
    pub id: String,
    pub title: String,
    pub start_ts: i64,
    pub end_ts: i64,
    pub all_day: bool,
    pub status: String,
    pub html_link: String,
}

/// Busca os eventos de um calendário numa janela [time_min, time_max].
/// `singleEvents=true` expande recorrências em instâncias (cada uma com seu
/// horário), ideal para notificações. Ignora eventos cancelados.
pub async fn fetch_events(
    access_token: &str,
    calendar_id: &str,
    time_min_rfc3339: &str,
    time_max_rfc3339: &str,
) -> Result<Vec<EventItem>> {
    let client = reqwest::Client::new();
    let url = format!(
        "{CAL_BASE}/calendars/{}/events",
        urlencoding::encode(calendar_id)
    );
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut q: Vec<(&str, String)> = vec![
            ("timeMin", time_min_rfc3339.to_string()),
            ("timeMax", time_max_rfc3339.to_string()),
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
            ("maxResults", "2500".to_string()),
        ];
        if let Some(t) = &page_token {
            q.push(("pageToken", t.clone()));
        }
        let v: EventsResp = client
            .get(&url)
            .bearer_auth(access_token)
            .query(&q)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for e in v.items {
            if e.status.as_deref() == Some("cancelled") {
                continue;
            }
            if let Some(ev) = parse_event(e) {
                out.push(ev);
            }
        }
        match v.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }
    Ok(out)
}

fn parse_time(t: &EventTime) -> Option<(i64, bool)> {
    if let Some(dt) = &t.date_time {
        chrono::DateTime::parse_from_rfc3339(dt)
            .ok()
            .map(|d| (d.timestamp(), false))
    } else if let Some(d) = &t.date {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .and_then(|nd| nd.and_hms_opt(0, 0, 0))
            .map(|ndt| (ndt.and_utc().timestamp(), true))
    } else {
        None
    }
}

fn parse_event(e: RawEvent) -> Option<EventItem> {
    let (start_ts, all_day) = e.start.as_ref().and_then(parse_time)?;
    let (end_ts, _) = e
        .end
        .as_ref()
        .and_then(parse_time)
        .unwrap_or((start_ts, all_day));
    Some(EventItem {
        id: e.id,
        title: e.summary.unwrap_or_else(|| "(sem título)".into()),
        start_ts,
        end_ts,
        all_day,
        status: e.status.unwrap_or_default(),
        html_link: e.html_link.unwrap_or_default(),
    })
}

#[derive(Deserialize)]
struct CalendarListResp {
    #[serde(default)]
    items: Vec<CalendarListEntry>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}
#[derive(Deserialize)]
struct CalendarListEntry {
    id: String,
    summary: Option<String>,
    primary: Option<bool>,
}

#[derive(Deserialize)]
struct EventsResp {
    #[serde(default)]
    items: Vec<RawEvent>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}
#[derive(Deserialize)]
struct RawEvent {
    id: String,
    summary: Option<String>,
    status: Option<String>,
    #[serde(rename = "htmlLink")]
    html_link: Option<String>,
    start: Option<EventTime>,
    end: Option<EventTime>,
}
#[derive(Deserialize)]
struct EventTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    date: Option<String>,
}
