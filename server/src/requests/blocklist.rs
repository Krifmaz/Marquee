//! Seerr's blocklist, ported from Seerr 3.5.1's `routes/blocklist.ts` and `entity/Blocklist.ts`: titles nobody
//! can ask for. A blocklisted title's status is Blocklisted (6) until it comes off the list.
//!
//! ```text
//! GET    /api/v1/blocklist                         the list, newest first (MANAGE_BLOCKLIST or VIEW_BLOCKLIST)
//! GET    /api/v1/blocklist/{tmdbId}?mediaType=     one entry (MANAGE_BLOCKLIST)
//! POST   /api/v1/blocklist                         add a title (MANAGE_BLOCKLIST)
//! DELETE /api/v1/blocklist/{tmdbId}?mediaType=     take a title off (MANAGE_BLOCKLIST)
//! POST   /api/v1/blocklist/collection/{id}         add every movie of a TMDB collection (MANAGE_BLOCKLIST)
//! DELETE /api/v1/blocklist/collection/{id}         take them all off (MANAGE_BLOCKLIST)
//! ```

use super::seerr::{has_any, perm, shown_user, Answer, Fail, Paging, Seerr};
use super::{media_by_tmdb, media_status, stamp};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The blocklist table.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_blocklist (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            tmdb_id INTEGER NOT NULL,
            title TEXT,
            user_id INTEGER,
            tags TEXT,
            created_at TEXT NOT NULL,
            UNIQUE(kind, tmdb_id)
        );",
    )
    .map_err(|e| anyhow::anyhow!("The blocklist table can't be made: {}", e.detail))
}

/// The routes.
pub fn router() -> Router<AppState> {
    let v1 = Router::new()
        .route("/blocklist", get(list).post(add))
        .route("/blocklist/collection/{id}", post(add_collection).delete(remove_collection))
        .route("/blocklist/{id}", get(one).delete(remove));
    Router::new().nest("/api/v1", v1)
}

/// What `GET /blocklist` takes.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// how many per page (25)
    pub take: Option<usize>,
    /// how many to skip
    pub skip: Option<usize>,
    /// part of a title to look for
    pub search: Option<String>,
    /// `all`, `manual` (added by hand) or `blocklistedTags` (added for their tags)
    pub filter: Option<String>,
}

/// Which title an entry is about: `?mediaType=movie|tv`.
#[derive(Deserialize, Default)]
pub struct KindQuery {
    /// `movie` or `tv`
    #[serde(rename = "mediaType")]
    pub media_type: Option<String>,
}

/// What `POST /blocklist` takes (Seerr's `blocklistAdd`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Add {
    /// the title's TMDB id
    pub tmdb_id: i64,
    /// `movie` or `tv`
    pub media_type: String,
    /// its name, shown in the list
    pub title: Option<String>,
    /// the account that blocklists it (Seerr's page sends the one signed in)
    pub user: Option<i64>,
    /// the tags it was blocklisted for, when it wasn't by hand
    pub blocklisted_tags: Option<String>,
}

/// An entry in Seerr's shape, with the account that added it.
fn entry_json(db: &SqliteDb, r: &Row, full: bool) -> kit_server_core::Result<Value> {
    let user = match r.get("user_id").and_then(Value::as_i64) {
        Some(id) => match db.one("SELECT * FROM users WHERE id=?", p![id])? {
            Some(u) => shown_user(db, &u, full)?,
            None => Value::Null,
        },
        None => Value::Null,
    };
    let text = |k: &str| r.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
    Ok(json!({
        "id": r.i("id"),
        "mediaType": r.s("kind"),
        "title": text("title"),
        "tmdbId": r.i("tmdb_id"),
        "user": user,
        "blocklistedTags": text("tags"),
        "createdAt": r.s("created_at"),
    }))
}

/// `movie` or `tv` from the query, or Seerr's 400.
fn kind_of(q: &KindQuery) -> std::result::Result<&str, Fail> {
    match q.media_type.as_deref() {
        Some(k @ ("movie" | "tv")) => Ok(k),
        _ => Err(Fail(err(400, "Invalid or missing mediaType query parameter."))),
    }
}

/// The entry for `kind`/`tmdb_id`, if there is one.
fn find(db: &SqliteDb, kind: &str, tmdb_id: i64) -> kit_server_core::Result<Option<Row>> {
    db.one("SELECT * FROM request_blocklist WHERE kind=? AND tmdb_id=?", p![kind, tmdb_id])
}

/// The answer when a title isn't on the list.
fn not_listed() -> Fail {
    Fail(err(404, "This title isn't on the blocklist."))
}

/// Puts `kind`/`tmdb_id` on the list and marks its title Blocklisted (Seerr's `addToBlocklist`); false when it
/// was already there.
fn block(
    db: &SqliteDb,
    kind: &str,
    tmdb_id: i64,
    title: Option<&str>,
    user: Option<i64>,
    tags: Option<&str>,
) -> kit_server_core::Result<bool> {
    if find(db, kind, tmdb_id)?.is_some() {
        return Ok(false);
    }
    let now = stamp();
    db.execute(
        "INSERT INTO request_blocklist (kind, tmdb_id, title, user_id, tags, created_at) VALUES (?,?,?,?,?,?)",
        p![kind, tmdb_id, title, user, tags, now],
    )?;
    match media_by_tmdb(db, kind, tmdb_id)? {
        Some(m) => {
            db.execute("UPDATE request_media SET status_override=?, updated_at=? WHERE id=?", p![media_status::BLOCKLISTED, now, m.id])?
        }
        None => db.execute(
            "INSERT INTO request_media (kind, tmdb_id, title, status_override, created_at, updated_at) VALUES (?,?,?,?,?,?)",
            p![kind, tmdb_id, title.unwrap_or(""), media_status::BLOCKLISTED, now, now],
        )?,
    };
    Ok(true)
}

/// Takes `kind`/`tmdb_id` off the list; its title goes back to what the library and its requests say. False when
/// it wasn't there.
fn unblock(db: &SqliteDb, kind: &str, tmdb_id: i64) -> kit_server_core::Result<bool> {
    let Some(r) = find(db, kind, tmdb_id)? else { return Ok(false) };
    db.execute("DELETE FROM request_blocklist WHERE id=?", p![r.i("id")])?;
    if let Some(m) = media_by_tmdb(db, kind, tmdb_id)?.filter(|m| m.status_override == Some(media_status::BLOCKLISTED)) {
        db.execute("UPDATE request_media SET status_override=NULL, updated_at=? WHERE id=?", p![stamp(), m.id])?;
    }
    Ok(true)
}

/// `GET /blocklist`: newest first, Seerr's paging (25 a page).
async fn list(State(st): State<AppState>, s: Seerr, Query(q): Query<ListQuery>) -> Answer {
    s.needs(&[perm::MANAGE_BLOCKLIST, perm::VIEW_BLOCKLIST])?;
    let mut sql = "SELECT * FROM request_blocklist WHERE 1=1".to_string();
    match q.filter.as_deref() {
        Some("manual") => sql.push_str(" AND tags IS NULL"),
        Some("blocklistedTags") => sql.push_str(" AND tags IS NOT NULL"),
        _ => {}
    }
    sql.push_str(" AND (? = '' OR title LIKE '%' || ? || '%') ORDER BY created_at DESC, id DESC");
    let search = q.search.clone().unwrap_or_default();
    let rows = st.db.query(&sql, p![search.clone(), search])?;
    let full = has_any(&s.0.row, &[perm::MANAGE_USERS]);
    let items = rows.iter().map(|r| entry_json(&st.db, r, full)).collect::<kit_server_core::Result<Vec<_>>>()?;
    Ok(Json(super::seerr::paged(items, &Paging { take: q.take, skip: q.skip }, 25)).into_response())
}

/// `GET /blocklist/{tmdbId}?mediaType=`.
async fn one(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Query(q): Query<KindQuery>) -> Answer {
    s.needs(&[perm::MANAGE_BLOCKLIST])?;
    let kind = kind_of(&q)?;
    let r = find(&st.db, kind, id)?.ok_or_else(not_listed)?;
    let full = has_any(&s.0.row, &[perm::MANAGE_USERS]);
    Ok(Json(entry_json(&st.db, &r, full)?).into_response())
}

/// `POST /blocklist`: 201, or 412 "Item already blocklisted".
async fn add(State(st): State<AppState>, s: Seerr, Json(b): Json<Add>) -> Answer {
    s.needs(&[perm::MANAGE_BLOCKLIST])?;
    if b.media_type != "movie" && b.media_type != "tv" {
        return Err(Fail(err(400, "Say whether it is a movie or a series (mediaType movie or tv).")));
    }
    let by = b.user.unwrap_or(s.0.id());
    if !block(&st.db, &b.media_type, b.tmdb_id, b.title.as_deref(), Some(by), b.blocklisted_tags.as_deref())? {
        return Err(Fail(err(412, "Item already blocklisted")));
    }
    Ok(StatusCode::CREATED.into_response())
}

/// `DELETE /blocklist/{tmdbId}?mediaType=`: 204.
async fn remove(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Query(q): Query<KindQuery>) -> Answer {
    s.needs(&[perm::MANAGE_BLOCKLIST])?;
    let kind = kind_of(&q)?;
    if !unblock(&st.db, kind, id)? {
        return Err(not_listed());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The movies of TMDB collection `id`, as `(tmdb id, title)`, each once.
async fn parts(st: &AppState, id: i64) -> std::result::Result<Vec<(i64, String)>, Fail> {
    let c = crate::movies::tmdb(st).get(&format!("/collection/{id}"), &[("language", "en".to_string())]).await?;
    let mut out: Vec<(i64, String)> = Vec::new();
    for p in c["parts"].as_array().into_iter().flatten() {
        let Some(pid) = p["id"].as_i64() else { continue };
        if !out.iter().any(|(i, _)| *i == pid) {
            out.push((pid, p["title"].as_str().unwrap_or("").to_string()));
        }
    }
    Ok(out)
}

/// `POST /blocklist/collection/{id}`: every movie of the collection not already on the list; 201.
async fn add_collection(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_BLOCKLIST])?;
    for (tmdb_id, title) in parts(&st, id).await? {
        block(&st.db, "movie", tmdb_id, Some(&title), Some(s.0.id()), None)?;
    }
    Ok(StatusCode::CREATED.into_response())
}

/// `DELETE /blocklist/collection/{id}`: every movie of the collection comes off the list; 204.
async fn remove_collection(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_BLOCKLIST])?;
    for (tmdb_id, _) in parts(&st, id).await? {
        unblock(&st.db, "movie", tmdb_id)?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
