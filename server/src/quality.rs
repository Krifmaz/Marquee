//! Qualities and quality profiles, kept per media kind (`movie` now; `tv` and `anime` later): which qualities
//! a profile takes, in what order, and where it stops upgrading. A profile is kept in the shape the Radarr and
//! Sonarr APIs use (`name`, `upgradeAllowed`, `cutoff`, `items`, `formatItems`…), so those APIs hand it over
//! as it is.
//!
//! The first start fills in the standard quality list and starter profiles from `data/radarr/` (the TRaSH
//! profiles replace them as the default in a later step).
//!
//! ```text
//! GET    /api/quality-profiles?kind=movie   → [profile] (signed in)
//! ```

use crate::roles::Me;
use crate::AppState;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// Radarr's standard quality list (sizes in MB per minute), the movie defaults.
const MOVIE_DEFINITIONS: &str = include_str!("../data/radarr/qualitydefinition.json");

/// The starter movie profiles.
const MOVIE_PROFILES: &str = include_str!("../data/radarr/qualityprofile.json");

/// The profile Marquee picks for a new movie when none is given, if it exists.
const PREFERRED_MOVIE_PROFILE: &str = "HD-1080p";

/// The quality tables, filled with the starter list and profiles on the first start.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    let e = |e: kit_server_core::Error| anyhow::anyhow!("The quality tables can't be made: {}", e.detail);
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS quality_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            body TEXT NOT NULL,
            UNIQUE(kind, name)
        );
        CREATE TABLE IF NOT EXISTS quality_definitions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            quality_id INTEGER NOT NULL,
            body TEXT NOT NULL,
            UNIQUE(kind, quality_id)
        );",
    )
    .map_err(e)?;
    seed(db, "movie", MOVIE_DEFINITIONS, MOVIE_PROFILES).map_err(e)
}

/// Fills in `kind`'s definitions and profiles when it has none yet.
fn seed(db: &SqliteDb, kind: &str, definitions: &str, profiles: &str) -> Result<()> {
    let parse =
        |s: &str| serde_json::from_str::<Vec<Value>>(s).map_err(|e| err(500, format!("A built-in quality file can't be read: {e}")));
    if db.one("SELECT id FROM quality_definitions WHERE kind=?", p![kind])?.is_none() {
        for mut d in parse(definitions)? {
            let q = d["quality"]["id"].as_i64().unwrap_or(0);
            strip_id(&mut d);
            db.execute("INSERT INTO quality_definitions (kind, quality_id, body) VALUES (?,?,?)", p![kind, q, d.to_string()])?;
        }
    }
    if db.one("SELECT id FROM quality_profiles WHERE kind=?", p![kind])?.is_none() {
        for mut pr in parse(profiles)? {
            strip_id(&mut pr);
            let name = pr["name"].as_str().unwrap_or("Profile").to_string();
            db.execute("INSERT INTO quality_profiles (kind, name, body) VALUES (?,?,?)", p![kind, name, pr.to_string()])?;
        }
    }
    Ok(())
}

/// Removes an object's `id` (the database gives its own).
fn strip_id(v: &mut Value) {
    if let Some(o) = v.as_object_mut() {
        o.remove("id");
    }
}

/// A body with `id` set.
fn with_id(body: &str, id: i64) -> Value {
    let mut v: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("id".into(), json!(id));
    }
    v
}

/// Every profile of `kind`, with ids.
pub fn profiles(db: &SqliteDb, kind: &str) -> Result<Vec<Value>> {
    Ok(db
        .query("SELECT * FROM quality_profiles WHERE kind=? ORDER BY id", p![kind])?
        .iter()
        .map(|r| with_id(&r.s("body"), r.i("id")))
        .collect())
}

/// The profile `id` (any kind), with its id.
pub fn profile(db: &SqliteDb, id: i64) -> Result<Option<Value>> {
    Ok(db.one("SELECT * FROM quality_profiles WHERE id=?", p![id])?.map(|r| with_id(&r.s("body"), r.i("id"))))
}

/// The profile a new item of `kind` gets: `HD-1080p` for movies when there is one, else the first.
pub fn default_profile(db: &SqliteDb, kind: &str) -> Result<i64> {
    if let Some(r) = db.one("SELECT id FROM quality_profiles WHERE kind=? AND name=?", p![kind, PREFERRED_MOVIE_PROFILE])? {
        return Ok(r.i("id"));
    }
    db.one("SELECT id FROM quality_profiles WHERE kind=? ORDER BY id LIMIT 1", p![kind])?
        .map(|r| r.i("id"))
        .ok_or_else(|| err(400, "There is no quality profile yet"))
}

/// Checks a profile body: a name and a list of items; 400 with the reason otherwise.
fn check(body: &Value) -> Result<String> {
    let name = body["name"].as_str().map(str::trim).unwrap_or("");
    if name.is_empty() {
        return Err(err(400, "A quality profile needs a name"));
    }
    if !body["items"].is_array() {
        return Err(err(400, "A quality profile needs its list of qualities (items)"));
    }
    Ok(name.to_string())
}

/// Adds a profile of `kind`; 409 when the name is taken.
pub fn create_profile(db: &SqliteDb, kind: &str, mut body: Value) -> Result<Value> {
    let name = check(&body)?;
    if db.one("SELECT id FROM quality_profiles WHERE kind=? AND name=?", p![kind, name])?.is_some() {
        return Err(err(409, "A quality profile with this name already exists"));
    }
    strip_id(&mut body);
    let id = db.execute("INSERT INTO quality_profiles (kind, name, body) VALUES (?,?,?)", p![kind, name, body.to_string()])?;
    Ok(with_id(&body.to_string(), id))
}

/// Replaces the profile `id`; 404 when there is none.
pub fn update_profile(db: &SqliteDb, id: i64, mut body: Value) -> Result<Value> {
    let name = check(&body)?;
    if profile(db, id)?.is_none() {
        return Err(err(404, "There is no such quality profile"));
    }
    strip_id(&mut body);
    db.execute("UPDATE quality_profiles SET name=?, body=? WHERE id=?", p![name, body.to_string(), id])?;
    Ok(with_id(&body.to_string(), id))
}

/// Removes the profile `id`; 400 while a movie uses it.
pub fn delete_profile(db: &SqliteDb, id: i64) -> Result<()> {
    if profile(db, id)?.is_none() {
        return Err(err(404, "There is no such quality profile"));
    }
    if db.one("SELECT id FROM movies WHERE quality_profile_id=? LIMIT 1", p![id])?.is_some() {
        return Err(err(400, "Movies still use this quality profile. Give them another one first."));
    }
    db.execute("DELETE FROM quality_profiles WHERE id=?", p![id])?;
    Ok(())
}

/// The quality list of `kind` with ids.
pub fn definitions(db: &SqliteDb, kind: &str) -> Result<Vec<Value>> {
    Ok(db
        .query("SELECT * FROM quality_definitions WHERE kind=? ORDER BY id", p![kind])?
        .iter()
        .map(|r| with_id(&r.s("body"), r.i("id")))
        .collect())
}

/// Replaces the quality definition `id` (its sizes and title); 404 when there is none.
pub fn update_definition(db: &SqliteDb, id: i64, mut body: Value) -> Result<Value> {
    let Some(r) = db.one("SELECT * FROM quality_definitions WHERE id=?", p![id])? else {
        return Err(err(404, "There is no such quality"));
    };
    strip_id(&mut body);
    // the quality itself never changes, only its title and sizes
    if let (Some(o), Ok(old)) = (body.as_object_mut(), serde_json::from_str::<Value>(&r.s("body"))) {
        o.insert("quality".into(), old["quality"].clone());
    }
    db.execute("UPDATE quality_definitions SET body=? WHERE id=?", p![body.to_string(), id])?;
    Ok(with_id(&body.to_string(), id))
}

/// The quality routes of Marquee's own API.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/quality-profiles", get(list))
}

/// `GET /api/quality-profiles` query.
#[derive(Deserialize)]
pub struct KindQuery {
    /// `movie`, `tv` or `anime`
    pub kind: Option<String>,
}

/// `GET /api/quality-profiles?kind=movie`: id and name of each profile.
async fn list(State(st): State<AppState>, _me: Me, Query(q): Query<KindQuery>) -> Result<Json<Value>> {
    let kind = q.kind.unwrap_or_else(|| "movie".into());
    let out: Vec<Value> = profiles(&st.db, &kind)?.iter().map(|p| json!({ "id": p["id"], "name": p["name"] })).collect();
    Ok(Json(json!(out)))
}
