//! The Anime tab's own lists: the anime library (every season of every anime series as its own entry, and anime
//! films, with AniList's posters, airing status and next episode), and the calendar and downloads narrowed to
//! anime. Underneath, anime series stay Sonarr series of type `anime` with absolute numbers, so the Sonarr API
//! and the mapping list see the same data.
//!
//! ```text
//! GET /api/anime/library                       → [entry] by title (signed in)
//!     entry: {key, kind: "season"|"movie", series_id?, movie_id?, season?, anilist_id?, title, year, poster,
//!             format, status, next_episode?: {episode, airs}, episodes, files, monitored}
//! GET /api/anime/calendar?start=&end=&unmonitored=  → the calendar's entries for anime only (signed in)
//! GET /api/anime/activity                      → the downloads of anime only, newest first (signed in)
//! ```
//!
//! AniList's answers are kept in `anime_anilist` for six hours (less once a kept next episode has aired), so the
//! library opens fast and still shows something when AniList can't be reached.

use super::anilist::{AniList, Media};
use super::mapping::{self, Entry};
use crate::movies::{self, day, Movie};
use crate::roles::Me;
use crate::series::{self, Episode, Series};
use crate::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use serde_json::{json, Value};
use std::collections::HashMap;

/// How long a kept AniList answer is used before it is asked for again, in seconds.
const KEEP_SECONDS: i64 = 6 * 3600;

/// The table of kept AniList answers.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS anime_anilist (id INTEGER PRIMARY KEY, body TEXT NOT NULL, fetched INTEGER NOT NULL);")
        .map_err(|e| anyhow::anyhow!("The AniList table can't be made: {}", e.detail))
}

/// The routes of the Anime tab's lists.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/anime/library", get(library))
        .route("/api/anime/calendar", get(calendar))
        .route("/api/anime/activity", get(activity))
}

/// Whether a kept answer fetched at `fetched` (Unix seconds) is still good at `now`: younger than six hours, and
/// its next episode (when it had one) hasn't aired yet.
pub fn still_good(m: &Media, fetched: i64, now: i64) -> bool {
    now - fetched < KEEP_SECONDS && m.next_episode.is_none_or(|(_, at)| at > now)
}

/// AniList's entries for `ids`: kept ones while still good, the rest asked for (and kept). When AniList can't be
/// reached, the kept ones are used however old they are, and the others are left out.
pub async fn entries(st: &AppState, ids: &[i64]) -> HashMap<i64, Media> {
    let now = Utc::now().timestamp();
    let mut out = HashMap::new();
    let mut stale = HashMap::new();
    for id in ids {
        if let Ok(Some(r)) = st.db.one("SELECT body, fetched FROM anime_anilist WHERE id=?", p![id]) {
            if let Ok(m) = serde_json::from_str::<Media>(&r.s("body")) {
                if still_good(&m, r.i("fetched"), now) {
                    out.insert(*id, m);
                } else {
                    stale.insert(*id, m);
                }
            }
        }
    }
    let missing: Vec<i64> = ids.iter().copied().filter(|id| !out.contains_key(id)).collect();
    if missing.is_empty() {
        return out;
    }
    match AniList::from(st).by_ids(&missing).await {
        Ok(found) => {
            for m in found {
                let body = serde_json::to_string(&m).unwrap_or_default();
                let _ = st.db.execute(
                    "INSERT INTO anime_anilist (id, body, fetched) VALUES (?,?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body, fetched=excluded.fetched",
                    p![m.id, body, now],
                );
                out.insert(m.id, m);
            }
        }
        Err(e) => tracing::warn!("{}", e.detail),
    }
    for (id, m) in stale {
        out.entry(id).or_insert(m);
    }
    out
}

/// The anime root folders' paths.
fn anime_roots(db: &SqliteDb) -> Vec<String> {
    crate::library::folders(db, Some("anime")).unwrap_or_default().iter().map(|r| r.s("path")).collect()
}

/// Whether a movie belongs in the Anime tab: it sits in an anime root folder, or TMDB calls it Japanese
/// animation.
pub fn is_anime_movie(m: &Movie, roots: &[String]) -> bool {
    let inside = |p: &str| roots.iter().any(|r| p == r || p.starts_with(&format!("{}/", r.trim_end_matches('/'))));
    inside(&m.root_folder) || inside(&m.path) || crate::requests::is_anime(&m.info.genres, &m.info.original_language)
}

/// The anime series and the anime films of the library.
pub fn anime_titles(db: &SqliteDb) -> Result<(Vec<Series>, Vec<Movie>)> {
    let roots = anime_roots(db);
    let shows = series::all(db)?.into_iter().filter(|s| s.kind == "anime" || s.series_type == "anime").collect();
    let films = movies::all(db)?.into_iter().filter(|m| is_anime_movie(m, &roots)).collect();
    Ok((shows, films))
}

/// One season (or cour) of an anime series, as the library shows it: the mapping entry that names it (none when
/// the mapping list doesn't know the series), its season, and the first and last episode of that season it covers.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// the mapping entry (its AniList id), when there is one
    pub entry: Option<Entry>,
    /// the season
    pub season: i64,
    /// the first episode of the season it covers
    pub first: i64,
    /// the last episode it covers (`i64::MAX` for "to the end of the season")
    pub last: i64,
}

/// The parts of a series: one per mapping entry of a regular season (a season split in cours gives one part per
/// cour, each up to where the next begins), else one per season the series has.
pub fn parts(entries: &[Entry], seasons: &[i64]) -> Vec<Part> {
    let mut mapped: Vec<&Entry> = entries.iter().filter(|e| e.anilist.is_some() && e.season.is_some_and(|s| s > 0)).collect();
    mapped.sort_by_key(|e| (e.season, e.offset));
    if mapped.is_empty() {
        return seasons.iter().filter(|n| **n > 0).map(|n| Part { entry: None, season: *n, first: 1, last: i64::MAX }).collect();
    }
    let mut out: Vec<Part> = Vec::new();
    for (i, e) in mapped.iter().enumerate() {
        let season = e.season.unwrap_or(1);
        let next = mapped.get(i + 1).filter(|n| n.season == e.season).map(|n| n.offset);
        out.push(Part { entry: Some((*e).clone()), season, first: e.offset + 1, last: next.unwrap_or(i64::MAX) });
    }
    out
}

/// The library entry of one part of a series.
fn season_entry(s: &Series, part: &Part, eps: &[&Episode], al: Option<&Media>) -> Value {
    let mine: Vec<&&Episode> =
        eps.iter().filter(|e| e.season == part.season && e.episode >= part.first && e.episode <= part.last).collect();
    let files = mine.iter().filter(|e| e.has_file()).count();
    let anilist = part.entry.as_ref().and_then(|e| e.anilist);
    let tmdb_poster = movies::poster_url(&s.info.poster);
    let fallback_title = if part.season == 1 && s.seasons.iter().filter(|x| x.season > 0).count() <= 1 {
        s.info.title.clone()
    } else {
        format!("{} Season {}", s.info.title, part.season)
    };
    let status = match s.status() {
        "ended" => "Finished",
        "upcoming" => "Not yet aired",
        _ => "Airing",
    };
    json!({
        "key": format!("s{}-{}-{}", s.id, part.season, part.first),
        "kind": "season",
        "series_id": s.id,
        "season": part.season,
        "anilist_id": anilist,
        "title": al.map(|m| m.title.clone()).filter(|t| !t.is_empty()).unwrap_or(fallback_title),
        "year": al.and_then(|m| m.year).unwrap_or(s.info.year),
        "poster": al.map(|m| m.cover.clone()).filter(|c| !c.is_empty()).unwrap_or(tmdb_poster),
        "format": al.map(|m| m.format.clone()).unwrap_or_else(|| "TV".into()),
        "status": al.map(|m| m.status.clone()).filter(|x| !x.is_empty()).unwrap_or_else(|| status.into()),
        "next_episode": al.and_then(|m| m.next_episode).map(|(n, at)| json!({ "episode": n, "airs": at })),
        "episodes": al.and_then(|m| m.episodes).map(|n| n as usize).unwrap_or(mine.len()).max(mine.len()),
        "files": files,
        "monitored": s.monitored && s.season_monitored(part.season),
    })
}

/// The library entry of an anime film.
fn movie_entry(m: &Movie, al: Option<&Media>) -> Value {
    let status = match m.status(Utc::now()) {
        "released" => "Finished",
        "inCinemas" => "In cinemas",
        _ => "Not yet aired",
    };
    json!({
        "key": format!("m{}", m.id),
        "kind": "movie",
        "movie_id": m.id,
        "anilist_id": al.map(|a| a.id),
        "title": al.map(|a| a.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| m.info.title.clone()),
        "year": m.info.year,
        "poster": al.map(|a| a.cover.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| movies::poster_url(&m.info.poster)),
        "format": "MOVIE",
        "status": status,
        "next_episode": Value::Null,
        "episodes": 1,
        "files": i64::from(m.has_file()),
        "monitored": m.monitored,
    })
}

/// The whole anime library, by title.
pub async fn build(st: &AppState) -> Result<Vec<Value>> {
    let (shows, films) = anime_titles(&st.db)?;
    let all_eps = series::all_episodes(&st.db)?;
    let mut plan: Vec<(&Series, Vec<Part>)> = Vec::new();
    let mut ids = Vec::new();
    for s in &shows {
        let entries = if s.info.tvdb_id > 0 { mapping::by_tvdb(&st.db, s.info.tvdb_id)? } else { vec![] };
        let seasons: Vec<i64> = s.seasons.iter().map(|x| x.season).collect();
        let ps = parts(&entries, &seasons);
        ids.extend(ps.iter().filter_map(|p| p.entry.as_ref().and_then(|e| e.anilist)));
        plan.push((s, ps));
    }
    let film_ids: Vec<Option<i64>> =
        films.iter().map(|m| mapping::by_tmdb_movie(&st.db, m.info.tmdb_id).ok().flatten().and_then(|e| e.anilist)).collect();
    ids.extend(film_ids.iter().flatten());
    ids.sort_unstable();
    ids.dedup();
    let al = entries(st, &ids).await;
    let mut out = Vec::new();
    for (s, ps) in plan {
        let eps: Vec<&Episode> = all_eps.iter().filter(|e| e.series_id == s.id).collect();
        for p in &ps {
            let m = p.entry.as_ref().and_then(|e| e.anilist).and_then(|id| al.get(&id));
            out.push(season_entry(s, p, &eps, m));
        }
    }
    for (m, id) in films.iter().zip(film_ids) {
        out.push(movie_entry(m, id.and_then(|id| al.get(&id))));
    }
    out.sort_by_key(|v| (v["title"].as_str().unwrap_or("").to_lowercase(), v["season"].as_i64().unwrap_or(0)));
    Ok(out)
}

/// `GET /api/anime/library`.
async fn library(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    Ok(Json(json!(build(&st).await?)))
}

/// `GET /api/anime/calendar` query: the same as the calendar's.
#[derive(serde::Deserialize, Default)]
pub struct CalendarQuery {
    /// the first day, `YYYY-MM-DD` (today when left out)
    pub start: Option<String>,
    /// the last day, `YYYY-MM-DD`, included (7 days after `start` when left out)
    pub end: Option<String>,
    /// also what Marquee isn't looking for
    pub unmonitored: Option<bool>,
}

/// `GET /api/anime/calendar`: the calendar of anime series and anime films only.
async fn calendar(State(st): State<AppState>, _me: Me, Query(q): Query<CalendarQuery>) -> Result<Json<Value>> {
    let today = day(&Utc::now().format("%Y-%m-%d").to_string()).unwrap_or_else(Utc::now);
    let start = match q.start.as_deref() {
        Some(s) => day(s).ok_or_else(|| err(400, "The start day isn't a date. Write it as 2026-11-05."))?,
        None => today,
    };
    let end = match q.end.as_deref() {
        Some(s) => day(s).ok_or_else(|| err(400, "The end day isn't a date. Write it as 2026-11-12."))?,
        None => start + Duration::days(7),
    };
    if end < start {
        return Err(err(400, "The end day is before the start day."));
    }
    if end - start > Duration::days(400) {
        return Err(err(400, "Ask for at most 400 days at a time."));
    }
    let (shows, films) = anime_titles(&st.db)?;
    let eps: Vec<Episode> = series::all_episodes(&st.db)?.into_iter().filter(|e| shows.iter().any(|s| s.id == e.series_id)).collect();
    let mut list = crate::calendar::entries(&films, &shows, &eps, start, end, q.unmonitored.unwrap_or(false));
    for v in &mut list {
        if let Some(id) = v["episode_id"].as_i64() {
            v["absolute"] = json!(eps.iter().find(|e| e.id == id).and_then(|e| e.absolute));
        }
    }
    Ok(Json(json!(list)))
}

/// `GET /api/anime/activity`: the downloads of anime series and anime films.
async fn activity(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    let (shows, films) = anime_titles(&st.db)?;
    let list: Vec<Value> = crate::downloads::all(&st.db)?
        .iter()
        .filter(|d| {
            d.kind == "anime"
                || d.series_id.is_some_and(|id| shows.iter().any(|s| s.id == id))
                || d.movie_id.is_some_and(|id| films.iter().any(|m| m.id == id))
        })
        .map(crate::downloads::public)
        .collect();
    Ok(Json(json!(list)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(anilist: i64, season: i64, offset: i64) -> Entry {
        Entry { anilist: Some(anilist), tvdb: Some(1), season: Some(season), offset, kind: "TV".into(), ..Default::default() }
    }

    #[test]
    fn every_season_and_cour_is_its_own_part() {
        let got = parts(&[entry(3, 2, 0), entry(1, 1, 0), entry(2, 1, 12), entry(9, 0, 0)], &[1, 2]);
        let seen: Vec<(Option<i64>, i64, i64, i64)> =
            got.iter().map(|p| (p.entry.as_ref().and_then(|e| e.anilist), p.season, p.first, p.last)).collect();
        assert_eq!(seen, [(Some(1), 1, 1, 12), (Some(2), 1, 13, i64::MAX), (Some(3), 2, 1, i64::MAX)], "specials aren't seasons");
        let plain = parts(&[], &[0, 1, 2]);
        assert_eq!(plain.iter().map(|p| p.season).collect::<Vec<_>>(), [1, 2], "without a mapping, the series' own seasons");
    }

    #[test]
    fn kept_answers_expire_and_aired_episodes_refresh() {
        let mut m = Media::default();
        assert!(still_good(&m, 1000, 1000 + 60));
        assert!(!still_good(&m, 1000, 1000 + KEEP_SECONDS));
        m.next_episode = Some((5, 2000));
        assert!(still_good(&m, 1000, 1500));
        assert!(!still_good(&m, 1000, 2001), "the next episode has aired: ask again");
    }
}
