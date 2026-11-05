//! The calendar: what comes out between two days, movies and episodes in one list, for the phone app and anyone
//! signed in. The Radarr and Sonarr APIs keep their own calendars (`/radarr/api/v3/calendar`,
//! `/sonarr/api/v3/calendar`) in their shapes; this one is Marquee's own.
//!
//! ```text
//! GET /api/calendar?start=YYYY-MM-DD&end=YYYY-MM-DD&unmonitored=false
//!     → [entry] by date, then title (signed in). Without start/end: today and the next 7 days.
//!       entry: {kind: "movie", date, release: "cinemas"|"physical"|"digital", movie_id, title, year, poster,
//!               has_file, monitored}
//!            | {kind: "episode", date, series_id, episode_id, series_title, series_kind, season, episode, title,
//!               poster, has_file, monitored}
//! ```

use crate::movies::{self, day, Movie};
use crate::roles::Me;
use crate::series::{self, Episode, Series};
use crate::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use kit_server_core::{err, Result};
use serde::Deserialize;
use serde_json::{json, Value};

/// The days the calendar shows when none are asked for: today and this many after it.
const DEFAULT_DAYS: i64 = 7;

/// The longest window one call may ask for, in days, so a typo can't ask for centuries.
const MAX_DAYS: i64 = 400;

/// What `GET /api/calendar` takes.
#[derive(Deserialize, Default)]
pub struct CalendarQuery {
    /// the first day, `YYYY-MM-DD` (today when left out)
    pub start: Option<String>,
    /// the last day, `YYYY-MM-DD`, included (7 days after `start` when left out)
    pub end: Option<String>,
    /// also what Marquee isn't looking for (false: only monitored movies and episodes of monitored series)
    pub unmonitored: Option<bool>,
}

/// The calendar's route.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/calendar", get(calendar))
}

/// `GET /api/calendar`.
async fn calendar(State(st): State<AppState>, _me: Me, Query(q): Query<CalendarQuery>) -> Result<Json<Value>> {
    let today = day(&Utc::now().format("%Y-%m-%d").to_string()).unwrap_or_else(Utc::now);
    let start = match q.start.as_deref() {
        Some(s) => day(s).ok_or_else(|| err(400, "The start day isn't a date. Write it as 2026-11-05."))?,
        None => today,
    };
    let end = match q.end.as_deref() {
        Some(s) => day(s).ok_or_else(|| err(400, "The end day isn't a date. Write it as 2026-11-12."))?,
        None => start + Duration::days(DEFAULT_DAYS),
    };
    if end < start {
        return Err(err(400, "The end day is before the start day."));
    }
    if end - start > Duration::days(MAX_DAYS) {
        return Err(err(400, format!("Ask for at most {MAX_DAYS} days at a time.")));
    }
    let all = q.unmonitored.unwrap_or(false);
    Ok(Json(json!(entries(&movies::all(&st.db)?, &series::all(&st.db)?, &series::all_episodes(&st.db)?, start, end, all))))
}

/// The calendar between `start` and `end` (both days included): a movie once for each of its dates in the window,
/// an episode on its air day; only monitored ones unless `all`. Sorted by date, then title.
pub fn entries(
    movies: &[Movie],
    series: &[Series],
    episodes: &[Episode],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    all: bool,
) -> Vec<Value> {
    let inside = |d: &Option<String>| -> Option<String> {
        let d = d.as_deref()?;
        let t = day(d)?;
        (t >= start && t <= end).then(|| d[..10].to_string())
    };
    let mut out: Vec<(String, String, Value)> = Vec::new();
    for m in movies.iter().filter(|m| all || m.monitored) {
        let dates = [("cinemas", &m.info.in_cinemas), ("physical", &m.info.physical_release), ("digital", &m.info.digital_release)];
        for (release, date) in dates {
            if let Some(d) = inside(date) {
                out.push((
                    d.clone(),
                    m.info.title.clone(),
                    json!({
                        "kind": "movie", "date": d, "release": release, "movie_id": m.id, "title": m.info.title,
                        "year": m.info.year, "poster": movies::poster_url(&m.info.poster), "has_file": m.has_file(),
                        "monitored": m.monitored,
                    }),
                ));
            }
        }
    }
    for e in episodes {
        let Some(s) = series.iter().find(|s| s.id == e.series_id) else { continue };
        if !all && !(s.monitored && e.monitored) {
            continue;
        }
        let Some(d) = inside(&e.air_date) else { continue };
        out.push((
            d.clone(),
            s.info.title.clone(),
            json!({
                "kind": "episode", "date": d, "series_id": s.id, "episode_id": e.id, "series_title": s.info.title,
                "series_kind": s.kind, "season": e.season, "episode": e.episode, "title": e.title,
                "poster": movies::poster_url(&s.info.poster), "has_file": e.has_file(), "monitored": s.monitored && e.monitored,
            }),
        ));
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out.into_iter().map(|(_, _, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(id: i64, title: &str, cinemas: Option<&str>, digital: Option<&str>, monitored: bool) -> Movie {
        let info = crate::tmdb::MovieInfo {
            title: title.into(),
            in_cinemas: cinemas.map(String::from),
            digital_release: digital.map(String::from),
            ..Default::default()
        };
        Movie {
            id,
            info,
            path: String::new(),
            root_folder: String::new(),
            quality_profile_id: 1,
            monitored,
            minimum_availability: "released".into(),
            tags: vec![],
            movie_file_id: 0,
            added: String::new(),
            last_info_sync: None,
        }
    }

    fn episode(id: i64, series_id: i64, n: i64, date: &str, monitored: bool, file: i64) -> Episode {
        Episode {
            id,
            series_id,
            tmdb_id: id,
            season: 1,
            episode: n,
            absolute: Some(n),
            title: format!("Episode {n}"),
            overview: String::new(),
            air_date: Some(date.into()),
            runtime: 30,
            monitored,
            episode_file_id: file,
        }
    }

    fn show(id: i64, title: &str, monitored: bool) -> Series {
        let mut s = Series::from_row(&Default::default());
        s.id = id;
        s.info.title = title.into();
        s.kind = "tv".into();
        s.monitored = monitored;
        s
    }

    #[test]
    fn movies_and_episodes_in_the_window_by_day() {
        let movies = [
            movie(1, "Zebra", Some("2026-11-06"), Some("2026-11-20"), true),
            movie(2, "Apple", None, Some("2026-11-06"), true),
            movie(3, "Hidden", Some("2026-11-06"), None, false),
        ];
        let series = [show(10, "The Bear", true), show(11, "Paused", false)];
        let eps = [
            episode(100, 10, 1, "2026-11-05", true, 7),
            episode(101, 10, 2, "2026-11-12", false, 0),
            episode(102, 11, 1, "2026-11-05", true, 0),
            episode(103, 10, 3, "2027-01-01", true, 0),
        ];
        let start = day("2026-11-05").unwrap();
        let end = day("2026-11-12").unwrap();
        let got = entries(&movies, &series, &eps, start, end, false);
        let seen: Vec<(String, String)> =
            got.iter().map(|e| (e["date"].as_str().unwrap().into(), e["kind"].as_str().unwrap().into())).collect();
        assert_eq!(
            seen,
            [("2026-11-05".into(), "episode".into()), ("2026-11-06".into(), "movie".into()), ("2026-11-06".into(), "movie".into())]
        );
        assert_eq!(got[0]["has_file"], true);
        assert_eq!(got[0]["series_title"], "The Bear");
        assert_eq!(got[1]["title"], "Apple", "the same day goes by title");
        assert_eq!(got[2]["release"], "cinemas");

        let all = entries(&movies, &series, &eps, start, end, true);
        assert_eq!(all.len(), 6, "unmonitored ones too: Hidden, Paused's episode and the unmonitored episode");
    }
}
