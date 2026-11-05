//! Port of Sonarr's and Radarr's `CalendarFeedController`: the iCal feed calendar apps subscribe to.
//!
//! ```text
//! GET /sonarr/feed/v3/calendar/Sonarr.ics?apikey=…&pastDays=7&futureDays=28&tags=&unmonitored=false&premieresOnly=false&asAllDay=false
//! GET /radarr/feed/v3/calendar/Radarr.ics?apikey=…&pastDays=7&futureDays=28&tags=&unmonitored=false&releaseTypes=…
//! ```
//!
//! Upstream builds the feed with Ical.Net; the same properties are written here by hand (RFC 5545 text escaping
//! and 75-octet line folding, CRLF line ends). Paths are matched in any case ([`crate::arr::fold_api_path`]).

use crate::{movies, series, tags, AppState};
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use kit_server_core::Result;
use std::collections::HashMap;

/// Escapes text for an iCal value (`\`, `;`, `,` and new lines).
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace('\n', "\\n")
}

/// One content line, folded at 75 octets as RFC 5545 asks (continuations start with a space).
fn line(out: &mut String, text: &str) {
    let mut width = 0;
    for c in text.chars() {
        let n = c.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
}

/// A UTC time as iCal writes it.
fn utc(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// One event of the feed.
struct Event {
    /// `UID`
    uid: String,
    /// `CONFIRMED` or `TENTATIVE`
    status: &'static str,
    /// `DESCRIPTION`
    description: String,
    /// `CATEGORIES` (the network or studio)
    category: String,
    /// `DTSTART`: a time, or a date for all-day events
    start: Start,
    /// `DTEND`, when the event has a time
    end: Option<DateTime<Utc>>,
    /// `SUMMARY`
    summary: String,
}

/// When an event starts.
enum Start {
    /// at a time
    At(DateTime<Utc>),
    /// all day on a date
    Day(NaiveDate),
}

/// The whole calendar as iCal text.
fn render(product: &str, name: &str, events: &[Event]) -> String {
    let mut out = String::new();
    let stamp = utc(Utc::now());
    line(&mut out, "BEGIN:VCALENDAR");
    line(&mut out, &format!("PRODID:{product}"));
    line(&mut out, "VERSION:2.0");
    line(&mut out, &format!("NAME:{}", escape(name)));
    line(&mut out, &format!("X-WR-CALNAME:{}", escape(name)));
    for e in events {
        line(&mut out, "BEGIN:VEVENT");
        if !e.category.is_empty() {
            line(&mut out, &format!("CATEGORIES:{}", escape(&e.category)));
        }
        if !e.description.is_empty() {
            line(&mut out, &format!("DESCRIPTION:{}", escape(&e.description)));
        }
        match &e.start {
            Start::At(t) => {
                if let Some(end) = e.end {
                    line(&mut out, &format!("DTEND:{}", utc(end)));
                }
                line(&mut out, &format!("DTSTAMP:{stamp}"));
                line(&mut out, &format!("DTSTART:{}", utc(*t)));
            }
            Start::Day(d) => {
                // Ical.Net writes an all-day event's end as the start for Radarr (`End = Start`)
                line(&mut out, &format!("DTEND;VALUE=DATE:{}", d.format("%Y%m%d")));
                line(&mut out, &format!("DTSTAMP:{stamp}"));
                line(&mut out, &format!("DTSTART;VALUE=DATE:{}", d.format("%Y%m%d")));
            }
        }
        line(&mut out, "SEQUENCE:0");
        line(&mut out, &format!("STATUS:{}", e.status));
        line(&mut out, &format!("SUMMARY:{}", escape(&e.summary)));
        line(&mut out, &format!("UID:{}", e.uid));
        line(&mut out, "END:VEVENT");
    }
    line(&mut out, "END:VCALENDAR");
    out
}

/// A query value as a number, else `default`.
fn num(q: &HashMap<String, String>, key: &str, default: i64) -> i64 {
    q.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| v.parse().ok()).unwrap_or(default)
}

/// A query value as a flag (ASP.NET reads `true`/`false` in any case).
fn flag(q: &HashMap<String, String>, key: &str) -> bool {
    q.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).is_some_and(|(_, v)| v.eq_ignore_ascii_case("true"))
}

/// The tag ids named in `tags` (comma-separated labels, as upstream's `GetTag`); unknown labels are left out.
fn tag_ids(st: &AppState, q: &HashMap<String, String>) -> Result<Vec<i64>> {
    let Some((_, raw)) = q.iter().find(|(k, _)| k.eq_ignore_ascii_case("tags")) else { return Ok(vec![]) };
    let all = tags::all(&st.db)?;
    Ok(raw
        .split(',')
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .filter_map(|l| all.iter().find(|t| t["label"] == l.as_str()).and_then(|t| t["id"].as_i64()))
        .collect())
}

/// `DateTime.Today` (local midnight) as UTC.
fn today() -> DateTime<Utc> {
    let d = Local::now().date_naive();
    Local.from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight")).single().map(|t| t.with_timezone(&Utc)).unwrap_or_else(Utc::now)
}

/// The text/calendar answer.
fn ics(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/calendar; charset=utf-8")], body).into_response()
}

/// `GET Sonarr.ics`.
async fn sonarr(State(st): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Result<Response> {
    let start = today() - Duration::days(num(&q, "pastDays", 7));
    let end = today() + Duration::days(num(&q, "futureDays", 28));
    let (unmonitored, premieres, all_day) = (flag(&q, "unmonitored"), flag(&q, "premieresOnly"), flag(&q, "asAllDay"));
    let wanted_tags = tag_ids(&st, &q)?;
    let all_series = series::all(&st.db)?;
    let mut eps: Vec<(DateTime<Utc>, series::Episode)> = series::all_episodes(&st.db)?
        .into_iter()
        .filter_map(|e| e.airs().map(|a| (a, e)))
        .filter(|(a, _)| *a >= start && *a <= end)
        .collect();
    eps.sort_by_key(|(a, _)| *a);
    let mut events = Vec::new();
    for (airs, e) in eps {
        let Some(s) = all_series.iter().find(|s| s.id == e.series_id) else { continue };
        // `EpisodesBetweenDates(…, includeUnmonitored)`: without it, only monitored episodes of monitored series
        if !unmonitored && !(e.monitored && s.monitored) {
            continue;
        }
        if premieres && (e.season == 0 || e.episode != 1) {
            continue;
        }
        if !wanted_tags.is_empty() && !wanted_tags.iter().any(|t| s.tags.contains(t)) {
            continue;
        }
        let summary = if s.series_type == "daily" {
            format!("{} - {}", s.info.title, e.title)
        } else {
            format!("{} - {}x{:02} - {}", s.info.title, e.season, e.episode, e.title)
        };
        let (start, end) = if all_day {
            (Start::Day(airs.with_timezone(&Local).date_naive()), None)
        } else {
            (Start::At(airs), Some(airs + Duration::minutes(s.info.runtime)))
        };
        events.push(Event {
            uid: format!("NzbDrone_episode_{}", e.id),
            status: if e.has_file() { "CONFIRMED" } else { "TENTATIVE" },
            description: e.overview.clone(),
            category: s.info.network.clone(),
            start,
            end,
            summary,
        });
    }
    Ok(ics(render("-//sonarr.tv//Sonarr//EN", "Sonarr TV Schedule", &events)))
}

/// `GET Radarr.ics`.
async fn radarr(State(st): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Result<Response> {
    let start = today() - Duration::days(num(&q, "pastDays", 7));
    let end = today() + Duration::days(num(&q, "futureDays", 28));
    let unmonitored = flag(&q, "unmonitored");
    let wanted_tags = tag_ids(&st, &q)?;
    // `releaseTypes` (repeated or comma-separated): cinemaRelease, digitalRelease, physicalRelease; none = all
    let types: Vec<String> = q
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("releaseTypes"))
        .flat_map(|(_, v)| v.split(',').map(|t| t.trim().to_lowercase()).collect::<Vec<_>>())
        .filter(|t| !t.is_empty())
        .collect();
    let wants = |t: &str| types.is_empty() || types.iter().any(|x| x == t);
    let now = Utc::now();
    let mut list: Vec<movies::Movie> =
        movies::all(&st.db)?.into_iter().filter(|m| (unmonitored || m.monitored) && m.in_window(start, end)).collect();
    list.sort_by(|a, b| a.added.cmp(&b.added));
    let mut events = Vec::new();
    for m in &list {
        if !wanted_tags.is_empty() && !wanted_tags.iter().any(|t| m.tags.contains(t)) {
            continue;
        }
        for (kind, date, suffix, text) in [
            ("cinemarelease", &m.info.in_cinemas, "_cinemas", "(Theatrical Release)"),
            ("digitalrelease", &m.info.digital_release, "_digital", "(Digital Release)"),
            ("physicalrelease", &m.info.physical_release, "_physical", "(Physical Release)"),
        ] {
            let Some(day) = date.as_deref().and_then(|d| NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok()) else { continue };
            if !wants(kind) {
                continue;
            }
            events.push(Event {
                uid: format!("Radarr_movie_{}{suffix}", m.id),
                status: if m.status(now) == "announced" { "TENTATIVE" } else { "CONFIRMED" },
                description: m.info.overview.clone(),
                category: m.info.studio.clone(),
                start: Start::Day(day),
                end: None,
                summary: format!("{} {text}", m.info.title),
            });
        }
    }
    Ok(ics(render("-//radarr.video//Radarr//EN", "Radarr Movies Calendar", &events)))
}

/// Sonarr's feed route (lower case, see the module notes), merged into the Sonarr router behind the API key.
pub fn sonarr_routes() -> Router<AppState> {
    Router::new().route("/sonarr/feed/v3/calendar/sonarr.ics", get(sonarr))
}

/// Radarr's feed route, merged into the Radarr router behind the API key.
pub fn radarr_routes() -> Router<AppState> {
    Router::new().route("/radarr/feed/v3/calendar/radarr.ics", get(radarr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_lines_folded() {
        assert_eq!(escape("a,b;c\\d\ne"), "a\\,b\\;c\\\\d\\ne");
        let mut out = String::new();
        line(&mut out, &"x".repeat(100));
        let parts: Vec<&str> = out.split("\r\n").collect();
        assert_eq!(parts[0].len(), 75);
        assert!(parts[1].starts_with(' ') && parts[1].len() == 26);
    }
}
