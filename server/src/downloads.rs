//! Downloads: searching Lantern for what is wanted, grabbing the best release, following it until it is done, and
//! importing the files into the library. Every grab is a row in `downloads`; files in the library are rows in
//! `movie_files` and `episode_files`.
//!
//! ```text
//! GET    /api/downloads                    → [download] newest first (signed in)
//! DELETE /api/downloads/{id}               → {ok}; stops it in Lantern (admins)
//! POST   /api/movies/{id}/search           → {grabbed: download|null, looked_at, reason} (admins)
//! GET    /api/movies/{id}/releases         → [candidate] every release Lantern found, with why each is turned down
//! POST   /api/series/{id}/search           {season?, episode_ids?} → {grabbed: [download], looked_at, reason}
//! POST   /api/lantern/events               {event, grab} (Lantern calls this with Marquee's API key)
//! POST   /api/lantern/sync                 → {ok}; asks Lantern about every download now (admins)
//! ```

use crate::decide::{self, Candidate, Profile, Sizes, Wanted, WantedEpisode};
use crate::import::{self, Placed};
use crate::lantern::{Grab, Lantern, Query as LQuery};
use crate::movies::{self, now_iso, Movie};
use crate::roles::Me;
use crate::series::{self, Episode, Series};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::Utc;
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

/// The downloads and library files tables.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS downloads (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            lantern_id TEXT UNIQUE NOT NULL,
            kind TEXT NOT NULL,
            movie_id INTEGER,
            series_id INTEGER,
            episode_ids TEXT NOT NULL DEFAULT '[]',
            title TEXT NOT NULL,
            quality TEXT NOT NULL DEFAULT '',
            protocol TEXT NOT NULL DEFAULT '',
            indexer TEXT NOT NULL DEFAULT '',
            size INTEGER NOT NULL DEFAULT 0,
            left_bytes INTEGER NOT NULL DEFAULT 0,
            eta INTEGER,
            status TEXT NOT NULL DEFAULT 'queued',
            progress REAL NOT NULL DEFAULT 0,
            message TEXT NOT NULL DEFAULT '',
            files TEXT NOT NULL DEFAULT '[]',
            added TEXT NOT NULL,
            updated TEXT NOT NULL,
            imported TEXT
        );
        CREATE TABLE IF NOT EXISTS movie_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            movie_id INTEGER NOT NULL,
            path TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            quality TEXT NOT NULL,
            release_group TEXT NOT NULL DEFAULT '',
            scene_name TEXT NOT NULL DEFAULT '',
            hardlinked INTEGER NOT NULL DEFAULT 1,
            added TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS episode_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            series_id INTEGER NOT NULL,
            season INTEGER NOT NULL,
            path TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            quality TEXT NOT NULL,
            release_group TEXT NOT NULL DEFAULT '',
            scene_name TEXT NOT NULL DEFAULT '',
            hardlinked INTEGER NOT NULL DEFAULT 1,
            added TEXT NOT NULL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The download tables can't be made: {}", e.detail))
}

/// A download as Marquee keeps it.
#[derive(Clone, Debug)]
pub struct Download {
    /// Marquee's id
    pub id: i64,
    /// Lantern's id
    pub lantern_id: String,
    /// `movie`, `tv` or `anime`
    pub kind: String,
    /// the movie it is for
    pub movie_id: Option<i64>,
    /// the series it is for
    pub series_id: Option<i64>,
    /// the episodes it is for
    pub episode_ids: Vec<i64>,
    /// the release's name
    pub title: String,
    /// its quality, as the kind's quality list spells it
    pub quality: String,
    /// `torrent` or `usenet`
    pub protocol: String,
    /// the indexer's name
    pub indexer: String,
    /// bytes
    pub size: i64,
    /// bytes still to come
    pub left: i64,
    /// seconds still to go, when known
    pub eta: Option<i64>,
    /// `queued`, `downloading`, `paused`, `completed`, `failed`, `importing`, `imported`, `importBlocked` or `removed`
    pub status: String,
    /// from 0 to 1
    pub progress: f64,
    /// what happened last, in words (why it failed or can't be imported)
    pub message: String,
    /// the finished files
    pub files: Vec<String>,
    /// when it was grabbed
    pub added: String,
    /// when it was imported
    pub imported: Option<String>,
}

impl Download {
    /// The download of a table row.
    fn from_row(r: &Row) -> Download {
        let opt = |k: &str| r.get(k).and_then(Value::as_i64);
        Download {
            id: r.i("id"),
            lantern_id: r.s("lantern_id"),
            kind: r.s("kind"),
            movie_id: opt("movie_id"),
            series_id: opt("series_id"),
            episode_ids: serde_json::from_str(&r.s("episode_ids")).unwrap_or_default(),
            title: r.s("title"),
            quality: r.s("quality"),
            protocol: r.s("protocol"),
            indexer: r.s("indexer"),
            size: r.i("size"),
            left: r.i("left_bytes"),
            eta: opt("eta"),
            status: r.s("status"),
            progress: r.f("progress"),
            message: r.s("message"),
            files: serde_json::from_str(&r.s("files")).unwrap_or_default(),
            added: r.s("added"),
            imported: Some(r.s("imported")).filter(|s| !s.is_empty()),
        }
    }

    /// Whether it is still on its way (shown in the queue).
    pub fn active(&self) -> bool {
        !matches!(self.status.as_str(), "imported" | "removed")
    }
}

/// Every download, newest first.
pub fn all(db: &SqliteDb) -> Result<Vec<Download>> {
    Ok(db.query("SELECT * FROM downloads ORDER BY id DESC", p![])?.iter().map(Download::from_row).collect())
}

/// The downloads still on their way.
pub fn active(db: &SqliteDb) -> Result<Vec<Download>> {
    Ok(all(db)?.into_iter().filter(Download::active).collect())
}

/// One download by Marquee's id.
pub fn find(db: &SqliteDb, id: i64) -> Result<Option<Download>> {
    Ok(db.one("SELECT * FROM downloads WHERE id=?", p![id])?.map(|r| Download::from_row(&r)))
}

/// One download by Lantern's id.
fn by_lantern(db: &SqliteDb, id: &str) -> Result<Option<Download>> {
    Ok(db.one("SELECT * FROM downloads WHERE lantern_id=?", p![id])?.map(|r| Download::from_row(&r)))
}

/// A download for the web app.
pub fn public(d: &Download) -> Value {
    json!({
        "id": d.id, "kind": d.kind, "movie_id": d.movie_id, "series_id": d.series_id, "episode_ids": d.episode_ids,
        "title": d.title, "quality": d.quality, "protocol": d.protocol, "indexer": d.indexer, "size": d.size,
        "left": d.left, "eta": d.eta, "status": d.status, "progress": d.progress, "message": d.message,
        "added": d.added, "imported": d.imported,
    })
}

/// The quality object (`{id, name, source, resolution, modifier}`) of `kind`'s quality list for `name`.
pub fn quality_object(db: &SqliteDb, kind: &str, name: &str) -> Value {
    let k = if kind == "movie" { "movie" } else { "tv" };
    crate::quality::definitions(db, k)
        .unwrap_or_default()
        .into_iter()
        .find(|d| d["quality"]["name"] == name)
        .map(|d| d["quality"].clone())
        .unwrap_or_else(|| json!({ "id": 0, "name": "Unknown", "source": "unknown", "resolution": 0, "modifier": "none" }))
}

/// `h:mm:ss` for a number of seconds (the queue's `timeleft`).
fn time_left(secs: i64) -> String {
    let d = secs / 86400;
    let (h, m, s) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    if d > 0 {
        format!("{d}.{h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

/// One queue record in Radarr's and Sonarr's shape (without the movie or series ids).
fn queue_base(db: &SqliteDb, d: &Download) -> serde_json::Map<String, Value> {
    let (status, tracked, state) = match d.status.as_str() {
        "queued" => ("queued", "ok", "downloading"),
        "paused" => ("paused", "ok", "downloading"),
        "completed" => ("completed", "ok", "importPending"),
        "importing" => ("completed", "ok", "importing"),
        "importBlocked" => ("completed", "warning", "importBlocked"),
        "failed" => ("failed", "error", "failedPending"),
        _ => ("downloading", "ok", "downloading"),
    };
    let mut o = serde_json::Map::new();
    o.insert("languages".into(), json!([{ "id": 1, "name": "English" }]));
    o.insert(
        "quality".into(),
        json!({ "quality": quality_object(db, &d.kind, &d.quality), "revision": { "version": 1, "real": 0, "isRepack": false } }),
    );
    o.insert("customFormats".into(), json!([]));
    o.insert("customFormatScore".into(), json!(0));
    o.insert("size".into(), json!(d.size));
    o.insert("title".into(), json!(d.title));
    o.insert("sizeleft".into(), json!(d.left));
    if let Some(eta) = d.eta {
        o.insert("timeleft".into(), json!(time_left(eta)));
        o.insert(
            "estimatedCompletionTime".into(),
            json!((Utc::now() + chrono::Duration::seconds(eta)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        );
    }
    o.insert("added".into(), json!(d.added));
    o.insert("status".into(), json!(status));
    o.insert("trackedDownloadStatus".into(), json!(tracked));
    o.insert("trackedDownloadState".into(), json!(state));
    let msgs = if d.message.is_empty() { json!([]) } else { json!([{ "title": d.title, "messages": [d.message] }]) };
    o.insert("statusMessages".into(), msgs);
    if d.status == "failed" {
        o.insert("errorMessage".into(), json!(d.message));
    }
    o.insert("downloadId".into(), json!(d.lantern_id));
    o.insert("protocol".into(), json!(d.protocol));
    o.insert("downloadClient".into(), json!("Lantern"));
    o.insert("downloadClientHasPostImportCategory".into(), json!(false));
    o.insert("indexer".into(), json!(d.indexer));
    if let Some(f) = d.files.first() {
        o.insert("outputPath".into(), json!(f));
    }
    o
}

/// Radarr's queue: the movie downloads on their way.
pub fn radarr_queue(st: &AppState) -> Vec<Value> {
    active(&st.db)
        .unwrap_or_default()
        .iter()
        .filter(|d| d.movie_id.is_some())
        .map(|d| {
            let mut o = serde_json::Map::new();
            o.insert("movieId".into(), json!(d.movie_id));
            o.extend(queue_base(&st.db, d));
            o.insert("id".into(), json!(d.id));
            Value::Object(o)
        })
        .collect()
}

/// Sonarr's queue: one record per episode of the TV downloads on their way.
pub fn sonarr_queue(st: &AppState) -> Vec<Value> {
    let mut out = Vec::new();
    for d in active(&st.db).unwrap_or_default().iter().filter(|d| d.series_id.is_some()) {
        for eid in &d.episode_ids {
            let e = series::episode(&st.db, *eid).ok().flatten();
            let mut o = serde_json::Map::new();
            o.insert("seriesId".into(), json!(d.series_id));
            o.insert("episodeId".into(), json!(eid));
            o.insert("seasonNumber".into(), json!(e.as_ref().map(|e| e.season)));
            o.extend(queue_base(&st.db, d));
            o.insert("episodeHasFile".into(), json!(e.as_ref().is_some_and(Episode::has_file)));
            // Sonarr gives each episode of a download its own id
            o.insert("id".into(), json!(d.id * 10_000 + eid % 10_000));
            out.push(Value::Object(o));
        }
    }
    out
}

/// Starts a search in the background, when Lantern is set up (Radarr and Sonarr commands, adds).
pub fn spawn_search(st: &AppState, what: Search) {
    if !Lantern::configured(st) {
        tracing::info!("No search: Lantern isn't set up in Settings → Connections");
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        let r = match &what {
            Search::Movies(ids) => {
                for id in ids {
                    if let Err(e) = search_movie(&st, *id).await {
                        tracing::warn!("Searching for movie {id} didn't work: {}", e.detail);
                    }
                }
                Ok(())
            }
            Search::Series(id, season) => search_series(&st, *id, *season, &[]).await.map(|_| ()),
            Search::Episodes(ids) => {
                let mut by_series: Vec<(i64, Vec<i64>)> = Vec::new();
                for id in ids {
                    if let Ok(Some(e)) = series::episode(&st.db, *id) {
                        match by_series.iter_mut().find(|(s, _)| *s == e.series_id) {
                            Some(x) => x.1.push(*id),
                            None => by_series.push((e.series_id, vec![*id])),
                        }
                    }
                }
                for (s, eps) in by_series {
                    if let Err(e) = search_series(&st, s, None, &eps).await {
                        tracing::warn!("Searching for episodes of series {s} didn't work: {}", e.detail);
                    }
                }
                Ok(())
            }
            Search::Missing { movies, tv } => {
                search_missing(&st, *movies, *tv).await;
                Ok(())
            }
        };
        if let Err(e) = r {
            tracing::warn!("The search didn't work: {}", e.detail);
        }
    });
}

/// What a background search looks for.
#[derive(Clone, Debug)]
pub enum Search {
    /// these movies
    Movies(Vec<i64>),
    /// a series (one season, or all wanted episodes)
    Series(i64, Option<i64>),
    /// these episodes
    Episodes(Vec<i64>),
    /// everything missing, of movies and/or TV
    Missing {
        /// movies too
        movies: bool,
        /// TV too
        tv: bool,
    },
}

/// The profile and size rules for an item of `kind` with profile `profile_id`.
fn rules(db: &SqliteDb, kind: &str, profile_id: i64) -> Result<(Profile, Sizes)> {
    let body = crate::quality::profile(db, profile_id)?.ok_or_else(|| err(400, "Its quality profile is gone. Pick another one."))?;
    let formats = crate::trash::formats(db, kind);
    Ok((Profile::read(&body).with_formats(&formats), Sizes::read(&crate::quality::definitions(db, kind)?)))
}

/// The movie file of a movie: (path, quality name).
fn movie_file(db: &SqliteDb, id: i64) -> Result<Option<(String, String)>> {
    Ok(db.one("SELECT path, quality FROM movie_files WHERE id=?", p![id])?.map(|r| (r.s("path"), r.s("quality"))))
}

/// The episode file: (path, quality name).
fn episode_file(db: &SqliteDb, id: i64) -> Result<Option<(String, String)>> {
    Ok(db.one("SELECT path, quality FROM episode_files WHERE id=?", p![id])?.map(|r| (r.s("path"), r.s("quality"))))
}

/// A movie file for the Radarr API and the web app: `{id, path, size, quality, releaseGroup, sceneName, added}`.
pub fn movie_file_row(db: &SqliteDb, id: i64) -> Option<Row> {
    db.one("SELECT * FROM movie_files WHERE id=?", p![id]).ok().flatten()
}

/// Every episode file of a series.
pub fn episode_files(db: &SqliteDb, series_id: i64) -> Result<Vec<Row>> {
    db.query("SELECT * FROM episode_files WHERE series_id=? ORDER BY season, path", p![series_id])
}

/// Whether a download for one of these is on its way already.
fn already_downloading(db: &SqliteDb, movie: Option<i64>, episodes: &[i64]) -> Result<bool> {
    Ok(active(db)?
        .iter()
        .any(|d| d.status != "failed" && (movie.is_some() && d.movie_id == movie || d.episode_ids.iter().any(|e| episodes.contains(e)))))
}

/// Keeps a new grab.
fn record(db: &SqliteDb, kind: &str, movie: Option<i64>, series: Option<i64>, c: &Candidate, g: &Grab) -> Result<Download> {
    let now = now_iso();
    let status = if g.status.is_empty() { "queued".to_string() } else { g.status.clone() };
    let id = db.execute(
        "INSERT INTO downloads (lantern_id, kind, movie_id, series_id, episode_ids, title, quality, protocol, indexer, size,
            left_bytes, status, added, updated) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        p![
            g.id,
            kind,
            movie,
            series,
            json!(c.episode_ids).to_string(),
            c.release.title,
            c.quality,
            c.release.protocol,
            c.release.indexer,
            c.release.size,
            c.release.size,
            status,
            now,
            now
        ],
    )?;
    find(db, id)?.ok_or_else(|| err(500, "The download wasn't kept"))
}

/// What a search did: the downloads it started, how many releases it looked at, and in words why nothing was
/// grabbed when nothing was.
#[derive(Debug, Default)]
pub struct Outcome {
    /// the downloads started
    pub grabbed: Vec<Download>,
    /// releases looked at
    pub looked_at: usize,
    /// why nothing was grabbed
    pub reason: Option<String>,
}

/// Every release Lantern finds for a movie, checked and in order (the best first).
pub async fn movie_candidates(st: &AppState, m: &Movie) -> Result<Vec<Candidate>> {
    let lantern = Lantern::from(st)?;
    let (profile, sizes) = rules(&st.db, "movie", m.quality_profile_id)?;
    let current = if m.has_file() { movie_file(&st.db, m.movie_file_id)?.and_then(|(_, q)| profile.rank(&q).map(|r| r.0)) } else { None };
    let q = LQuery {
        kind: "movie",
        terms: format!("{} {}", m.info.title, if m.info.year > 0 { m.info.year.to_string() } else { String::new() }).trim().to_string(),
        imdb: m.info.imdb_id.clone(),
        tmdb: m.info.tmdb_id,
        categories: vec![2000],
        ..LQuery::default()
    };
    let found = lantern.search(&q).await?;
    let w = Wanted { kind: "movie", profile: &profile, sizes: &sizes, current };
    Ok(decide::for_movie(&w, found, &[&m.info.title, &m.info.original_title], m.info.year, m.info.runtime))
}

/// Searches Lantern for a movie and grabs the best release.
pub async fn search_movie(st: &AppState, id: i64) -> Result<Outcome> {
    let m = movies::find(&st.db, id)?.ok_or_else(|| err(404, "There is no such movie"))?;
    if already_downloading(&st.db, Some(id), &[])? {
        return Ok(Outcome { reason: Some(format!("{} is downloading already", m.info.title)), ..Outcome::default() });
    }
    let found = movie_candidates(st, &m).await?;
    let mut out = Outcome { looked_at: found.len(), ..Outcome::default() };
    match found.into_iter().find(Candidate::ok) {
        Some(c) => {
            let g = Lantern::from(st)?.grab("movie", &format!("movie:{id}"), &c.release).await?;
            tracing::info!("Grabbed {} for {}", c.release.title, m.info.title);
            out.grabbed.push(record(&st.db, "movie", Some(id), None, &c, &g)?);
        }
        None if out.looked_at == 0 => out.reason = Some(format!("Lantern found nothing for {}", m.info.title)),
        None => out.reason = Some(format!("None of the {} releases Lantern found for {} can be used", out.looked_at, m.info.title)),
    }
    Ok(out)
}

/// The episodes of a series that are wanted now: looked for, aired, and without a file (or with one the profile
/// would upgrade); `only` limits them to some ids, `season` to one season.
fn wanted_episodes(
    st: &AppState,
    s: &Series,
    profile: &Profile,
    only: &[i64],
    season: Option<i64>,
) -> Result<Vec<(Episode, WantedEpisode)>> {
    let now = Utc::now();
    let mut out = Vec::new();
    for e in series::episodes(&st.db, s.id)? {
        if !only.is_empty() && !only.contains(&e.id) {
            continue;
        }
        if season.is_some_and(|n| n != e.season) {
            continue;
        }
        // asked for by id: searched even when not looked for (like Sonarr's episode search)
        if only.is_empty() && (!e.monitored || !s.season_monitored(e.season)) {
            continue;
        }
        if !e.aired(now) {
            continue;
        }
        let current =
            if e.has_file() { episode_file(&st.db, e.episode_file_id)?.and_then(|(_, q)| profile.rank(&q).map(|r| r.0)) } else { None };
        if current.is_some() && (!profile.upgrades || current >= Some(profile.cutoff)) {
            continue;
        }
        let w = WantedEpisode {
            id: e.id,
            season: e.season,
            episode: e.episode,
            absolute: e.absolute,
            runtime: if e.runtime > 0 { e.runtime } else { s.info.runtime },
            current,
        };
        out.push((e, w));
    }
    Ok(out)
}

/// Searches Lantern for a series' wanted episodes (some ids, one season or all) and grabs the best releases:
/// a whole-season release when a season's every episode is wanted, else one release per episode.
pub async fn search_series(st: &AppState, id: i64, season: Option<i64>, only: &[i64]) -> Result<Outcome> {
    let s = series::find(&st.db, id)?.ok_or_else(|| err(404, "There is no such series"))?;
    let lantern = Lantern::from(st)?;
    let (profile, sizes) = rules(&st.db, &s.kind, s.quality_profile_id)?;
    let mut wanted = wanted_episodes(st, &s, &profile, only, season)?;
    let busy: Vec<i64> = active(&st.db)?.iter().filter(|d| d.status != "failed").flat_map(|d| d.episode_ids.clone()).collect();
    wanted.retain(|(e, _)| !busy.contains(&e.id));
    let mut out = Outcome::default();
    if wanted.is_empty() {
        out.reason = Some(format!("No episode of {} is wanted right now", s.info.title));
        return Ok(out);
    }
    let all_eps = series::episodes(&st.db, s.id)?;
    let season_sizes: Vec<(i64, usize)> = {
        let mut v: Vec<(i64, usize)> = Vec::new();
        for e in all_eps.iter().filter(|e| e.aired(Utc::now()) || e.air_date.is_none()) {
            match v.iter_mut().find(|(n, _)| *n == e.season) {
                Some(x) => x.1 += 1,
                None => v.push((e.season, 1)),
            }
        }
        v
    };
    let titles = [s.info.title.as_str(), s.info.original_title.as_str()];
    let cats = if s.kind == "anime" { vec![5070, 5000] } else { vec![5000] };
    let w = Wanted { kind: &s.kind, profile: &profile, sizes: &sizes, current: None };
    let mut seasons: Vec<i64> = wanted.iter().map(|(e, _)| e.season).collect();
    seasons.dedup();
    let mut done: Vec<i64> = Vec::new();
    let base = LQuery {
        kind: "tvsearch",
        terms: s.info.title.clone(),
        tvdb: s.info.tvdb_id,
        imdb: s.info.imdb_id.clone(),
        tmdb: s.info.tmdb_id,
        categories: cats,
        ..LQuery::default()
    };
    for n in seasons {
        let in_season: Vec<WantedEpisode> = wanted.iter().filter(|(e, _)| e.season == n).map(|(_, w)| w.clone()).collect();
        let whole = season_sizes.iter().find(|(x, _)| *x == n).map(|(_, c)| *c).unwrap_or(0);
        let mut picks: Vec<Candidate> = Vec::new();
        if in_season.len() >= 2 && in_season.len() >= whole {
            let found = lantern.search(&LQuery { season: Some(n), ..base.clone() }).await?;
            out.looked_at += found.len();
            let c = decide::for_episodes(&w, found, &titles, &in_season, &season_sizes);
            if let Some(best) = c.into_iter().find(|c| c.ok() && c.parsed.full_season) {
                picks.push(best);
            }
        }
        if picks.is_empty() {
            for e in &in_season {
                if done.contains(&e.id) {
                    continue;
                }
                let found = lantern.search(&LQuery { season: Some(n), episode: Some(e.episode), ..base.clone() }).await?;
                out.looked_at += found.len();
                let c = decide::for_episodes(&w, found, &titles, &in_season, &season_sizes);
                if let Some(best) = c.into_iter().find(|c| c.ok() && !c.parsed.full_season && c.episode_ids.contains(&e.id)) {
                    done.extend(best.episode_ids.iter().copied());
                    picks.push(best);
                }
            }
        }
        for c in picks {
            let refs = c.episode_ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            let g = lantern.grab(&s.kind, &format!("episodes:{refs}"), &c.release).await?;
            tracing::info!("Grabbed {} for {}", c.release.title, s.info.title);
            out.grabbed.push(record(&st.db, &s.kind, None, Some(s.id), &c, &g)?);
        }
    }
    if out.grabbed.is_empty() {
        out.reason = Some(if out.looked_at == 0 {
            format!("Lantern found nothing for {}", s.info.title)
        } else {
            format!("None of the {} releases Lantern found for {} can be used", out.looked_at, s.info.title)
        });
    }
    Ok(out)
}

/// Searches for every monitored movie that is out and has no file, and every wanted episode.
pub async fn search_missing(st: &AppState, movies_too: bool, tv_too: bool) {
    let now = Utc::now();
    if movies_too {
        for m in movies::all(&st.db).unwrap_or_default().into_iter().filter(|m| m.monitored && !m.has_file() && m.is_available(now)) {
            if let Err(e) = search_movie(st, m.id).await {
                tracing::warn!("Searching for {} didn't work: {}", m.info.title, e.detail);
            }
        }
    }
    if tv_too {
        for s in series::all(&st.db).unwrap_or_default().into_iter().filter(|s| s.monitored) {
            if let Err(e) = search_series(st, s.id, None, &[]).await {
                tracing::warn!("Searching for {} didn't work: {}", s.info.title, e.detail);
            }
        }
    }
}

/// Takes Lantern's news about a download: progress and status, and the import once it is finished.
pub async fn update(st: &AppState, g: &Grab) -> Result<Option<Download>> {
    let Some(d) = by_lantern(&st.db, &g.id)? else { return Ok(None) };
    if matches!(d.status.as_str(), "imported" | "importing") {
        return Ok(Some(d));
    }
    let status = if g.status.is_empty() { d.status.clone() } else { g.status.clone() };
    let message = match (&g.error, status.as_str()) {
        (Some(e), _) if !e.is_empty() => e.clone(),
        (_, "failed") => "Lantern couldn't download it".to_string(),
        _ => String::new(),
    };
    let files = if g.files.is_empty() { d.files.clone() } else { g.files.clone() };
    st.db.execute(
        "UPDATE downloads SET status=?, progress=?, left_bytes=?, eta=?, message=?, files=?, size=?, updated=? WHERE id=?",
        p![status, g.progress, g.left, g.eta, message, json!(files).to_string(), if g.size > 0 { g.size } else { d.size }, now_iso(), d.id],
    )?;
    if status == "completed" {
        import_download(st, d.id).await?;
    } else if status == "failed" {
        tracing::info!("{} failed in Lantern: {message}", d.title);
    }
    find(&st.db, d.id)
}

/// Marks a download and says why it can't be imported.
fn blocked(db: &SqliteDb, id: i64, why: &str) -> Result<()> {
    tracing::warn!("Download {id} can't be imported: {why}");
    db.execute("UPDATE downloads SET status='importBlocked', message=?, updated=? WHERE id=?", p![why, now_iso(), id]).map(|_| ())
}

/// Imports a finished download into the library and tells Lantern.
pub async fn import_download(st: &AppState, id: i64) -> Result<()> {
    let d = find(&st.db, id)?.ok_or_else(|| err(404, "There is no such download"))?;
    st.db.execute("UPDATE downloads SET status='importing', updated=? WHERE id=?", p![now_iso(), id])?;
    let videos = import::videos(&d.files);
    if videos.is_empty() {
        let why = if d.files.is_empty() {
            "Lantern sent no files for it".to_string()
        } else {
            format!("There is no video file in {}", d.files.join(", "))
        };
        return blocked(&st.db, id, &why);
    }
    let result = match (d.movie_id, d.series_id) {
        (Some(m), _) => import_movie(st, &d, m, &videos[0]),
        (_, Some(s)) => import_episodes(st, &d, s, &videos),
        _ => Err(err(400, "It isn't for a movie or a series")),
    };
    match result {
        Ok(()) => {
            st.db.execute(
                "UPDATE downloads SET status='imported', imported=?, message='', updated=? WHERE id=?",
                p![now_iso(), now_iso(), id],
            )?;
            if let Ok(l) = Lantern::from(st) {
                if let Err(e) = l.imported(&d.lantern_id).await {
                    tracing::warn!("Lantern wasn't told {} is imported: {}", d.title, e.detail);
                }
            }
            Ok(())
        }
        Err(e) => blocked(&st.db, id, &e.detail),
    }
}

/// An I/O error in words.
fn io(what: &str, e: std::io::Error) -> kit_server_core::Error {
    err(500, format!("{what}: {e}"))
}

/// Places a video (and its subtitles) at `to`; whether it was hardlinked.
fn place_with_subs(video: &std::path::Path, to: &std::path::Path) -> Result<bool> {
    let how = import::place(video, to).map_err(|e| io(&format!("{} can't be put in the library", video.display()), e))?;
    let stem = to.with_extension("");
    let vstem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    for sub in import::subtitles_of(video) {
        let name = sub.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let tail = &name[vstem.len().min(name.len())..];
        let dest = PathBuf::from(format!("{}{tail}", stem.display()));
        if let Err(e) = import::place(&sub, &dest) {
            tracing::warn!("The subtitles {} weren't brought along: {e}", sub.display());
        }
    }
    Ok(how == Placed::Hardlink)
}

/// Imports a movie's video.
fn import_movie(st: &AppState, d: &Download, movie_id: i64, video: &std::path::Path) -> Result<()> {
    let mut m = movies::find(&st.db, movie_id)?.ok_or_else(|| err(404, "The movie was removed from Marquee"))?;
    let name = import::movie_file_name(&m.info.title, m.info.year, &d.quality, video);
    let to = PathBuf::from(&m.path).join(&name);
    if m.has_file() {
        if let Some((old, _)) = movie_file(&st.db, m.movie_file_id)? {
            let old = PathBuf::from(old);
            if old.exists() && old != to {
                let gone =
                    import::recycle(&old, std::path::Path::new(&m.root_folder)).map_err(|e| io("The old file can't be moved aside", e))?;
                tracing::info!("{} was replaced; the old file is in {}", m.info.title, gone.display());
            }
        }
    }
    let linked = place_with_subs(video, &to)?;
    let size = std::fs::metadata(&to).map(|x| x.len() as i64).unwrap_or(0);
    let parsed = crate::release::parse(&d.title);
    let fid = st.db.execute(
        "INSERT INTO movie_files (movie_id, path, size, quality, release_group, scene_name, hardlinked, added) VALUES (?,?,?,?,?,?,?,?)",
        p![movie_id, to.to_string_lossy(), size, d.quality, parsed.group, d.title, linked, now_iso()],
    )?;
    if m.movie_file_id > 0 {
        st.db.execute("DELETE FROM movie_files WHERE id=?", p![m.movie_file_id])?;
    }
    m.movie_file_id = fid;
    st.db.execute("UPDATE movies SET movie_file_id=? WHERE id=?", p![fid, movie_id])?;
    tracing::info!(
        "{} is in the library at {}{}",
        m.info.title,
        to.display(),
        if linked { "" } else { " (copied: the download is on another disk)" }
    );
    Ok(())
}

/// Imports a series' videos: each file's name says its episodes, or the download's when there is one file.
fn import_episodes(st: &AppState, d: &Download, series_id: i64, videos: &[PathBuf]) -> Result<()> {
    let s = series::find(&st.db, series_id)?.ok_or_else(|| err(404, "The series was removed from Marquee"))?;
    let eps = series::episodes(&st.db, series_id)?;
    let whole = crate::release::parse(&d.title);
    let mut placed = 0;
    for v in videos {
        let name = v.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let p = crate::release::parse(name);
        let mine: Vec<&Episode> = match (p.season, p.absolute) {
            (Some(n), _) if !p.episodes.is_empty() => eps.iter().filter(|e| e.season == n && p.episodes.contains(&e.episode)).collect(),
            (None, Some(a)) => eps.iter().filter(|e| e.absolute == Some(a)).collect(),
            _ if videos.len() == 1 => eps.iter().filter(|e| d.episode_ids.contains(&e.id)).collect(),
            _ => vec![],
        };
        let mine: Vec<&Episode> = mine.into_iter().filter(|e| d.episode_ids.contains(&e.id) || whole.full_season).collect();
        if mine.is_empty() {
            tracing::info!("{name} in {} isn't a wanted episode; left out", d.title);
            continue;
        }
        let season = mine[0].season;
        let list: Vec<(i64, String)> = mine.iter().map(|e| (e.episode, e.title.clone())).collect();
        let file = import::episode_file_name(&s.info.title, season, &list, &d.quality, v);
        let dir = if s.season_folder { PathBuf::from(&s.path).join(import::season_folder(season)) } else { PathBuf::from(&s.path) };
        let to = dir.join(file);
        for e in &mine {
            if e.has_file() {
                if let Some((old, _)) = episode_file(&st.db, e.episode_file_id)? {
                    let old = PathBuf::from(old);
                    if old.exists() && old != to {
                        import::recycle(&old, std::path::Path::new(&s.root_folder))
                            .map_err(|e| io("The old file can't be moved aside", e))?;
                    }
                }
            }
        }
        let linked = place_with_subs(v, &to)?;
        let size = std::fs::metadata(&to).map(|x| x.len() as i64).unwrap_or(0);
        let fid = st.db.execute(
            "INSERT INTO episode_files (series_id, season, path, size, quality, release_group, scene_name, hardlinked, added) VALUES (?,?,?,?,?,?,?,?,?)",
            p![series_id, season, to.to_string_lossy(), size, d.quality, p.group, name, linked, now_iso()],
        )?;
        for e in &mine {
            st.db.execute("UPDATE episodes SET episode_file_id=? WHERE id=?", p![fid, e.id])?;
        }
        placed += 1;
    }
    st.db.execute(
        "DELETE FROM episode_files WHERE series_id=? AND id NOT IN (SELECT episode_file_id FROM episodes WHERE series_id=?)",
        p![series_id, series_id],
    )?;
    if placed == 0 {
        return Err(err(400, "None of its files is a wanted episode"));
    }
    Ok(())
}

/// Asks Lantern about every download on its way and takes the news (the fallback when Lantern can't call back).
pub async fn sync(st: &AppState) -> Result<()> {
    let lantern = Lantern::from(st)?;
    let grabs = lantern.grabs().await?;
    for d in active(&st.db)? {
        match grabs.iter().find(|g| g.id == d.lantern_id) {
            Some(g) => {
                update(st, g).await?;
            }
            None if d.status != "importBlocked" && d.status != "failed" => {
                st.db.execute(
                    "UPDATE downloads SET status='removed', message='Lantern has no such download any more', updated=? WHERE id=?",
                    p![now_iso(), d.id],
                )?;
            }
            None => {}
        }
    }
    Ok(())
}

/// The callback address and key Marquee registers with Lantern (from Settings → Connections).
fn callback(st: &AppState) -> Option<(String, String)> {
    let base = st.settings.str("marquee_url").trim().trim_end_matches('/').to_string();
    (!base.is_empty()).then(|| (format!("{base}/api/lantern/events"), st.settings.str("api_key")))
}

/// Runs in the background: registers with Lantern, asks about downloads every minute, and looks for everything
/// missing every 6 hours.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        let mut registered = String::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            tick += 1;
            let Ok(l) = Lantern::from(&st) else { continue };
            let (cb, key) = callback(&st).unwrap_or_default();
            let who = format!("{}|{}|{cb}", st.settings.str("lantern_url"), st.settings.str("lantern_api_key"));
            if registered != who {
                match l.register(&cb, &key).await {
                    Ok(_) => registered = who,
                    Err(e) => tracing::warn!("Marquee can't register with Lantern: {}", e.detail),
                }
            }
            if let Err(e) = sync(&st).await {
                tracing::warn!("Lantern can't be asked about downloads: {}", e.detail);
            }
            if tick.is_multiple_of(360) {
                search_missing(&st, true, true).await;
            }
        }
    });
}

/// `POST /api/lantern/events` body.
#[derive(Deserialize)]
pub struct Event {
    /// `grab.progress`, `grab.completed`, `grab.failed` or `grab.removed`
    pub event: String,
    /// the download as Lantern has it now
    pub grab: Grab,
}

/// `POST /api/lantern/events`: Lantern's news about a download (with Marquee's API key).
async fn event(State(st): State<AppState>, headers: HeaderMap, Json(b): Json<Event>) -> Result<Json<Value>> {
    let key = st.settings.str("api_key");
    let sent = headers.get("x-api-key").and_then(|v| v.to_str().ok()).unwrap_or("");
    if key.is_empty() || sent != key {
        return Err(err(401, "The API key is missing or wrong"));
    }
    let mut g = b.grab;
    if g.status.is_empty() {
        g.status = b.event.strip_prefix("grab.").unwrap_or("").replace("progress", "downloading");
    }
    let d = update(&st, &g).await?;
    Ok(Json(json!({ "ok": true, "known": d.is_some() })))
}

/// `GET /api/downloads`.
async fn list(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    Ok(Json(json!(all(&st.db)?.iter().map(public).collect::<Vec<_>>())))
}

/// `DELETE /api/downloads/{id}`: stops it in Lantern and forgets it.
async fn remove(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    let d = find(&st.db, id)?.ok_or_else(|| err(404, "There is no such download"))?;
    if d.active() {
        if let Ok(l) = Lantern::from(&st) {
            l.remove(&d.lantern_id, true).await?;
        }
    }
    st.db.execute("UPDATE downloads SET status='removed', updated=? WHERE id=?", p![now_iso(), id])?;
    Ok(Json(json!({ "ok": true })))
}

/// An outcome for the web app.
fn outcome(o: Outcome) -> Value {
    json!({ "grabbed": o.grabbed.iter().map(public).collect::<Vec<_>>(), "looked_at": o.looked_at, "reason": o.reason })
}

/// `POST /api/movies/{id}/search`.
async fn movie_search(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    Ok(Json(outcome(search_movie(&st, id).await?)))
}

/// `GET /api/movies/{id}/releases`.
async fn movie_releases(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    let m = movies::find(&st.db, id)?.ok_or_else(|| err(404, "There is no such movie"))?;
    Ok(Json(json!(movie_candidates(&st, &m).await?)))
}

/// `POST /api/series/{id}/search` body.
#[derive(Deserialize, Default)]
pub struct SeriesSearch {
    /// one season only
    pub season: Option<i64>,
    /// these episodes only
    #[serde(default)]
    pub episode_ids: Vec<i64>,
}

/// `POST /api/series/{id}/search`.
async fn series_search(State(st): State<AppState>, me: Me, Path(id): Path<i64>, b: Option<Json<SeriesSearch>>) -> Result<Json<Value>> {
    me.admin()?;
    let b = b.map(|Json(b)| b).unwrap_or_default();
    Ok(Json(outcome(search_series(&st, id, b.season, &b.episode_ids).await?)))
}

/// `POST /api/lantern/sync`.
async fn sync_now(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    sync(&st).await?;
    Ok(Json(json!({ "ok": true })))
}

/// The download routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/downloads", get(list))
        .route("/api/downloads/{id}", delete(remove))
        .route("/api/movies/{id}/search", post(movie_search))
        .route("/api/movies/{id}/releases", get(movie_releases))
        .route("/api/series/{id}/search", post(series_search))
        .route("/api/lantern/events", post(event))
        .route("/api/lantern/sync", post(sync_now))
}
