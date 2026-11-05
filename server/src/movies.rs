//! Movies: what Marquee keeps for each one, when it counts as out, and Marquee's own movie API (the web app's).
//! The Radarr-compatible API in [`crate::radarr`] answers from the same table.
//!
//! ```text
//! GET    /api/movies                 → [movie] (signed in)
//! GET    /api/movies/lookup?term=…   → TMDB's matches, each with `id` when it is already added (signed in)
//! POST   /api/movies                 {tmdb_id, quality_profile_id?, root_folder?, monitored?, minimum_availability?}
//!                                    → the movie, its details from TMDB (admins)
//! GET    /api/movies/{id}            → one movie (signed in)
//! PATCH  /api/movies/{id}            {monitored?, quality_profile_id?, minimum_availability?, tags?} (admins)
//! DELETE /api/movies/{id}            → {ok}; files on disk are left alone (admins)
//! ```

use crate::roles::Me;
use crate::tmdb::MovieInfo;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// When a movie counts as out, so Marquee looks for it.
pub const AVAILABILITY: [&str; 3] = ["announced", "inCinemas", "released"];

/// Days after the cinema release a movie counts as released when it has no disc or digital date (Radarr's
/// rule).
const CINEMA_TO_RELEASE_DAYS: i64 = 90;

/// The movie table.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS movies (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tmdb_id INTEGER UNIQUE NOT NULL,
            imdb_id TEXT,
            title TEXT NOT NULL,
            sort_title TEXT NOT NULL,
            year INTEGER NOT NULL DEFAULT 0,
            info TEXT NOT NULL,
            path TEXT NOT NULL,
            root_folder TEXT NOT NULL,
            quality_profile_id INTEGER NOT NULL,
            monitored INTEGER NOT NULL DEFAULT 1,
            minimum_availability TEXT NOT NULL DEFAULT 'released',
            tags TEXT NOT NULL DEFAULT '[]',
            movie_file_id INTEGER NOT NULL DEFAULT 0,
            added TEXT NOT NULL,
            last_info_sync TEXT
        );
        CREATE INDEX IF NOT EXISTS movies_imdb ON movies(imdb_id);",
    )
    .map_err(|e| anyhow::anyhow!("The movie table can't be made: {}", e.detail))
}

/// A movie as Marquee keeps it.
#[derive(Clone, Debug)]
pub struct Movie {
    /// Marquee's id
    pub id: i64,
    /// the details from TMDB
    pub info: MovieInfo,
    /// the movie's folder
    pub path: String,
    /// the root folder it is in
    pub root_folder: String,
    /// its quality profile
    pub quality_profile_id: i64,
    /// whether Marquee looks for it and upgrades it
    pub monitored: bool,
    /// when it counts as out: `announced`, `inCinemas` or `released`
    pub minimum_availability: String,
    /// tag ids
    pub tags: Vec<i64>,
    /// its file's id, 0 while it has none
    pub movie_file_id: i64,
    /// when it was added, RFC 3339
    pub added: String,
    /// when its details were last fetched, RFC 3339
    pub last_info_sync: Option<String>,
}

impl Movie {
    /// The movie of a table row.
    pub fn from_row(r: &Row) -> Movie {
        Movie {
            id: r.i("id"),
            info: serde_json::from_str(&r.s("info")).unwrap_or_default(),
            path: r.s("path"),
            root_folder: r.s("root_folder"),
            quality_profile_id: r.i("quality_profile_id"),
            monitored: r.b("monitored"),
            minimum_availability: r.s("minimum_availability"),
            tags: serde_json::from_str(&r.s("tags")).unwrap_or_default(),
            movie_file_id: r.i("movie_file_id"),
            added: r.s("added"),
            last_info_sync: Some(r.s("last_info_sync")).filter(|s| !s.is_empty()),
        }
    }

    /// Whether it has a file.
    pub fn has_file(&self) -> bool {
        self.movie_file_id > 0
    }

    /// When it came or comes out on disc or online: the earlier of those two dates.
    pub fn release_date(&self) -> Option<String> {
        [&self.info.physical_release, &self.info.digital_release].into_iter().flatten().min().cloned()
    }

    /// Its stage on `now`: `tba` (no dates), `announced`, `inCinemas` or `released`.
    pub fn status(&self, now: DateTime<Utc>) -> &'static str {
        if self.released(now) {
            "released"
        } else if self.info.in_cinemas.as_deref().and_then(day).is_some_and(|d| d <= now) {
            "inCinemas"
        } else if self.info.in_cinemas.is_some() || self.release_date().is_some() || !self.info.tmdb_status.is_empty() {
            "announced"
        } else {
            "tba"
        }
    }

    /// Whether it is out on disc or online by `now` (or 90 days after cinemas when neither date is known).
    fn released(&self, now: DateTime<Utc>) -> bool {
        match self.release_date().as_deref().and_then(day) {
            Some(d) => d <= now,
            None => self.info.in_cinemas.as_deref().and_then(day).is_some_and(|d| d + Duration::days(CINEMA_TO_RELEASE_DAYS) <= now),
        }
    }

    /// Whether it counts as out by `now` for its minimum availability, so Marquee looks for it.
    pub fn is_available(&self, now: DateTime<Utc>) -> bool {
        match self.minimum_availability.as_str() {
            "announced" => true,
            "inCinemas" => matches!(self.status(now), "inCinemas" | "released"),
            _ => self.released(now),
        }
    }

    /// Whether any of its dates is between `start` and `end`.
    pub fn in_window(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        [&self.info.in_cinemas, &self.info.physical_release, &self.info.digital_release]
            .into_iter()
            .flatten()
            .filter_map(|d| day(d))
            .any(|d| d >= start && d <= end)
    }
}

/// Midnight UTC of a `YYYY-MM-DD` (or longer RFC 3339) date.
pub fn day(d: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc())
}

/// A date as Radarr writes it: `1999-03-30T00:00:00Z`.
pub fn iso_day(d: &str) -> String {
    format!("{}T00:00:00Z", d.get(..10).unwrap_or(d))
}

/// Now as RFC 3339 without fractions (`2026-11-05T01:00:00Z`).
pub fn now_iso() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// How a title sorts: lower case, without a leading "the", "a" or "an".
pub fn sort_title(title: &str) -> String {
    let t = title.trim().to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = t.strip_prefix(article) {
            return rest.trim().to_string();
        }
    }
    t
}

/// A title with only its letters and digits, lower case (`thematrix`): how titles are matched.
pub fn clean_title(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A movie's folder name, `Title (Year)`, with characters that folders can't have replaced.
pub fn folder_name(title: &str, year: i64) -> String {
    let t: String = title
        .chars()
        .map(|c| match c {
            ':' => '-',
            '/' | '\\' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c => c,
        })
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_end_matches('.').to_string();
    if year > 0 {
        format!("{t} ({year})")
    } else {
        t
    }
}

/// Every movie, by title.
pub fn all(db: &SqliteDb) -> Result<Vec<Movie>> {
    Ok(db.query("SELECT * FROM movies ORDER BY sort_title", p![])?.iter().map(Movie::from_row).collect())
}

/// The movie `id`, if there is one.
pub fn find(db: &SqliteDb, id: i64) -> Result<Option<Movie>> {
    Ok(db.one("SELECT * FROM movies WHERE id=?", p![id])?.as_ref().map(Movie::from_row))
}

/// The movie with TMDB id `tmdb_id`, if it is added.
pub fn by_tmdb(db: &SqliteDb, tmdb_id: i64) -> Result<Option<Movie>> {
    Ok(db.one("SELECT * FROM movies WHERE tmdb_id=?", p![tmdb_id])?.as_ref().map(Movie::from_row))
}

/// The movie with IMDb id `imdb_id`, if it is added.
pub fn by_imdb(db: &SqliteDb, imdb_id: &str) -> Result<Option<Movie>> {
    Ok(db.one("SELECT * FROM movies WHERE imdb_id=?", p![imdb_id])?.as_ref().map(Movie::from_row))
}

/// What a new movie gets besides its details.
#[derive(Clone, Debug)]
pub struct AddOptions {
    /// its quality profile
    pub quality_profile_id: i64,
    /// the root folder; its folder is `<root>/<Title (Year)>` unless `path` is given
    pub root_folder: String,
    /// its folder, when the caller chose one
    pub path: Option<String>,
    /// whether Marquee looks for it
    pub monitored: bool,
    /// `announced`, `inCinemas` or `released`
    pub minimum_availability: String,
    /// tag ids
    pub tags: Vec<i64>,
}

/// Adds the movie `info`; 409 when it is already there, 400 for an unknown profile or availability.
pub fn add(db: &SqliteDb, info: &MovieInfo, o: &AddOptions) -> Result<Movie> {
    if by_tmdb(db, info.tmdb_id)?.is_some() {
        return Err(err(409, "This movie has already been added"));
    }
    if !AVAILABILITY.contains(&o.minimum_availability.as_str()) {
        return Err(err(400, "Minimum availability must be announced, inCinemas or released"));
    }
    if crate::quality::profile(db, o.quality_profile_id)?.is_none() {
        return Err(err(400, "There is no such quality profile"));
    }
    let root = crate::library::clean_path(&o.root_folder)?;
    let path = match &o.path {
        Some(p) if !p.trim().is_empty() => crate::library::clean_path(p)?,
        _ => format!("{}/{}", root.trim_end_matches('/'), folder_name(&info.title, info.year)),
    };
    let now = now_iso();
    let id = db.execute(
        "INSERT INTO movies (tmdb_id, imdb_id, title, sort_title, year, info, path, root_folder, quality_profile_id, monitored,
            minimum_availability, tags, added, last_info_sync) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        p![
            info.tmdb_id,
            info.imdb_id,
            info.title,
            sort_title(&info.title),
            info.year,
            serde_json::to_string(info).unwrap_or_default(),
            path,
            root,
            o.quality_profile_id,
            o.monitored as i64,
            o.minimum_availability,
            serde_json::to_string(&o.tags).unwrap_or_else(|_| "[]".into()),
            now,
            now
        ],
    )?;
    find(db, id)?.ok_or_else(|| err(500, "The new movie can't be read back"))
}

/// Saves `m`'s editable fields (monitoring, profile, availability, tags, folder) and its details.
pub fn save(db: &SqliteDb, m: &Movie) -> Result<()> {
    if !AVAILABILITY.contains(&m.minimum_availability.as_str()) {
        return Err(err(400, "Minimum availability must be announced, inCinemas or released"));
    }
    if crate::quality::profile(db, m.quality_profile_id)?.is_none() {
        return Err(err(400, "There is no such quality profile"));
    }
    db.execute(
        "UPDATE movies SET monitored=?, quality_profile_id=?, minimum_availability=?, tags=?, path=?, root_folder=?, info=?,
            title=?, sort_title=?, year=?, imdb_id=? WHERE id=?",
        p![
            m.monitored as i64,
            m.quality_profile_id,
            m.minimum_availability,
            serde_json::to_string(&m.tags).unwrap_or_else(|_| "[]".into()),
            m.path,
            m.root_folder,
            serde_json::to_string(&m.info).unwrap_or_default(),
            m.info.title,
            sort_title(&m.info.title),
            m.info.year,
            m.info.imdb_id,
            m.id
        ],
    )?;
    Ok(())
}

/// Forgets the movie `id` (its files stay on disk); false when there was none.
pub fn remove(db: &SqliteDb, id: i64) -> Result<bool> {
    if find(db, id)?.is_none() {
        return Ok(false);
    }
    db.execute("DELETE FROM movies WHERE id=?", p![id])?;
    Ok(true)
}

/// A TMDB client with the server's key and address.
pub fn tmdb(st: &AppState) -> crate::tmdb::Tmdb {
    crate::tmdb::Tmdb::new(&st.settings.str("tmdb_api_key"), &st.settings.str("tmdb_url"), st.http.clone())
}

/// The movie routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/movies", get(list).post(create))
        .route("/api/movies/lookup", get(lookup))
        .route("/api/movies/{id}", get(one).patch(update).delete(delete))
}

/// A movie as the web app gets it.
pub fn public(m: &Movie) -> Value {
    let now = Utc::now();
    json!({
        "id": m.id,
        "tmdb_id": m.info.tmdb_id,
        "imdb_id": m.info.imdb_id,
        "title": m.info.title,
        "year": m.info.year,
        "overview": m.info.overview,
        "runtime": m.info.runtime,
        "genres": m.info.genres,
        "certification": m.info.certification,
        "poster": poster_url(&m.info.poster),
        "fanart": poster_url(&m.info.fanart),
        "status": m.status(now),
        "available": m.is_available(now),
        "has_file": m.has_file(),
        "monitored": m.monitored,
        "minimum_availability": m.minimum_availability,
        "quality_profile_id": m.quality_profile_id,
        "path": m.path,
        "root_folder": m.root_folder,
        "tags": m.tags,
        "in_cinemas": m.info.in_cinemas,
        "physical_release": m.info.physical_release,
        "digital_release": m.info.digital_release,
        "added": m.added,
    })
}

/// A TMDB picture's full address, "" for none.
pub fn poster_url(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!("{}{path}", crate::tmdb::IMAGE_BASE)
    }
}

/// `GET /api/movies`.
async fn list(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    Ok(Json(json!(all(&st.db)?.iter().map(public).collect::<Vec<_>>())))
}

/// `GET /api/movies/{id}`.
async fn one(State(st): State<AppState>, _me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    let m = find(&st.db, id)?.ok_or_else(|| err(404, "There is no such movie"))?;
    Ok(Json(public(&m)))
}

/// `GET /api/movies/lookup` query.
#[derive(Deserialize)]
pub struct Lookup {
    /// words of the title, or `tmdb:<id>` / `imdb:<tt…>`
    pub term: String,
}

/// TMDB's movies for `term`: words of a title (`The Matrix 1999` takes the year), `tmdb:603` or `imdb:tt0133093`.
pub async fn search(st: &AppState, term: &str) -> Result<Vec<MovieInfo>> {
    let t = term.trim();
    let tmdb = tmdb(st);
    if let Some(id) = t.strip_prefix("tmdb:").and_then(|i| i.trim().parse().ok()) {
        return Ok(vec![tmdb.movie(id).await?]);
    }
    if let Some(imdb) = t.strip_prefix("imdb:").map(str::trim).or_else(|| t.starts_with("tt").then_some(t)) {
        return match tmdb.find_imdb(imdb).await? {
            Some(id) => Ok(vec![tmdb.movie(id).await?]),
            None => Ok(vec![]),
        };
    }
    // a trailing year narrows the search: "Dune 2021"
    let (words, year) = match t.rsplit_once(' ') {
        Some((w, y)) if y.len() == 4 && y.parse::<i64>().is_ok_and(|y| (1880..=2100).contains(&y)) => (w, y.parse().ok()),
        _ => (t, None),
    };
    if words.is_empty() {
        return Ok(vec![]);
    }
    tmdb.search_movies(words, year).await
}

/// `GET /api/movies/lookup?term=`.
async fn lookup(State(st): State<AppState>, _me: Me, Query(q): Query<Lookup>) -> Result<Json<Value>> {
    let found = search(&st, &q.term).await?;
    let mut out = Vec::new();
    for info in found {
        let added = by_tmdb(&st.db, info.tmdb_id)?;
        out.push(json!({
            "tmdb_id": info.tmdb_id,
            "title": info.title,
            "year": info.year,
            "overview": info.overview,
            "poster": poster_url(&info.poster),
            "id": added.map(|m| m.id),
        }));
    }
    Ok(Json(json!(out)))
}

/// `POST /api/movies` body.
#[derive(Deserialize)]
pub struct NewMovie {
    /// TMDB's id
    pub tmdb_id: i64,
    /// the quality profile (default: the first movie profile)
    pub quality_profile_id: Option<i64>,
    /// the root folder (default: the first movie root folder)
    pub root_folder: Option<String>,
    /// look for it (default yes)
    pub monitored: Option<bool>,
    /// `announced`, `inCinemas` or `released` (default)
    pub minimum_availability: Option<String>,
    /// tag ids
    #[serde(default)]
    pub tags: Vec<i64>,
}

/// The first movie root folder, 400 when there is none.
pub fn default_root(db: &SqliteDb) -> Result<String> {
    crate::library::folders(db, Some("movie"))?
        .first()
        .map(|r| r.s("path"))
        .ok_or_else(|| err(400, "Add a movie folder in Settings → Folders first"))
}

/// `POST /api/movies`: add a movie by its TMDB id.
async fn create(State(st): State<AppState>, me: Me, Json(b): Json<NewMovie>) -> Result<Json<Value>> {
    me.admin()?;
    if by_tmdb(&st.db, b.tmdb_id)?.is_some() {
        return Err(err(409, "This movie has already been added"));
    }
    let root = match b.root_folder {
        Some(r) if !r.trim().is_empty() => r,
        _ => default_root(&st.db)?,
    };
    let profile = match b.quality_profile_id {
        Some(p) => p,
        None => crate::quality::default_profile(&st.db, "movie")?,
    };
    let info = tmdb(&st).movie(b.tmdb_id).await?;
    let o = AddOptions {
        quality_profile_id: profile,
        root_folder: root,
        path: None,
        monitored: b.monitored.unwrap_or(true),
        minimum_availability: b.minimum_availability.unwrap_or_else(|| "released".into()),
        tags: b.tags,
    };
    let m = add(&st.db, &info, &o)?;
    tracing::info!("{} added the movie {} ({})", me.account.username, m.info.title, m.info.year);
    Ok(Json(public(&m)))
}

/// `PATCH /api/movies/{id}` body.
#[derive(Deserialize)]
pub struct MovieChange {
    /// look for it
    pub monitored: Option<bool>,
    /// its quality profile
    pub quality_profile_id: Option<i64>,
    /// when it counts as out
    pub minimum_availability: Option<String>,
    /// tag ids
    pub tags: Option<Vec<i64>>,
}

/// `PATCH /api/movies/{id}`.
async fn update(State(st): State<AppState>, me: Me, Path(id): Path<i64>, Json(b): Json<MovieChange>) -> Result<Json<Value>> {
    me.admin()?;
    let mut m = find(&st.db, id)?.ok_or_else(|| err(404, "There is no such movie"))?;
    if let Some(v) = b.monitored {
        m.monitored = v;
    }
    if let Some(v) = b.quality_profile_id {
        m.quality_profile_id = v;
    }
    if let Some(v) = b.minimum_availability {
        m.minimum_availability = v;
    }
    if let Some(v) = b.tags {
        m.tags = v;
    }
    save(&st.db, &m)?;
    Ok(Json(public(&m)))
}

/// `DELETE /api/movies/{id}`: forget the movie; its files stay where they are.
async fn delete(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    if !remove(&st.db, id)? {
        return Err(err(404, "There is no such movie"));
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A movie with these dates and minimum availability.
    fn movie(cinemas: Option<&str>, physical: Option<&str>, digital: Option<&str>, min: &str) -> Movie {
        let info = MovieInfo {
            in_cinemas: cinemas.map(Into::into),
            physical_release: physical.map(Into::into),
            digital_release: digital.map(Into::into),
            ..MovieInfo::default()
        };
        Movie {
            id: 1,
            info,
            path: String::new(),
            root_folder: String::new(),
            quality_profile_id: 1,
            monitored: true,
            minimum_availability: min.into(),
            tags: vec![],
            movie_file_id: 0,
            added: String::new(),
            last_info_sync: None,
        }
    }

    #[test]
    fn status_and_availability_follow_the_dates() {
        let now = day("2026-11-05").unwrap();
        let matrix = movie(Some("1999-03-30"), Some("1999-09-21"), Some("2001-11-20"), "released");
        assert_eq!((matrix.status(now), matrix.is_available(now)), ("released", true));
        assert_eq!(matrix.release_date().as_deref(), Some("1999-09-21"));
        let future = movie(Some("2026-11-20"), None, Some("2027-01-10"), "released");
        assert_eq!((future.status(now), future.is_available(now)), ("announced", false));
        let showing = movie(Some("2026-10-30"), None, None, "inCinemas");
        assert_eq!((showing.status(now), showing.is_available(now)), ("inCinemas", true));
        let mut showing = showing;
        showing.minimum_availability = "released".into();
        assert!(!showing.is_available(now), "90 days after cinemas without other dates");
        assert_eq!(movie(None, None, None, "announced").status(now), "tba");
        assert!(movie(None, None, None, "announced").is_available(now));
    }

    #[test]
    fn titles_sort_match_and_name_folders() {
        assert_eq!(sort_title("The Matrix"), "matrix");
        assert_eq!(sort_title("An American Tail"), "american tail");
        assert_eq!(clean_title("The Matrix: Reloaded"), "thematrixreloaded");
        assert_eq!(folder_name("Mission: Impossible", 1996), "Mission- Impossible (1996)");
        assert_eq!(folder_name("What/If?", 2020), "What If (2020)");
        assert_eq!(iso_day("1999-03-30"), "1999-03-30T00:00:00Z");
    }
}
