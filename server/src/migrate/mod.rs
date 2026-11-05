//! Moving in from Radarr and Sonarr: Marquee reads their API with their API key and adds every movie and series
//! with its quality profile (matched by name), root folder, tags and monitored choices, and keeps the files they
//! already have where they are. A dry run (`apply: false`, the default) only says what would happen.
//!
//! ```text
//! POST /api/migrate/radarr   {url, api_key, apply?, path_map?: [{from, to}]} → report (admins)
//! POST /api/migrate/sonarr   {url, api_key, apply?, path_map?: [{from, to}]} → report (admins)
//! ```
//!
//! `path_map` turns the other app's paths into Marquee's (for example `/movies` → `/data/media/movies` when the
//! containers mount the same disk in different places).

use crate::movies;
use crate::roles::Me;
use crate::series;
use crate::AppState;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// One path rewrite: paths starting with `from` start with `to` in Marquee.
#[derive(Clone, Debug, Deserialize)]
pub struct PathMap {
    /// the other app's folder
    pub from: String,
    /// the same folder as Marquee sees it
    pub to: String,
}

/// What to move in from: the app's address and API key, and whether to do it or only look.
#[derive(Clone, Debug, Deserialize)]
pub struct Source {
    /// the app's address, for example `http://radarr:7878`
    pub url: String,
    /// its API key (Settings → General)
    pub api_key: String,
    /// `true` adds everything; `false` (the default) is a dry run
    #[serde(default)]
    pub apply: bool,
    /// path rewrites, first match wins
    #[serde(default)]
    pub path_map: Vec<PathMap>,
}

/// One title in the report: what happens (or happened) to it.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    /// its title and year
    pub title: String,
    /// `add`, `skip` or `problem`
    pub action: String,
    /// why, in words
    pub detail: String,
    /// how many files Marquee keeps for it
    pub files: usize,
}

/// What a migration did, or would do.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    /// `radarr` or `sonarr`
    pub source: String,
    /// whether it was done (`false`: a dry run)
    pub applied: bool,
    /// titles added (or that would be)
    pub added: usize,
    /// titles left out because Marquee has them already
    pub skipped: usize,
    /// titles that can't be moved, with the reason
    pub problems: usize,
    /// files kept where they are
    pub files: usize,
    /// things to set up first or to know (profiles matched by name, folders added)
    pub notes: Vec<String>,
    /// every title
    pub items: Vec<Item>,
}

impl Report {
    /// Adds a title to the report and counts it.
    fn push(&mut self, title: String, action: &str, detail: String, files: usize) {
        match action {
            "add" => self.added += 1,
            "skip" => self.skipped += 1,
            _ => self.problems += 1,
        }
        self.files += files;
        self.items.push(Item { title, action: action.into(), detail, files });
    }
}

/// The migration routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/migrate/radarr", post(radarr_route)).route("/api/migrate/sonarr", post(sonarr_route))
}

/// A Radarr or Sonarr API, read with its key.
struct Arr<'a> {
    /// the shared HTTP client
    http: &'a reqwest::Client,
    /// `<url>/api/v3`
    base: String,
    /// its API key
    key: String,
    /// `Radarr` or `Sonarr`, for messages
    name: &'static str,
}

impl Arr<'_> {
    /// `GET <base><path>` as JSON, with a plain error when the app can't be reached or says no.
    async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let r = self
            .http
            .get(&url)
            .header("X-Api-Key", &self.key)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| err(502, format!("{} can't be reached at {}: {e}", self.name, self.base)))?;
        match r.status().as_u16() {
            200 => r.json().await.map_err(|e| err(502, format!("{} sent an answer Marquee can't read: {e}", self.name))),
            401 | 403 => Err(err(400, format!("{} turned the API key down. Copy it from {} → Settings → General.", self.name, self.name))),
            s => Err(err(502, format!("{} answered {s} for {path}", self.name))),
        }
    }
}

/// Turns the other app's path into Marquee's with the first matching rewrite.
pub fn map_path(maps: &[PathMap], path: &str) -> String {
    for m in maps {
        let from = m.from.trim_end_matches('/');
        if !from.is_empty() && (path == from || path.starts_with(&format!("{from}/"))) {
            return format!("{}{}", m.to.trim_end_matches('/'), &path[from.len()..]);
        }
    }
    path.to_string()
}

/// Checks the address and builds the client.
fn arr<'a>(st: &'a AppState, s: &Source, name: &'static str) -> Result<Arr<'a>> {
    let url = s.url.trim().trim_end_matches('/');
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(err(400, format!("{name}'s address must start with http:// or https://")));
    }
    if s.api_key.trim().is_empty() {
        return Err(err(400, format!("Add {name}'s API key (Settings → General in {name})")));
    }
    Ok(Arr { http: &st.http, base: format!("{url}/api/v3"), key: s.api_key.trim().to_string(), name })
}

/// Marquee's profile id for each of the other app's profile ids, matched by name (case ignored), else Marquee's
/// default profile for `kind`; a note for each one that had no match.
fn profile_map(db: &SqliteDb, kind: &str, theirs: &Value, notes: &mut Vec<String>) -> Result<HashMap<i64, i64>> {
    let ours = crate::quality::profiles(db, kind)?;
    let fallback = crate::quality::default_profile(db, kind)?;
    let fallback_name = ours.iter().find(|p| p["id"].as_i64() == Some(fallback)).and_then(|p| p["name"].as_str()).unwrap_or("").to_string();
    let mut map = HashMap::new();
    for p in theirs.as_array().into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or("");
        let id = ours.iter().find(|o| o["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))).and_then(|o| o["id"].as_i64());
        if id.is_none() {
            notes.push(format!("The quality profile \"{name}\" isn't in Marquee; its titles get \"{fallback_name}\""));
        }
        map.insert(p["id"].as_i64().unwrap_or(0), id.unwrap_or(fallback));
    }
    Ok(map)
}

/// Marquee's tag id for each of the other app's tag ids, made by label when `apply` (else counted in a note).
fn tag_map(db: &SqliteDb, theirs: &Value, apply: bool, notes: &mut Vec<String>) -> Result<HashMap<i64, i64>> {
    let ours = crate::tags::all(db)?;
    let mut map = HashMap::new();
    let mut new = Vec::new();
    for t in theirs.as_array().into_iter().flatten() {
        let label = t["label"].as_str().unwrap_or("").to_string();
        let found = ours.iter().find(|o| o["label"].as_str() == Some(label.as_str())).and_then(|o| o["id"].as_i64());
        let id = match (found, apply) {
            (Some(id), _) => Some(id),
            (None, true) => crate::tags::create(db, &label)?["id"].as_i64(),
            (None, false) => None,
        };
        if found.is_none() {
            new.push(label);
        }
        if let Some(id) = id {
            map.insert(t["id"].as_i64().unwrap_or(0), id);
        }
    }
    if !new.is_empty() {
        notes.push(format!("New tags{}: {}", if apply { "" } else { " to add" }, new.join(", ")));
    }
    Ok(map)
}

/// Adds the other app's root folders that Marquee doesn't have (when `apply`); notes for each, and for folders
/// Marquee can't see.
fn root_folders(db: &SqliteDb, kind: &str, theirs: &Value, maps: &[PathMap], apply: bool, notes: &mut Vec<String>) -> Result<()> {
    for f in theirs.as_array().into_iter().flatten() {
        let path = map_path(maps, f["path"].as_str().unwrap_or("").trim_end_matches('/'));
        if path.is_empty() || db.one("SELECT id FROM root_folders WHERE path=?", p![path])?.is_some() {
            continue;
        }
        if !std::path::Path::new(&path).is_dir() {
            notes.push(format!("Marquee can't see the folder {path}. Mount it into Marquee's container, or add a path rewrite."));
            continue;
        }
        if apply {
            db.execute("INSERT INTO root_folders (kind, path, created_at) VALUES (?,?,?)", p![kind, path, crate::auth::now()])?;
        }
        notes.push(format!("Root folder {path}{}", if apply { " added" } else { " will be added" }));
    }
    Ok(())
}

/// The quality name, size, release group and scene name of a Radarr or Sonarr file.
fn file_facts(f: &Value) -> (i64, String, String, String) {
    (
        f["size"].as_i64().unwrap_or(0),
        f["quality"]["quality"]["name"].as_str().unwrap_or("Unknown").to_string(),
        f["releaseGroup"].as_str().unwrap_or("").to_string(),
        f["sceneName"].as_str().unwrap_or("").to_string(),
    )
}

/// Moves in from Radarr: every movie, its profile, folder, tags, monitored choice and file.
pub async fn radarr(st: &AppState, s: &Source) -> Result<Report> {
    let a = arr(st, s, "Radarr")?;
    let mut r = Report { source: "radarr".into(), applied: s.apply, ..Report::default() };
    let profiles = profile_map(&st.db, "movie", &a.get("/qualityprofile").await?, &mut r.notes)?;
    let tags = tag_map(&st.db, &a.get("/tag").await?, s.apply, &mut r.notes)?;
    root_folders(&st.db, "movie", &a.get("/rootfolder").await?, &s.path_map, s.apply, &mut r.notes)?;
    let list = a.get("/movie").await?;
    for m in list.as_array().into_iter().flatten() {
        let title = format!("{} ({})", m["title"].as_str().unwrap_or("?"), m["year"].as_i64().unwrap_or(0));
        let tmdb_id = m["tmdbId"].as_i64().unwrap_or(0);
        if tmdb_id <= 0 {
            r.push(title, "problem", "It has no TMDB id".into(), 0);
            continue;
        }
        if movies::by_tmdb(&st.db, tmdb_id)?.is_some() {
            r.push(title, "skip", "Marquee has it already".into(), 0);
            continue;
        }
        let file = m["movieFile"].as_object().map(|_| &m["movieFile"]);
        let file_path = file.and_then(|f| f["path"].as_str()).map(|p| map_path(&s.path_map, p));
        let files = usize::from(file_path.is_some());
        if let Some(fp) = &file_path {
            if !std::path::Path::new(fp).is_file() {
                r.push(title, "problem", format!("Marquee can't see its file {fp}. Mount the folder, or add a path rewrite."), 0);
                continue;
            }
        }
        if !s.apply {
            r.push(title, "add", if files > 0 { "Will be added with its file".into() } else { "Will be added".into() }, files);
            continue;
        }
        let mut b = m.clone();
        let profile = profiles.get(&m["qualityProfileId"].as_i64().unwrap_or(0)).copied().unwrap_or(0);
        let path = map_path(&s.path_map, m["path"].as_str().unwrap_or(""));
        let root = map_path(&s.path_map, m["rootFolderPath"].as_str().unwrap_or("")).trim_end_matches('/').to_string();
        let root = if root.is_empty() {
            std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            root
        };
        b["tags"] =
            json!(m["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_i64().and_then(|t| tags.get(&t))).collect::<Vec<_>>());
        let info = match crate::radarr::details_for_add(st, tmdb_id, &b).await {
            Ok(i) => i,
            Err(e) => {
                r.push(title, "problem", e.detail, 0);
                continue;
            }
        };
        let o = crate::radarr::add_options(&b, profile, root, Some(path).filter(|p| !p.is_empty()));
        match movies::add(&st.db, &info, &o) {
            Ok(added) => {
                if let (Some(f), Some(fp)) = (file, &file_path) {
                    let (size, quality, group, scene) = file_facts(f);
                    crate::downloads::keep_movie_file(&st.db, added.id, fp, size, &quality, &group, &scene)?;
                }
                r.push(title, "add", if files > 0 { "Added with its file".into() } else { "Added".into() }, files);
            }
            Err(e) => r.push(title, "problem", e.detail, 0),
        }
    }
    tracing::info!(
        "Radarr migration{}: {} added, {} skipped, {} problems",
        if s.apply { "" } else { " (dry run)" },
        r.added,
        r.skipped,
        r.problems
    );
    Ok(r)
}

/// Moves in from Sonarr: every series, its profile, folder, type, tags, monitored seasons and episodes, and files.
pub async fn sonarr(st: &AppState, s: &Source) -> Result<Report> {
    let a = arr(st, s, "Sonarr")?;
    let mut r = Report { source: "sonarr".into(), applied: s.apply, ..Report::default() };
    let theirs = a.get("/qualityprofile").await?;
    let tv_profiles = profile_map(&st.db, "tv", &theirs, &mut r.notes)?;
    let mut anime_notes = Vec::new();
    // anime has its own profiles once TRaSH's are set up; until then anime series get the TV ones
    let anime_profiles = profile_map(&st.db, "anime", &theirs, &mut anime_notes).unwrap_or_else(|_| tv_profiles.clone());
    let tags = tag_map(&st.db, &a.get("/tag").await?, s.apply, &mut r.notes)?;
    root_folders(&st.db, "tv", &a.get("/rootfolder").await?, &s.path_map, s.apply, &mut r.notes)?;
    let list = a.get("/series").await?;
    for x in list.as_array().into_iter().flatten() {
        let title = format!("{} ({})", x["title"].as_str().unwrap_or("?"), x["year"].as_i64().unwrap_or(0));
        let tvdb_id = x["tvdbId"].as_i64().unwrap_or(0);
        let tmdb_hint = x["tmdbId"].as_i64().unwrap_or(0);
        if tvdb_id <= 0 {
            r.push(title, "problem", "It has no TheTVDB id".into(), 0);
            continue;
        }
        if series::by_tvdb(&st.db, tvdb_id)?.is_some() || (tmdb_hint > 0 && series::by_tmdb(&st.db, tmdb_hint)?.is_some()) {
            r.push(title, "skip", "Marquee has it already".into(), 0);
            continue;
        }
        let their_id = x["id"].as_i64().unwrap_or(0);
        let files = a.get(&format!("/episodefile?seriesId={their_id}")).await?;
        let files: Vec<&Value> = files.as_array().map(|v| v.iter().collect()).unwrap_or_default();
        let missing =
            files.iter().filter_map(|f| f["path"].as_str()).map(|p| map_path(&s.path_map, p)).find(|p| !std::path::Path::new(p).is_file());
        if let Some(p) = missing {
            r.push(title, "problem", format!("Marquee can't see its file {p}. Mount the folder, or add a path rewrite."), 0);
            continue;
        }
        if !s.apply {
            r.push(title, "add", format!("Will be added with {} files", files.len()), files.len());
            continue;
        }
        let anime = x["seriesType"] == "anime";
        let theirs_profile = x["qualityProfileId"].as_i64().unwrap_or(0);
        let profile = if anime { anime_profiles.get(&theirs_profile) } else { tv_profiles.get(&theirs_profile) }.copied().unwrap_or(0);
        let path = map_path(&s.path_map, x["path"].as_str().unwrap_or(""));
        let root = map_path(&s.path_map, x["rootFolderPath"].as_str().unwrap_or("")).trim_end_matches('/').to_string();
        let root = if root.is_empty() {
            std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            root
        };
        let mut b = x.clone();
        b["tags"] =
            json!(x["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_i64().and_then(|t| tags.get(&t))).collect::<Vec<_>>());
        let (info, eps) = match crate::sonarr::details_for_add(st, tvdb_id, tmdb_hint, &b).await {
            Ok(v) => v,
            Err(e) => {
                r.push(title, "problem", e.detail, 0);
                continue;
            }
        };
        // the seasons as Sonarr has them decide what is monitored, not a monitor choice
        if let Some(o) = b["addOptions"].as_object_mut() {
            o.remove("monitor");
        }
        let o = crate::sonarr::add_options(&b, profile, root, Some(path).filter(|p| !p.is_empty()));
        let added = match series::add(&st.db, &info, eps, &o) {
            Ok(added) => added,
            Err(e) => {
                r.push(title, "problem", e.detail, 0);
                continue;
            }
        };
        let kept = keep_episodes(st, &a, their_id, &added, &files, &s.path_map).await?;
        r.push(title, "add", format!("Added with {kept} files"), kept);
    }
    tracing::info!(
        "Sonarr migration{}: {} added, {} skipped, {} problems",
        if s.apply { "" } else { " (dry run)" },
        r.added,
        r.skipped,
        r.problems
    );
    Ok(r)
}

/// Copies Sonarr's episode monitored choices and files onto the series Marquee just added; how many files it kept.
async fn keep_episodes(st: &AppState, a: &Arr<'_>, their_id: i64, s: &series::Series, files: &[&Value], maps: &[PathMap]) -> Result<usize> {
    let theirs = a.get(&format!("/episode?seriesId={their_id}")).await?;
    let ours = series::episodes(&st.db, s.id)?;
    let find = |season: i64, episode: i64| ours.iter().find(|e| e.season == season && e.episode == episode);
    let mut kept = 0;
    let mut file_ids: HashMap<i64, i64> = HashMap::new();
    for f in files {
        let Some(path) = f["path"].as_str() else { continue };
        let (size, quality, group, scene) = file_facts(f);
        let season = f["seasonNumber"].as_i64().unwrap_or(0);
        let id = crate::downloads::keep_episode_file(&st.db, s.id, season, &map_path(maps, path), size, &quality, &group, &scene)?;
        file_ids.insert(f["id"].as_i64().unwrap_or(0), id);
        kept += 1;
    }
    for e in theirs.as_array().into_iter().flatten() {
        let Some(mine) = find(e["seasonNumber"].as_i64().unwrap_or(-1), e["episodeNumber"].as_i64().unwrap_or(-1)) else { continue };
        let monitored = e["monitored"].as_bool().unwrap_or(mine.monitored);
        let file = e["episodeFileId"].as_i64().and_then(|f| file_ids.get(&f)).copied().unwrap_or(0);
        st.db.execute("UPDATE episodes SET monitored=?, episode_file_id=? WHERE id=?", p![monitored, file, mine.id])?;
    }
    Ok(kept)
}

/// `POST /api/migrate/radarr`.
async fn radarr_route(State(st): State<AppState>, me: Me, Json(s): Json<Source>) -> Result<Json<Value>> {
    me.admin()?;
    Ok(Json(json!(radarr(&st, &s).await?)))
}

/// `POST /api/migrate/sonarr`.
async fn sonarr_route(State(st): State<AppState>, me: Me, Json(s): Json<Source>) -> Result<Json<Value>> {
    me.admin()?;
    Ok(Json(json!(sonarr(&st, &s).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_rewritten_by_the_first_matching_folder() {
        let maps = vec![
            PathMap { from: "/movies/".into(), to: "/data/media/movies".into() },
            PathMap { from: "/tv".into(), to: "/data/tv".into() },
        ];
        assert_eq!(map_path(&maps, "/movies/The Matrix (1999)/a.mkv"), "/data/media/movies/The Matrix (1999)/a.mkv");
        assert_eq!(map_path(&maps, "/movies"), "/data/media/movies");
        assert_eq!(map_path(&maps, "/moviesx/a"), "/moviesx/a", "only whole folders");
        assert_eq!(map_path(&maps, "/tv/Show"), "/data/tv/Show");
        assert_eq!(map_path(&[], "/x"), "/x");
    }
}
