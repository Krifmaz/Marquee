//! Root folders, shared by movies and TV: where each kind of media lives (the TRaSH layout puts them in
//! `/data/media/movies`, `/data/media/tv` and `/data/media/anime`, next to the downloads in `/data/torrents`, so imports can be
//! hardlinks or instant moves).
//!
//! ```text
//! GET    /api/root-folders        → [{id, kind, path, accessible, free_space}] (signed in)
//! POST   /api/root-folders        {kind: "movie" | "tv" | "anime", path} → the folder (admins)
//! DELETE /api/root-folders/{id}   → {ok} (admins; the files in it are left alone)
//! ```

use crate::roles::Me;
use crate::AppState;
use axum::extract::{Path, State};
use axum::routing::{delete, get};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The kinds of media a root folder can hold.
pub const KINDS: [&str; 3] = ["movie", "tv", "anime"];

/// The root folder table.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS root_folders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            path TEXT UNIQUE NOT NULL,
            created_at REAL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The root folder table can't be made: {}", e.detail))
}

/// The root folder routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/root-folders", get(list).post(add)).route("/api/root-folders/{id}", delete(remove))
}

/// The free bytes on the disk that holds `path`, `None` when it can't be read.
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    fs2::available_space(path).ok()
}

/// A root folder as clients see it: whether the server can open it now, and the free space there.
pub fn public(r: &Row) -> Value {
    let path = std::path::PathBuf::from(r.s("path"));
    json!({
        "id": r.i("id"),
        "kind": r.s("kind"),
        "path": r.s("path"),
        "accessible": path.is_dir(),
        "free_space": free_space(&path),
    })
}

/// Every root folder of `kind` (`movie` or `tv`), or of both when `None`.
pub fn folders(db: &SqliteDb, kind: Option<&str>) -> Result<Vec<Row>> {
    match kind {
        Some(k) => db.query("SELECT * FROM root_folders WHERE kind=? ORDER BY path", p![k]),
        None => db.query("SELECT * FROM root_folders ORDER BY kind, path", p![]),
    }
}

/// `GET /api/root-folders`.
async fn list(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    Ok(Json(json!(folders(&st.db, None)?.iter().map(public).collect::<Vec<_>>())))
}

/// `POST /api/root-folders` body.
#[derive(Deserialize)]
pub struct NewFolder {
    /// `movie`, `tv` or `anime`
    pub kind: String,
    /// the folder, as the server sees it (inside its container)
    pub path: String,
}

/// The cleaned form of a folder path: absolute, no trailing slash; 400 with the reason otherwise.
pub fn clean_path(path: &str) -> Result<String> {
    let p = path.trim();
    if !p.starts_with('/') {
        return Err(err(400, "The folder must be a full path that starts with / (for example /data/media/movies)"));
    }
    if p.split('/').any(|part| part == "..") {
        return Err(err(400, "The folder's path can't contain .."));
    }
    let p = p.trim_end_matches('/');
    Ok(if p.is_empty() { "/".into() } else { p.to_string() })
}

/// `POST /api/root-folders`: add a folder the server can open.
async fn add(State(st): State<AppState>, me: Me, Json(b): Json<NewFolder>) -> Result<Json<Value>> {
    me.admin()?;
    if !KINDS.contains(&b.kind.as_str()) {
        return Err(err(400, "The kind must be movie, tv or anime"));
    }
    let path = clean_path(&b.path)?;
    if !std::path::Path::new(&path).is_dir() {
        return Err(err(400, format!("The server can't find the folder {path}. Check that it exists and is mounted into the container.")));
    }
    if st.db.one("SELECT id FROM root_folders WHERE path=?", p![path])?.is_some() {
        return Err(err(409, format!("{path} is already a root folder")));
    }
    let id = st.db.execute("INSERT INTO root_folders (kind, path, created_at) VALUES (?,?,?)", p![b.kind, path, crate::auth::now()])?;
    let row =
        st.db.one("SELECT * FROM root_folders WHERE id=?", p![id])?.ok_or_else(|| err(500, "The new root folder can't be read back"))?;
    Ok(Json(public(&row)))
}

/// `DELETE /api/root-folders/{id}`: forget a root folder (its files stay where they are).
async fn remove(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    if st.db.one("SELECT id FROM root_folders WHERE id=?", p![id])?.is_none() {
        return Err(err(404, "There is no such root folder"));
    }
    st.db.execute("DELETE FROM root_folders WHERE id=?", p![id])?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::clean_path;

    #[test]
    fn paths_are_absolute_and_tidy() {
        assert_eq!(clean_path(" /data/media/movies/ ").unwrap(), "/data/media/movies");
        assert_eq!(clean_path("/").unwrap(), "/");
        assert!(clean_path("data/media").is_err());
        assert!(clean_path("/data/../etc").is_err());
    }
}
