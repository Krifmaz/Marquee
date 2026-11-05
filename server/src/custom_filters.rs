//! Port of Sonarr's and Radarr's `CustomFilters` (`CustomFilterService`, `CustomFilterController`): filters people
//! save in the web app's lists (series, movies, queue, history…). Each app keeps its own, as upstream does in its
//! own database.
//!
//! ```text
//! GET    /<app>/api/v3/customfilter        → [{id, type, label, filters}]
//! GET    /<app>/api/v3/customfilter/{id}   → {id, type, label, filters}
//! POST   /<app>/api/v3/customfilter        → 201 {id, …}
//! PUT    /<app>/api/v3/customfilter/{id}   → 202 {id, …}
//! DELETE /<app>/api/v3/customfilter/{id}   → 200
//! ```

use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde_json::{json, Value};

/// The custom filter table: `app` is `sonarr` or `radarr`; `filters` is the JSON list as the web app sent it.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_filters (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app TEXT NOT NULL,
            type TEXT NOT NULL DEFAULT '',
            label TEXT NOT NULL DEFAULT '',
            filters TEXT NOT NULL DEFAULT '[]'
        );",
    )
    .map_err(|e| anyhow::anyhow!("The custom filter table can't be made: {}", e.detail))
}

/// A row as upstream's `CustomFilterResource`.
fn resource(r: &Row) -> Value {
    json!({
        "type": r.s("type"),
        "label": r.s("label"),
        "filters": serde_json::from_str::<Value>(&r.s("filters")).unwrap_or_else(|_| json!([])),
        "id": r.i("id"),
    })
}

/// Every filter of `app`.
pub fn all(db: &SqliteDb, app: &str) -> Result<Vec<Value>> {
    Ok(db.query("SELECT * FROM custom_filters WHERE app=? ORDER BY id", p![app])?.iter().map(resource).collect())
}

/// The filter `id` of `app`.
pub fn find(db: &SqliteDb, app: &str, id: i64) -> Result<Option<Value>> {
    Ok(db.one("SELECT * FROM custom_filters WHERE app=? AND id=?", p![app, id])?.as_ref().map(resource))
}

/// The fields of a body: (type, label, filters as JSON text).
fn fields(body: &Value) -> (String, String, String) {
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let filters = body.get("filters").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!([]));
    (text("type"), text("label"), filters.to_string())
}

/// Adds a filter for `app` and gives it back.
pub fn add(db: &SqliteDb, app: &str, body: &Value) -> Result<Value> {
    let (t, l, f) = fields(body);
    let id = db.execute("INSERT INTO custom_filters (app, type, label, filters) VALUES (?, ?, ?, ?)", p![app, t, l, f])?;
    find(db, app, id)?.ok_or_else(|| err(500, "The filter was saved but can't be read back"))
}

/// Replaces the filter `id` of `app`.
pub fn update(db: &SqliteDb, app: &str, id: i64, body: &Value) -> Result<Value> {
    if find(db, app, id)?.is_none() {
        return Err(err(404, "There is no such filter"));
    }
    let (t, l, f) = fields(body);
    db.execute("UPDATE custom_filters SET type=?, label=?, filters=? WHERE app=? AND id=?", p![t, l, f, app, id])?;
    find(db, app, id)?.ok_or_else(|| err(404, "There is no such filter"))
}

/// Removes the filter `id` of `app` (nothing happens when it is already gone, like upstream).
pub fn delete(db: &SqliteDb, app: &str, id: i64) -> Result<()> {
    db.execute("DELETE FROM custom_filters WHERE app=? AND id=?", p![app, id])?;
    Ok(())
}

/// A result as an answer with `status`.
fn answer(status: StatusCode, r: Result<Value>) -> Response {
    match r {
        Ok(v) => (status, Json(v)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// The custom filter routes of `app` (`sonarr` or `radarr`), to nest under its `/api/v3`.
pub fn routes(app: &'static str) -> Router<AppState> {
    Router::new()
        .route(
            "/customfilter",
            get(move |State(st): State<AppState>| async move { answer(StatusCode::OK, all(&st.db, app).map(|v| json!(v))) }).post(
                move |State(st): State<AppState>, Json(body): Json<Value>| async move { answer(StatusCode::CREATED, add(&st.db, app, &body)) },
            ),
        )
        .route(
            "/customfilter/{id}",
            get(move |State(st): State<AppState>, Path(id): Path<i64>| async move {
                answer(StatusCode::OK, find(&st.db, app, id).and_then(|v| v.ok_or_else(|| err(404, "There is no such filter"))))
            })
            .put(move |State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<Value>| async move {
                answer(StatusCode::ACCEPTED, update(&st.db, app, id, &body))
            })
            .delete(move |State(st): State<AppState>, Path(id): Path<i64>| async move {
                match delete(&st.db, app, id) {
                    Ok(()) => (StatusCode::OK, Json(json!({}))).into_response(),
                    Err(e) => e.into_response(),
                }
            }),
        )
}
