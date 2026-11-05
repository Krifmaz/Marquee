//! TRaSH Guides by default: quality profiles, custom formats and scores, quality sizes and naming for movies, TV
//! and anime, taken from the [TRaSH Guides](https://trash-guides.info/) data files (used as they are, MIT
//! licensed) and kept up to date. docs/TRACKS.md says who owns this folder.
//!
//! - [`snapshot`]: TRaSH's data files at one commit (one is built in, so the first start needs no network);
//! - [`sync`]: a newer snapshot from GitHub, fetching only the files that changed;
//! - [`convert`]: TRaSH's files as Marquee's profiles, sizes and naming;
//! - [`formats`]: custom formats, kept in the Radarr/Sonarr API shape, and matching releases against them;
//! - [`apply`]: writing it all into Marquee without overwriting a person's own edits, and the list of changes.
//!
//! On the first start (when `trash_auto` is on) Marquee sets up the defaults (HD Bluray + WEB for movies,
//! WEB-1080p for TV, TRaSH's anime profile, Jellyfin naming); the TRaSH page's short wizard changes them. Every
//! `trash_sync_hours` it looks for a newer TRaSH commit.
//!
//! ```text
//! GET  /api/trash                       → status, choices, what the wizard offers (signed in)
//! PUT  /api/trash/choices               → save the wizard's choices and apply them (admin)
//! POST /api/trash/sync                  → look for TRaSH updates now (admin)
//! GET  /api/trash/changes?status=       → what changed, newest first (signed in)
//! POST /api/trash/changes/{id}/take     → use TRaSH's version of a waiting change (admin)
//! POST /api/trash/changes/{id}/keep     → keep your own version (admin)
//! ```

pub mod apply;
pub mod convert;
pub mod dotnet_regex;
pub mod formats;
pub mod snapshot;
pub mod sync;

use crate::roles::Me;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use snapshot::Snapshot;
use std::time::Duration;

pub use formats::Format;

/// A media kind's name as people read it.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "movie" => "movies",
        "anime" => "anime",
        _ => "TV",
    }
}

/// What one media kind gets: the resolution (`1080p`, `2160p` or `both`) and the source (`web`, `bluray` (movies
/// only) or `remux`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pick {
    /// `1080p`, `2160p` or `both`
    pub resolution: String,
    /// `web`, `bluray` or `remux`
    pub source: String,
}

/// The wizard's choices.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Choices {
    /// the media server the naming suits: `jellyfin`, `plex`, `emby` or `none`
    pub server: String,
    /// movies
    pub movie: Pick,
    /// TV
    pub tv: Pick,
    /// whether anime gets TRaSH's anime profiles
    pub anime: bool,
}

impl Default for Choices {
    fn default() -> Choices {
        Choices {
            server: "jellyfin".into(),
            movie: Pick { resolution: "1080p".into(), source: "bluray".into() },
            tv: Pick { resolution: "1080p".into(), source: "web".into() },
            anime: true,
        }
    }
}

impl Choices {
    /// The choices with unknown values replaced by the defaults; 400 never (the wizard can't send nonsense that
    /// matters).
    fn clean(mut self) -> Choices {
        let d = Choices::default();
        if !["jellyfin", "plex", "emby", "none"].contains(&self.server.as_str()) {
            self.server = d.server;
        }
        for (p, dp, sources) in [(&mut self.movie, &d.movie, &["web", "bluray", "remux"][..]), (&mut self.tv, &d.tv, &["web", "remux"][..])]
        {
            if !["1080p", "2160p", "both"].contains(&p.resolution.as_str()) {
                p.resolution = dp.resolution.clone();
            }
            if !sources.contains(&p.source.as_str()) {
                p.source = dp.source.clone();
            }
        }
        self
    }
}

/// Makes this track's tables; anime gets its own quality list (a copy of TV's) so its sizes can differ.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    let e = |e: kit_server_core::Error| anyhow::anyhow!("The TRaSH tables can't be made: {}", e.detail);
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS trash_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS custom_formats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            body TEXT NOT NULL,
            UNIQUE(kind, name)
        );
        CREATE TABLE IF NOT EXISTS trash_naming (kind TEXT PRIMARY KEY, body TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS trash_managed (
            kind TEXT NOT NULL,
            item TEXT NOT NULL,
            trash_id TEXT NOT NULL,
            local_id INTEGER NOT NULL,
            applied TEXT NOT NULL,
            upstream TEXT NOT NULL,
            PRIMARY KEY (kind, item, trash_id)
        );
        CREATE TABLE IF NOT EXISTS trash_changes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            commit_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            item TEXT NOT NULL,
            trash_id TEXT NOT NULL,
            local_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            status TEXT NOT NULL,
            lines TEXT NOT NULL,
            proposed TEXT
        );",
    )
    .map_err(e)?;
    if db.one("SELECT id FROM quality_definitions WHERE kind='anime'", p![]).map_err(e)?.is_none() {
        db.execute("INSERT INTO quality_definitions (kind, quality_id, body) SELECT 'anime', quality_id, body FROM quality_definitions WHERE kind='tv' ORDER BY id", p![])
            .map_err(e)?;
    }
    Ok(())
}

/// A value kept in `trash_meta`.
pub fn meta(db: &SqliteDb, key: &str) -> Option<String> {
    db.one("SELECT value FROM trash_meta WHERE key=?", p![key]).ok().flatten().map(|r| r.s("value"))
}

/// Keeps a value in `trash_meta`.
pub fn set_meta(db: &SqliteDb, key: &str, value: &str) -> Result<()> {
    db.execute("INSERT INTO trash_meta (key, value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", p![key, value])?;
    Ok(())
}

/// The wizard's choices, `None` before the first setup.
pub fn choices(db: &SqliteDb) -> Option<Choices> {
    meta(db, "choices").and_then(|s| serde_json::from_str(&s).ok())
}

/// The naming kept for `movie` or `tv` (no id), `None` before TRaSH set it.
pub fn stored_naming(db: &SqliteDb, kind: &str) -> Result<Option<Value>> {
    Ok(db.one("SELECT body FROM trash_naming WHERE kind=?", p![kind])?.and_then(|r| serde_json::from_str(&r.s("body")).ok()))
}

/// Saves the choices and puts what they pick into Marquee. The first time, it also removes the stock starter
/// profiles nothing uses. (changes made, changes waiting for an admin)
pub fn setup(db: &SqliteDb, c: Choices) -> Result<(usize, usize)> {
    let c = c.clean();
    let first = choices(db).is_none();
    set_meta(db, "choices", &serde_json::to_string(&c).unwrap_or_default())?;
    let out = apply::apply(db, &Snapshot::load(db), &c)?;
    if first {
        remove_starters(db)?;
    }
    Ok(out)
}

/// Removes the stock starter profiles (Radarr's and Sonarr's own "Any", "HD-1080p"…) that nobody changed and
/// nothing uses, so the profile lists show TRaSH's. A changed or used one stays.
fn remove_starters(db: &SqliteDb) -> Result<()> {
    for (kind, file) in
        [("movie", include_str!("../../data/radarr/qualityprofile.json")), ("tv", include_str!("../../data/sonarr/qualityprofile.json"))]
    {
        let stock: Vec<Value> = serde_json::from_str(file).unwrap_or_default();
        for s in stock {
            let name = s["name"].as_str().unwrap_or("");
            let Some(r) = db.one("SELECT id, body FROM quality_profiles WHERE kind=? AND name=?", p![kind, name])? else { continue };
            let mut want = s.clone();
            if let Some(o) = want.as_object_mut() {
                o.remove("id");
            }
            let now: Value = serde_json::from_str(&r.s("body")).unwrap_or_default();
            if apply::fingerprint(&now) == apply::fingerprint(&want) {
                // refused while a movie or series uses it, which is what we want
                let _ = crate::quality::delete_profile(db, r.i("id"));
            }
        }
    }
    Ok(())
}

/// Serialises syncs (the schedule and Sync now).
static SYNCING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Looks for a newer TRaSH commit at `src` and applies it; the answer the TRaSH page shows.
pub async fn sync_from<S: sync::Source>(db: &SqliteDb, src: &S) -> Result<Value> {
    let _one = SYNCING.lock().await;
    let have = Snapshot::load(db);
    set_meta(db, "checked_at", &crate::movies::now_iso())?;
    let fetched = match sync::fetch(src, &have).await {
        Ok(f) => f,
        Err(e) => {
            set_meta(db, "last_error", &e.detail)?;
            return Err(e);
        }
    };
    set_meta(db, "last_error", "")?;
    let Some(snap) = fetched else {
        return Ok(json!({ "new": false, "commit": have.commit, "changed": 0, "waiting": 0 }));
    };
    snap.save(db)?;
    set_meta(db, "synced_at", &crate::movies::now_iso())?;
    let (changed, waiting) = match choices(db) {
        Some(c) => apply::apply(db, &snap, &c)?,
        None => (0, 0),
    };
    tracing::info!("TRaSH Guides {}: {changed} changes made, {waiting} waiting for you", &snap.commit[..snap.commit.len().min(7)]);
    Ok(json!({ "new": true, "commit": snap.commit, "changed": changed, "waiting": waiting }))
}

/// Looks for TRaSH updates on GitHub now.
pub async fn sync_now(st: &AppState) -> Result<Value> {
    let src = sync::GitHub { http: st.http.clone(), branch: "master".into() };
    sync_from(&st.db, &src).await
}

/// The first-start setup and the update schedule, when `trash_auto` is on.
pub fn spawn(st: AppState) {
    if !st.settings.bool("trash_auto") {
        return;
    }
    tokio::spawn(async move {
        if choices(&st.db).is_none() {
            match setup(&st.db, Choices::default()) {
                Ok((n, _)) => tracing::info!("Set up the TRaSH Guides defaults ({n} items)"),
                Err(e) => tracing::warn!("The TRaSH Guides defaults can't be set up: {}", e.detail),
            }
        }
        // the first look soon after the start, then on the schedule
        tokio::time::sleep(Duration::from_secs(120)).await;
        loop {
            if st.settings.bool("trash_auto") {
                if let Err(e) = sync_now(&st).await {
                    tracing::warn!("TRaSH Guides can't be updated: {}", e.detail);
                }
            }
            let hours = st.settings.int("trash_sync_hours").max(1) as u64;
            tokio::time::sleep(Duration::from_secs(hours * 3600)).await;
        }
    });
}

/// The custom formats the Radarr (`movie`) or Sonarr (`tv`) API lists at `customformat`, in their shape.
pub fn custom_formats(st: &AppState, kind: &str) -> Vec<Value> {
    formats::all(&st.db, kind).unwrap_or_default()
}

/// The naming the Radarr (`movie`) or Sonarr (`tv`) API answers at `config/naming`, `None` before TRaSH set it.
pub fn naming(st: &AppState, kind: &str) -> Option<Value> {
    let mut v = stored_naming(&st.db, if kind == "movie" { "movie" } else { "tv" }).ok().flatten()?;
    v["id"] = json!(1);
    Some(v)
}

/// The profile a new item of `kind` gets: the wizard's pick, while it exists.
pub fn default_profile(db: &SqliteDb, kind: &str) -> Option<i64> {
    let id: i64 = meta(db, &format!("default:{kind}"))?.parse().ok()?;
    db.one("SELECT id FROM quality_profiles WHERE id=? AND kind=?", p![id, kind]).ok().flatten().map(|_| id)
}

/// The custom formats releases of `kind` are scored with.
pub fn formats(db: &SqliteDb, kind: &str) -> std::sync::Arc<Vec<Format>> {
    formats::compiled(db, kind)
}

/// The TRaSH routes of Marquee's own API.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/trash", get(status))
        .route("/api/trash/choices", put(save_choices))
        .route("/api/trash/sync", post(sync_route))
        .route("/api/trash/changes", get(changes))
        .route("/api/trash/changes/{id}/take", post(|st: State<AppState>, me: Me, id: Path<i64>| decide(st, me, id, true)))
        .route("/api/trash/changes/{id}/keep", post(|st: State<AppState>, me: Me, id: Path<i64>| decide(st, me, id, false)))
}

/// What the wizard offers for one kind: the TRaSH profile each choice gives, with TRaSH's description.
fn options(snap: &Snapshot) -> Value {
    let describe = |key: &str| {
        snap.file(key).map(|p| json!({ "name": p["name"], "about": p["trash_description"].as_str().unwrap_or("").replace("<br>", "\n") }))
    };
    let mut out = serde_json::Map::new();
    for kind in ["movie", "tv"] {
        let mut list = Vec::new();
        let sources: &[&str] = if kind == "movie" { &["web", "bluray", "remux"] } else { &["web", "remux"] };
        for source in sources {
            for res in ["1080p", "2160p"] {
                let c = Choices {
                    movie: Pick { resolution: res.into(), source: source.to_string() },
                    tv: Pick { resolution: res.into(), source: source.to_string() },
                    anime: false,
                    ..Choices::default()
                };
                if let Some((_, file, _)) = apply::picks(&c).into_iter().find(|(k, _, _)| *k == kind) {
                    if let Some(d) = describe(&file) {
                        list.push(json!({ "source": source, "resolution": res, "profile": d }));
                    }
                }
            }
        }
        out.insert(kind.into(), json!(list));
    }
    out.insert("anime".into(), json!(describe("sonarr/quality-profiles/anime-remux-1080p.json")));
    Value::Object(out)
}

/// `GET /api/trash`.
async fn status(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    let snap = Snapshot::load(&st.db);
    let waiting = st.db.one("SELECT COUNT(*) n FROM trash_changes WHERE status='waiting'", p![])?.map(|r| r.i("n")).unwrap_or(0);
    let defaults: Value = ["movie", "tv", "anime"]
        .iter()
        .filter_map(|k| {
            let id = default_profile(&st.db, k)?;
            let p = crate::quality::profile(&st.db, id).ok().flatten()?;
            Some((k.to_string(), json!({ "id": id, "name": p["name"] })))
        })
        .collect::<serde_json::Map<String, Value>>()
        .into();
    Ok(Json(json!({
        "commit": snap.commit,
        "date": snap.date,
        "checkedAt": meta(&st.db, "checked_at"),
        "syncedAt": meta(&st.db, "synced_at"),
        "lastError": meta(&st.db, "last_error").filter(|s| !s.is_empty()),
        "setUp": choices(&st.db).is_some(),
        "choices": choices(&st.db).unwrap_or_default(),
        "defaults": defaults,
        "waiting": waiting,
        "auto": st.settings.bool("trash_auto"),
        "syncHours": st.settings.int("trash_sync_hours"),
        "options": options(&snap),
    })))
}

/// `PUT /api/trash/choices`: saves the wizard's choices and applies them.
async fn save_choices(State(st): State<AppState>, me: Me, Json(c): Json<Choices>) -> Result<Json<Value>> {
    me.admin()?;
    let (changed, waiting) = setup(&st.db, c)?;
    Ok(Json(json!({ "changed": changed, "waiting": waiting })))
}

/// `POST /api/trash/sync`.
async fn sync_route(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    sync_now(&st).await.map(Json)
}

/// `GET /api/trash/changes` query.
#[derive(Deserialize)]
struct ChangesQuery {
    /// only changes with this status (`waiting`, `updated`, `added`, `taken`, `kept`)
    status: Option<String>,
    /// at most this many (default 200)
    limit: Option<i64>,
}

/// `GET /api/trash/changes`: newest first.
async fn changes(State(st): State<AppState>, _me: Me, Query(q): Query<ChangesQuery>) -> Result<Json<Value>> {
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let rows = match q.status.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => st.db.query("SELECT * FROM trash_changes WHERE status=? ORDER BY id DESC LIMIT ?", p![s, limit])?,
        None => st.db.query("SELECT * FROM trash_changes WHERE status!='replaced' ORDER BY id DESC LIMIT ?", p![limit])?,
    };
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.i("id"),
                "at": r.s("at"),
                "commit": r.s("commit_id"),
                "kind": r.s("kind"),
                "item": r.s("item"),
                "name": r.s("name"),
                "status": r.s("status"),
                "lines": serde_json::from_str::<Value>(&r.s("lines")).unwrap_or_else(|_| json!([])),
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

/// `POST /api/trash/changes/{id}/take|keep`.
async fn decide(State(st): State<AppState>, me: Me, Path(id): Path<i64>, take: bool) -> Result<Json<Value>> {
    me.admin()?;
    if id <= 0 {
        return Err(err(404, "There is no such change"));
    }
    apply::decide(&st.db, id, take).map(Json)
}
