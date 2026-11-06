//! TV series and their episodes: what Marquee keeps, how a new series' episodes are chosen to look for, how
//! TMDB's episode list is kept in step, and Marquee's own series API (the web app's). The Sonarr-compatible API
//! in [`crate::sonarr`] answers from the same tables. Anime series are series of the `anime` kind.
//!
//! ```text
//! GET    /api/series                     → [series] (signed in)
//! GET    /api/series/lookup?term=…       → TMDB's matches, each with `id` when it is already added (signed in)
//! POST   /api/series                     {tmdb_id | tvdb_id, quality_profile_id?, root_folder?, monitor?, series_type?}
//!                                        → the series, its details and episodes from TMDB (admins)
//! GET    /api/series/{id}                → one series with its seasons (signed in)
//! GET    /api/series/{id}/episodes       → its episodes (signed in)
//! PATCH  /api/series/{id}                {monitored?, quality_profile_id?, seasons?: [{season, monitored}], tags?} (admins)
//! DELETE /api/series/{id}                → {ok}; files on disk are left alone (admins)
//! ```

use crate::roles::Me;
use crate::tmdb::{EpisodeInfo, SeriesInfo};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Which episodes a new series looks for (Sonarr's choices).
pub const MONITOR: [&str; 10] =
    ["all", "future", "missing", "existing", "pilot", "firstSeason", "lastSeason", "monitorSpecials", "unmonitorSpecials", "none"];

/// How episodes are numbered: by season (`standard`), by date (`daily`) or absolutely (`anime`).
pub const SERIES_TYPES: [&str; 3] = ["standard", "daily", "anime"];

/// The series and episode tables.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS series (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tmdb_id INTEGER UNIQUE NOT NULL,
            tvdb_id INTEGER NOT NULL DEFAULT 0,
            imdb_id TEXT,
            title TEXT NOT NULL,
            sort_title TEXT NOT NULL,
            year INTEGER NOT NULL DEFAULT 0,
            info TEXT NOT NULL,
            path TEXT NOT NULL,
            root_folder TEXT NOT NULL,
            kind TEXT NOT NULL DEFAULT 'tv',
            quality_profile_id INTEGER NOT NULL,
            monitored INTEGER NOT NULL DEFAULT 1,
            season_folder INTEGER NOT NULL DEFAULT 1,
            series_type TEXT NOT NULL DEFAULT 'standard',
            monitor_new_items TEXT NOT NULL DEFAULT 'all',
            seasons TEXT NOT NULL DEFAULT '[]',
            tags TEXT NOT NULL DEFAULT '[]',
            added TEXT NOT NULL,
            last_info_sync TEXT
        );
        CREATE INDEX IF NOT EXISTS series_tvdb ON series(tvdb_id);
        CREATE TABLE IF NOT EXISTS episodes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            series_id INTEGER NOT NULL,
            tmdb_id INTEGER NOT NULL DEFAULT 0,
            season INTEGER NOT NULL,
            episode INTEGER NOT NULL,
            absolute INTEGER,
            title TEXT NOT NULL DEFAULT '',
            overview TEXT NOT NULL DEFAULT '',
            air_date TEXT,
            runtime INTEGER NOT NULL DEFAULT 0,
            monitored INTEGER NOT NULL DEFAULT 1,
            episode_file_id INTEGER NOT NULL DEFAULT 0,
            UNIQUE(series_id, season, episode)
        );
        CREATE INDEX IF NOT EXISTS episodes_air ON episodes(air_date);",
    )
    .map_err(|e| anyhow::anyhow!("The series tables can't be made: {}", e.detail))
}

/// One season of a series and whether Marquee looks for it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Season {
    /// the season number (0 is specials)
    pub season: i64,
    /// look for its episodes
    pub monitored: bool,
}

/// A series as Marquee keeps it.
#[derive(Clone, Debug)]
pub struct Series {
    /// Marquee's id
    pub id: i64,
    /// the details from TMDB
    pub info: SeriesInfo,
    /// its folder
    pub path: String,
    /// the root folder it is in
    pub root_folder: String,
    /// `tv` or `anime`
    pub kind: String,
    /// its quality profile
    pub quality_profile_id: i64,
    /// whether Marquee looks for it
    pub monitored: bool,
    /// whether episodes go into `Season N` folders
    pub season_folder: bool,
    /// `standard`, `daily` or `anime`
    pub series_type: String,
    /// whether new seasons are looked for: `all` or `none`
    pub monitor_new_items: String,
    /// its seasons
    pub seasons: Vec<Season>,
    /// tag ids
    pub tags: Vec<i64>,
    /// when it was added
    pub added: String,
}

/// An episode as Marquee keeps it.
#[derive(Clone, Debug)]
pub struct Episode {
    /// Marquee's id
    pub id: i64,
    /// its series
    pub series_id: i64,
    /// TMDB's id
    pub tmdb_id: i64,
    /// the season (0 is specials)
    pub season: i64,
    /// the episode in its season
    pub episode: i64,
    /// its number counted from the first episode (specials have none)
    pub absolute: Option<i64>,
    /// the title
    pub title: String,
    /// the summary
    pub overview: String,
    /// when it airs, `YYYY-MM-DD`
    pub air_date: Option<String>,
    /// minutes
    pub runtime: i64,
    /// look for it
    pub monitored: bool,
    /// its file's id, 0 while it has none
    pub episode_file_id: i64,
}

impl Series {
    /// The series of a table row.
    pub fn from_row(r: &Row) -> Series {
        Series {
            id: r.i("id"),
            info: serde_json::from_str(&r.s("info")).unwrap_or_default(),
            path: r.s("path"),
            root_folder: r.s("root_folder"),
            kind: r.s("kind"),
            quality_profile_id: r.i("quality_profile_id"),
            monitored: r.b("monitored"),
            season_folder: r.b("season_folder"),
            series_type: r.s("series_type"),
            monitor_new_items: r.s("monitor_new_items"),
            seasons: serde_json::from_str(&r.s("seasons")).unwrap_or_default(),
            tags: serde_json::from_str(&r.s("tags")).unwrap_or_default(),
            added: r.s("added"),
        }
    }

    /// `continuing`, `ended` or `upcoming`, from TMDB's status.
    pub fn status(&self) -> &'static str {
        match self.info.tmdb_status.as_str() {
            "Ended" | "Canceled" => "ended",
            "Planned" | "Pilot" => "upcoming",
            _ => "continuing",
        }
    }

    /// Whether the season `n` is looked for.
    pub fn season_monitored(&self, n: i64) -> bool {
        self.seasons.iter().find(|s| s.season == n).is_some_and(|s| s.monitored)
    }
}

impl Episode {
    /// The episode of a table row.
    pub fn from_row(r: &Row) -> Episode {
        Episode {
            id: r.i("id"),
            series_id: r.i("series_id"),
            tmdb_id: r.i("tmdb_id"),
            season: r.i("season"),
            episode: r.i("episode"),
            absolute: r.get("absolute").and_then(Value::as_i64),
            title: r.s("title"),
            overview: r.s("overview"),
            air_date: Some(r.s("air_date")).filter(|s| !s.is_empty()),
            runtime: r.i("runtime"),
            monitored: r.b("monitored"),
            episode_file_id: r.i("episode_file_id"),
        }
    }

    /// When it airs, as a time (midnight UTC: TMDB gives dates only).
    pub fn airs(&self) -> Option<DateTime<Utc>> {
        self.air_date.as_deref().and_then(crate::movies::day)
    }

    /// Whether it has aired by `now`.
    pub fn aired(&self, now: DateTime<Utc>) -> bool {
        self.airs().is_some_and(|a| a <= now)
    }

    /// Whether it has a file.
    pub fn has_file(&self) -> bool {
        self.episode_file_id > 0
    }
}

/// Every series, by title.
pub fn all(db: &SqliteDb) -> Result<Vec<Series>> {
    Ok(db.query("SELECT * FROM series ORDER BY sort_title", p![])?.iter().map(Series::from_row).collect())
}

/// The series `id`.
pub fn find(db: &SqliteDb, id: i64) -> Result<Option<Series>> {
    Ok(db.one("SELECT * FROM series WHERE id=?", p![id])?.as_ref().map(Series::from_row))
}

/// The series with TheTVDB id `tvdb_id`, if it is added.
pub fn by_tvdb(db: &SqliteDb, tvdb_id: i64) -> Result<Option<Series>> {
    if tvdb_id <= 0 {
        return Ok(None);
    }
    Ok(db.one("SELECT * FROM series WHERE tvdb_id=?", p![tvdb_id])?.as_ref().map(Series::from_row))
}

/// The series with TMDB id `tmdb_id`, if it is added.
pub fn by_tmdb(db: &SqliteDb, tmdb_id: i64) -> Result<Option<Series>> {
    Ok(db.one("SELECT * FROM series WHERE tmdb_id=?", p![tmdb_id])?.as_ref().map(Series::from_row))
}

/// The episodes of a series, in order.
pub fn episodes(db: &SqliteDb, series_id: i64) -> Result<Vec<Episode>> {
    Ok(db
        .query("SELECT * FROM episodes WHERE series_id=? ORDER BY season, episode", p![series_id])?
        .iter()
        .map(Episode::from_row)
        .collect())
}

/// Every episode of every series, in order.
pub fn all_episodes(db: &SqliteDb) -> Result<Vec<Episode>> {
    Ok(db.query("SELECT * FROM episodes ORDER BY air_date, series_id, season, episode", p![])?.iter().map(Episode::from_row).collect())
}

/// The episode `id`.
pub fn episode(db: &SqliteDb, id: i64) -> Result<Option<Episode>> {
    Ok(db.one("SELECT * FROM episodes WHERE id=?", p![id])?.as_ref().map(Episode::from_row))
}

/// Turns looking for the episodes `ids` on or off.
pub fn set_episodes_monitored(db: &SqliteDb, ids: &[i64], monitored: bool) -> Result<()> {
    for id in ids {
        db.execute("UPDATE episodes SET monitored=? WHERE id=?", p![monitored as i64, id])?;
    }
    Ok(())
}

/// Numbers the regular episodes 1, 2, 3… across seasons (specials get none).
pub fn absolute_numbers(eps: &mut [EpisodeInfo]) -> Vec<Option<i64>> {
    eps.sort_by_key(|e| (e.season, e.episode));
    let mut n = 0;
    eps.iter()
        .map(|e| {
            if e.season > 0 {
                n += 1;
                Some(n)
            } else {
                None
            }
        })
        .collect()
}

/// Whether a new series' episode is looked for under the `monitor` choice, on `now`.
pub fn monitored_by(monitor: &str, e: &EpisodeInfo, first_season: i64, last_season: i64, now: DateTime<Utc>) -> bool {
    let special = e.season == 0;
    let airs = e.air_date.as_deref().and_then(crate::movies::day);
    match monitor {
        "none" | "existing" => false,
        "future" => !special && airs.is_none_or(|a| a > now),
        "pilot" => e.season == 1 && e.episode == 1,
        "firstSeason" => e.season == first_season,
        "lastSeason" => e.season == last_season,
        "monitorSpecials" => true,
        // "all", "missing" (nothing has a file yet), "unmonitorSpecials"
        _ => !special,
    }
}

/// What a new series gets besides its details.
#[derive(Clone, Debug)]
pub struct AddOptions {
    /// its quality profile
    pub quality_profile_id: i64,
    /// the root folder
    pub root_folder: String,
    /// its folder, when the caller chose one
    pub path: Option<String>,
    /// whether Marquee looks for it
    pub monitored: bool,
    /// which episodes to look for ([`MONITOR`])
    pub monitor: String,
    /// `standard`, `daily` or `anime`
    pub series_type: String,
    /// episodes in `Season N` folders
    pub season_folder: bool,
    /// tag ids
    pub tags: Vec<i64>,
    /// seasons the caller chose (Seerr sends the ones it requested), overriding `monitor` for seasons
    pub seasons: Option<Vec<Season>>,
}

/// Adds the series `info` with its episodes `eps`; 409 when it is already there.
pub fn add(db: &SqliteDb, info: &SeriesInfo, mut eps: Vec<EpisodeInfo>, o: &AddOptions) -> Result<Series> {
    if by_tmdb(db, info.tmdb_id)?.is_some() || by_tvdb(db, info.tvdb_id)?.is_some() {
        return Err(err(409, "This series has already been added"));
    }
    if !SERIES_TYPES.contains(&o.series_type.as_str()) {
        return Err(err(400, "The series type must be standard, daily or anime"));
    }
    if !MONITOR.contains(&o.monitor.as_str()) {
        return Err(err(400, "Unknown choice of episodes to look for"));
    }
    if crate::quality::profile(db, o.quality_profile_id)?.is_none() {
        return Err(err(400, "There is no such quality profile"));
    }
    let root = crate::library::clean_path(&o.root_folder)?;
    let path = match &o.path {
        Some(p) if !p.trim().is_empty() => crate::library::clean_path(p)?,
        _ => format!("{}/{}", root.trim_end_matches('/'), folder_name(&info.title)),
    };
    let now = Utc::now();
    let abs = absolute_numbers(&mut eps);
    let regular: Vec<i64> = eps.iter().map(|e| e.season).filter(|s| *s > 0).collect();
    let (first, last) = (regular.iter().min().copied().unwrap_or(1), regular.iter().max().copied().unwrap_or(1));
    let mut season_numbers: Vec<i64> = info.seasons.clone();
    season_numbers.extend(eps.iter().map(|e| e.season));
    season_numbers.sort_unstable();
    season_numbers.dedup();
    let chosen = |n: i64| o.seasons.as_ref().and_then(|ss| ss.iter().find(|s| s.season == n)).map(|s| s.monitored);
    let seasons: Vec<Season> = season_numbers
        .iter()
        .map(|&n| {
            let by_choice = eps.iter().filter(|e| e.season == n).any(|e| monitored_by(&o.monitor, e, first, last, now));
            Season { season: n, monitored: chosen(n).unwrap_or(by_choice || (n > 0 && o.monitor == "future")) }
        })
        .collect();
    let kind = if o.series_type == "anime" { "anime" } else { "tv" };
    let id = db.execute(
        "INSERT INTO series (tmdb_id, tvdb_id, imdb_id, title, sort_title, year, info, path, root_folder, kind, quality_profile_id,
            monitored, season_folder, series_type, monitor_new_items, seasons, tags, added, last_info_sync)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        p![
            info.tmdb_id,
            info.tvdb_id,
            info.imdb_id,
            info.title,
            crate::movies::sort_title(&info.title),
            info.year,
            serde_json::to_string(info).unwrap_or_default(),
            path,
            root,
            kind,
            o.quality_profile_id,
            o.monitored as i64,
            o.season_folder as i64,
            o.series_type,
            if o.monitor == "none" { "none" } else { "all" },
            serde_json::to_string(&seasons).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&o.tags).unwrap_or_else(|_| "[]".into()),
            crate::movies::now_iso(),
            crate::movies::now_iso()
        ],
    )?;
    for (e, a) in eps.iter().zip(abs) {
        let on = match chosen(e.season) {
            Some(season_on) => season_on && (e.season > 0 || o.monitor == "monitorSpecials"),
            None => monitored_by(&o.monitor, e, first, last, now),
        };
        insert_episode(db, id, e, a, on)?;
    }
    find(db, id)?.ok_or_else(|| err(500, "The new series can't be read back"))
}

/// Adds one episode row.
fn insert_episode(db: &SqliteDb, series_id: i64, e: &EpisodeInfo, absolute: Option<i64>, monitored: bool) -> Result<()> {
    db.execute(
        "INSERT INTO episodes (series_id, tmdb_id, season, episode, absolute, title, overview, air_date, runtime, monitored)
            VALUES (?,?,?,?,?,?,?,?,?,?)",
        p![series_id, e.tmdb_id, e.season, e.episode, absolute, e.title, e.overview, e.air_date, e.runtime, monitored as i64],
    )?;
    Ok(())
}

/// Brings a series' episodes in step with TMDB's list `eps`: details updated, new episodes added (looked for
/// when their season is, or when new seasons are), episodes TMDB dropped removed unless they have a file.
pub fn sync_episodes(db: &SqliteDb, s: &mut Series, mut eps: Vec<EpisodeInfo>) -> Result<()> {
    let abs = absolute_numbers(&mut eps);
    let have = episodes(db, s.id)?;
    for (e, a) in eps.iter().zip(abs) {
        match have.iter().find(|h| h.season == e.season && h.episode == e.episode) {
            Some(h) => {
                db.execute(
                    "UPDATE episodes SET tmdb_id=?, absolute=?, title=?, overview=?, air_date=?, runtime=? WHERE id=?",
                    p![e.tmdb_id, a, e.title, e.overview, e.air_date, e.runtime, h.id],
                )?;
            }
            None => {
                if !s.seasons.iter().any(|x| x.season == e.season) {
                    let on = e.season > 0 && s.monitor_new_items == "all";
                    s.seasons.push(Season { season: e.season, monitored: on });
                    s.seasons.sort_by_key(|x| x.season);
                }
                let on = s.season_monitored(e.season);
                insert_episode(db, s.id, e, a, on)?;
            }
        }
    }
    for h in have.iter().filter(|h| !h.has_file() && !eps.iter().any(|e| e.season == h.season && e.episode == h.episode)) {
        db.execute("DELETE FROM episodes WHERE id=?", p![h.id])?;
    }
    save(db, s)
}

/// Saves a series' editable fields and details.
pub fn save(db: &SqliteDb, s: &Series) -> Result<()> {
    if crate::quality::profile(db, s.quality_profile_id)?.is_none() {
        return Err(err(400, "There is no such quality profile"));
    }
    if !SERIES_TYPES.contains(&s.series_type.as_str()) {
        return Err(err(400, "The series type must be standard, daily or anime"));
    }
    db.execute(
        "UPDATE series SET info=?, title=?, sort_title=?, year=?, tvdb_id=?, imdb_id=?, path=?, root_folder=?, quality_profile_id=?,
            monitored=?, season_folder=?, series_type=?, monitor_new_items=?, seasons=?, tags=? WHERE id=?",
        p![
            serde_json::to_string(&s.info).unwrap_or_default(),
            s.info.title,
            crate::movies::sort_title(&s.info.title),
            s.info.year,
            s.info.tvdb_id,
            s.info.imdb_id,
            s.path,
            s.root_folder,
            s.quality_profile_id,
            s.monitored as i64,
            s.season_folder as i64,
            s.series_type,
            s.monitor_new_items,
            serde_json::to_string(&s.seasons).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&s.tags).unwrap_or_else(|_| "[]".into()),
            s.id
        ],
    )?;
    Ok(())
}

/// Sets which seasons are looked for, and their episodes with them (specials only when asked for).
pub fn set_seasons(db: &SqliteDb, s: &mut Series, wanted: &[Season]) -> Result<()> {
    for w in wanted {
        match s.seasons.iter_mut().find(|x| x.season == w.season) {
            Some(x) if x.monitored != w.monitored => {
                x.monitored = w.monitored;
                db.execute("UPDATE episodes SET monitored=? WHERE series_id=? AND season=?", p![w.monitored as i64, s.id, w.season])?;
            }
            _ => {}
        }
    }
    save(db, s)
}

/// Forgets the series `id` and its episodes (files stay on disk); false when there was none.
pub fn remove(db: &SqliteDb, id: i64) -> Result<bool> {
    if find(db, id)?.is_none() {
        return Ok(false);
    }
    db.execute("DELETE FROM episodes WHERE series_id=?", p![id])?;
    db.execute("DELETE FROM series WHERE id=?", p![id])?;
    Ok(true)
}

/// A series' folder name: its title, with characters folders can't have replaced.
pub fn folder_name(title: &str) -> String {
    crate::movies::folder_name(title, 0)
}

/// Every episode TMDB lists for a series (each season asked for in turn).
pub async fn fetch_episodes(t: &crate::tmdb::Tmdb, info: &SeriesInfo) -> Result<Vec<EpisodeInfo>> {
    let mut out = Vec::new();
    for n in &info.seasons {
        out.extend(t.season(info.tmdb_id, *n).await?);
    }
    Ok(out)
}

/// Fetches a series' details and episodes again from TMDB and keeps them in step.
pub async fn refresh(st: &AppState, id: i64) -> Result<()> {
    let mut s = find(&st.db, id)?.ok_or_else(|| err(404, "There is no such series"))?;
    let t = crate::movies::tmdb(st);
    let info = t.series(s.info.tmdb_id).await?;
    let eps = fetch_episodes(&t, &info).await?;
    s.info = info;
    sync_episodes(&st.db, &mut s, eps)?;
    st.db.execute("UPDATE series SET last_info_sync=? WHERE id=?", p![crate::movies::now_iso(), id])?;
    Ok(())
}

/// The series routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/series", get(list).post(create))
        .route("/api/series/lookup", get(lookup))
        .route("/api/series/{id}", get(one).patch(update).delete(delete))
        .route("/api/series/{id}/episodes", get(episode_list))
}

/// A series as the web app gets it, with episode counts.
pub fn public(s: &Series, eps: &[Episode]) -> Value {
    let now = Utc::now();
    let aired = eps.iter().filter(|e| e.season > 0 && e.aired(now)).count();
    let files = eps.iter().filter(|e| e.has_file()).count();
    let next = eps.iter().filter(|e| !e.aired(now)).filter_map(|e| e.air_date.clone()).min();
    json!({
        "id": s.id,
        "tmdb_id": s.info.tmdb_id,
        "tvdb_id": s.info.tvdb_id,
        "title": s.info.title,
        "year": s.info.year,
        "overview": s.info.overview,
        "network": s.info.network,
        "status": s.status(),
        "poster": crate::movies::poster_url(&s.info.poster),
        "fanart": crate::movies::poster_url(&s.info.fanart),
        "kind": s.kind,
        "series_type": s.series_type,
        "monitored": s.monitored,
        "quality_profile_id": s.quality_profile_id,
        "path": s.path,
        "seasons": s.seasons.iter().map(|x| {
            let in_season: Vec<&Episode> = eps.iter().filter(|e| e.season == x.season).collect();
            json!({ "season": x.season, "monitored": x.monitored, "episodes": in_season.len(), "files": in_season.iter().filter(|e| e.has_file()).count() })
        }).collect::<Vec<_>>(),
        "aired_episodes": aired,
        "episode_files": files,
        "next_airing": next,
        "tags": s.tags,
        "added": s.added,
    })
}

/// An episode as the web app gets it.
pub fn public_episode(e: &Episode) -> Value {
    json!({
        "id": e.id, "season": e.season, "episode": e.episode, "absolute": e.absolute, "title": e.title, "overview": e.overview,
        "air_date": e.air_date, "runtime": e.runtime, "monitored": e.monitored, "has_file": e.has_file(),
    })
}

/// `GET /api/series`.
async fn list(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    let eps = all_episodes(&st.db)?;
    let out: Vec<Value> = all(&st.db)?
        .iter()
        .map(|s| {
            let mine: Vec<Episode> = eps.iter().filter(|e| e.series_id == s.id).cloned().collect();
            public(s, &mine)
        })
        .collect();
    Ok(Json(json!(out)))
}

/// `GET /api/series/{id}`.
async fn one(State(st): State<AppState>, _me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    let s = find(&st.db, id)?.ok_or_else(|| err(404, "There is no such series"))?;
    Ok(Json(public(&s, &episodes(&st.db, id)?)))
}

/// `GET /api/series/{id}/episodes`.
async fn episode_list(State(st): State<AppState>, _me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    if find(&st.db, id)?.is_none() {
        return Err(err(404, "There is no such series"));
    }
    Ok(Json(json!(episodes(&st.db, id)?.iter().map(public_episode).collect::<Vec<_>>())))
}

/// TMDB's series for `term`: words of a title, `tmdb:<id>`, `tvdb:<id>` or an IMDb id.
pub async fn search(st: &AppState, term: &str) -> Result<Vec<SeriesInfo>> {
    let t = term.trim();
    let tmdb = crate::movies::tmdb(st);
    if let Some(id) = t.strip_prefix("tmdb:").and_then(|i| i.trim().parse().ok()) {
        return Ok(vec![tmdb.series(id).await?]);
    }
    if let Some(id) = t.strip_prefix("tvdb:").and_then(|i| i.trim().parse::<i64>().ok()) {
        return match tmdb.find_tvdb(id).await? {
            Some(tid) => Ok(vec![tmdb.series(tid).await?]),
            None => Ok(vec![]),
        };
    }
    if let Some(imdb) = t.strip_prefix("imdb:").map(str::trim).or_else(|| t.starts_with("tt").then_some(t)) {
        return match tmdb.find_imdb_series(imdb).await? {
            Some(tid) => Ok(vec![tmdb.series(tid).await?]),
            None => Ok(vec![]),
        };
    }
    if t.is_empty() {
        return Ok(vec![]);
    }
    tmdb.search_series(t).await
}

/// `GET /api/series/lookup` query.
#[derive(Deserialize)]
pub struct Lookup {
    /// words of the title, or `tmdb:`, `tvdb:`, `imdb:` and an id
    pub term: String,
}

/// `GET /api/series/lookup?term=`.
async fn lookup(State(st): State<AppState>, _me: Me, Query(q): Query<Lookup>) -> Result<Json<Value>> {
    let mut out = Vec::new();
    for info in search(&st, &q.term).await? {
        let added = by_tmdb(&st.db, info.tmdb_id)?;
        out.push(json!({
            "tmdb_id": info.tmdb_id, "title": info.title, "year": info.year, "overview": info.overview,
            "poster": crate::movies::poster_url(&info.poster), "id": added.map(|s| s.id),
        }));
    }
    Ok(Json(json!(out)))
}

/// `POST /api/series` body.
#[derive(Deserialize)]
pub struct NewSeries {
    /// TMDB's id
    pub tmdb_id: Option<i64>,
    /// TheTVDB's id, when TMDB's isn't known
    pub tvdb_id: Option<i64>,
    /// the quality profile (default: HD-1080p, else the first TV profile)
    pub quality_profile_id: Option<i64>,
    /// the root folder (default: the first TV folder, or anime folder for anime)
    pub root_folder: Option<String>,
    /// which episodes to look for (default `all`)
    pub monitor: Option<String>,
    /// `standard` (default), `daily` or `anime`
    pub series_type: Option<String>,
    /// tag ids
    #[serde(default)]
    pub tags: Vec<i64>,
}

/// The first root folder of `kind`, 400 when there is none.
pub fn default_root(db: &SqliteDb, kind: &str) -> Result<String> {
    let words = if kind == "anime" { "an anime folder" } else { "a TV folder" };
    crate::library::folders(db, Some(kind))?
        .first()
        .map(|r| r.s("path"))
        .ok_or_else(|| err(400, format!("Add {words} in Settings → Folders first")))
}

/// `POST /api/series`: add a series by its TMDB (or TheTVDB) id.
async fn create(State(st): State<AppState>, me: Me, Json(b): Json<NewSeries>) -> Result<Json<Value>> {
    me.admin()?;
    let t = crate::movies::tmdb(&st);
    let tmdb_id = match (b.tmdb_id, b.tvdb_id) {
        (Some(id), _) => id,
        (None, Some(tvdb)) => t.find_tvdb(tvdb).await?.ok_or_else(|| err(404, "TMDB doesn't know this series"))?,
        _ => return Err(err(400, "Say which series: tmdb_id or tvdb_id")),
    };
    if by_tmdb(&st.db, tmdb_id)?.is_some() {
        return Err(err(409, "This series has already been added"));
    }
    let series_type = b.series_type.unwrap_or_else(|| "standard".into());
    let kind = if series_type == "anime" { "anime" } else { "tv" };
    let root = match b.root_folder {
        Some(r) if !r.trim().is_empty() => r,
        _ => default_root(&st.db, kind)?,
    };
    let profile = match b.quality_profile_id {
        Some(p) => p,
        None => crate::quality::default_profile(&st.db, "tv")?,
    };
    let info = t.series(tmdb_id).await?;
    let eps = fetch_episodes(&t, &info).await?;
    let o = AddOptions {
        quality_profile_id: profile,
        root_folder: root,
        path: None,
        monitored: true,
        monitor: b.monitor.unwrap_or_else(|| "all".into()),
        series_type,
        season_folder: true,
        tags: b.tags,
        seasons: None,
    };
    let s = add(&st.db, &info, eps, &o)?;
    crate::augment::series_added(&st);
    tracing::info!("{} added the series {}", me.account.username, s.info.title);
    Ok(Json(public(&s, &episodes(&st.db, s.id)?)))
}

/// `PATCH /api/series/{id}` body.
#[derive(Deserialize)]
pub struct SeriesChange {
    /// look for it
    pub monitored: Option<bool>,
    /// its quality profile
    pub quality_profile_id: Option<i64>,
    /// seasons to look for or not
    pub seasons: Option<Vec<Season>>,
    /// tag ids
    pub tags: Option<Vec<i64>>,
}

/// `PATCH /api/series/{id}`.
async fn update(State(st): State<AppState>, me: Me, Path(id): Path<i64>, Json(b): Json<SeriesChange>) -> Result<Json<Value>> {
    me.admin()?;
    let mut s = find(&st.db, id)?.ok_or_else(|| err(404, "There is no such series"))?;
    if let Some(v) = b.monitored {
        s.monitored = v;
    }
    if let Some(v) = b.quality_profile_id {
        s.quality_profile_id = v;
    }
    if let Some(v) = b.tags {
        s.tags = v;
    }
    match b.seasons {
        Some(ss) => set_seasons(&st.db, &mut s, &ss)?,
        None => save(&st.db, &s)?,
    }
    Ok(Json(public(&s, &episodes(&st.db, id)?)))
}

/// `DELETE /api/series/{id}`.
async fn delete(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    if !remove(&st.db, id)? {
        return Err(err(404, "There is no such series"));
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An episode on `date`.
    fn ep(season: i64, episode: i64, date: &str) -> EpisodeInfo {
        EpisodeInfo { season, episode, air_date: Some(date.into()), ..EpisodeInfo::default() }
    }

    #[test]
    fn absolute_numbers_skip_specials() {
        let mut eps = vec![ep(2, 1, "2024-01-01"), ep(0, 1, "2020-01-01"), ep(1, 2, "2022-01-08"), ep(1, 1, "2022-01-01")];
        assert_eq!(absolute_numbers(&mut eps), vec![None, Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn the_monitor_choice_picks_episodes() {
        let now = crate::movies::day("2026-11-05").unwrap();
        let (old, new, special) = (ep(1, 1, "2022-06-23"), ep(2, 1, "2026-11-12"), ep(0, 1, "2022-06-01"));
        assert!(monitored_by("all", &old, 1, 2, now) && !monitored_by("all", &special, 1, 2, now));
        assert!(!monitored_by("future", &old, 1, 2, now) && monitored_by("future", &new, 1, 2, now));
        assert!(monitored_by("pilot", &old, 1, 2, now) && !monitored_by("pilot", &new, 1, 2, now));
        assert!(monitored_by("lastSeason", &new, 1, 2, now) && !monitored_by("lastSeason", &old, 1, 2, now));
        assert!(monitored_by("monitorSpecials", &special, 1, 2, now));
        assert!(!monitored_by("none", &new, 1, 2, now));
    }
}
