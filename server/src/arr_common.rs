//! The routes the Radarr- and Sonarr-compatible APIs answer the same way, for one [`App`]: health, disks,
//! queue and history, quality profiles and definitions, languages, tags, root folders, indexers and commands.
//! Each app adds its own media routes next to them.

use crate::arr::{self, not_found, paged, validation, with_status, Paging};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use kit_server_core::{p, Db, Result, Row, RowExt};
use serde_json::{json, Value};

/// What differs between the two apps in the shared routes.
pub struct App {
    /// `radarr` or `sonarr`: the key of its commands and indexers
    pub name: &'static str,
    /// the quality profile kind it shows (`movie`, `tv`)
    pub profile_kind: &'static str,
    /// the root folder kinds it shows; the first is what a new root folder gets
    pub root_kinds: &'static [&'static str],
    /// its language list (`[{id, name, nameLower}]`)
    pub languages: &'static str,
    /// its Torznab and Newznab indexer templates
    pub indexer_schema: &'static str,
    /// its queue records (downloads on their way, in its shape)
    pub queue: fn(&AppState) -> Vec<Value>,
}

/// A database failure as an answer.
pub fn fail(e: kit_server_core::Error) -> Response {
    e.into_response()
}

/// `[]`.
async fn empty_list() -> Json<Value> {
    Json(json!([]))
}

/// Upstream's `IndexerFlagController`: the release flags indexers can set, as `[{id, name, nameLower}]`. Sonarr lists its
/// whole `IndexerFlags` enum; Radarr leaves out the ones marked obsolete (`AHD_Internal`, `AHD_UserRelease`).
pub fn indexer_flags(app: &str) -> Value {
    let flags: &[(i64, &str)] = if app == "radarr" {
        &[
            (1, "G_Freeleech"),
            (2, "G_Halfleech"),
            (4, "G_DoubleUpload"),
            (8, "PTP_Golden"),
            (16, "PTP_Approved"),
            (32, "G_Internal"),
            (128, "G_Scene"),
            (2048, "Nuked"),
        ]
    } else {
        &[(1, "Freeleech"), (2, "Halfleech"), (4, "DoubleUpload"), (8, "Internal"), (16, "Scene"), (128, "Nuked")]
    };
    Value::Array(flags.iter().map(|(id, name)| json!({ "id": id, "name": name, "nameLower": name.to_lowercase() })).collect())
}

/// The shared routes for `app`, to nest under its `/api/v3`.
pub fn routes(app: &'static App) -> Router<AppState> {
    Router::new()
        .route("/health", get(empty_list))
        .route("/diskspace", get(move |State(st): State<AppState>| async move { diskspace(&st, app) }))
        .route("/queue", get(move |State(st): State<AppState>, Query(q): Query<Paging>| async move { Json(paged(&q, "timeleft", "descending", (app.queue)(&st))) }))
        .route("/queue/status", get(move |State(st): State<AppState>| async move { queue_status(&st, app) }))
        .route("/queue/details", get(move |State(st): State<AppState>| async move { Json(json!((app.queue)(&st))) }))
        .route("/history", get(|Query(q): Query<Paging>| async move { Json(paged(&q, "date", "descending", vec![])) }))
        .route("/customformat", get(move |State(st): State<AppState>| async move { Json(json!(crate::trash::custom_formats(&st, app.profile_kind))) }))
        .route("/notification", get(empty_list))
        .route("/downloadclient", get(empty_list))
        .route("/importlist", get(empty_list))
        .route("/language", get(move || async move { Json(serde_json::from_str::<Value>(app.languages).unwrap_or_else(|_| json!([]))) }))
        .route(
            "/qualityprofile",
            get(move |State(st): State<AppState>| async move { crate::quality::profiles(&st.db, app.profile_kind).map(|v| Json(json!(v))) })
                .post(move |State(st): State<AppState>, Json(b): Json<Value>| async move { answer(201, crate::quality::create_profile(&st.db, app.profile_kind, b)) }),
        )
        .route(
            "/qualityprofile/{id}",
            get(|State(st): State<AppState>, Path(id): Path<i64>| async move { found(crate::quality::profile(&st.db, id)) })
                .put(|State(st): State<AppState>, Path(id): Path<i64>, Json(b): Json<Value>| async move { answer(202, crate::quality::update_profile(&st.db, id, b)) })
                .delete(|State(st): State<AppState>, Path(id): Path<i64>| async move { answer(200, crate::quality::delete_profile(&st.db, id).map(|_| json!({}))) }),
        )
        .route(
            "/qualitydefinition",
            get(move |State(st): State<AppState>| async move { crate::quality::definitions(&st.db, app.profile_kind).map(|v| Json(json!(v))) }),
        )
        .route("/qualitydefinition/update", put(definitions_put))
        .route(
            "/qualitydefinition/{id}",
            put(|State(st): State<AppState>, Path(id): Path<i64>, Json(b): Json<Value>| async move { answer(202, crate::quality::update_definition(&st.db, id, b)) }),
        )
        .route(
            "/tag",
            get(|State(st): State<AppState>| async move { crate::tags::all(&st.db).map(|v| Json(json!(v))) })
                .post(|State(st): State<AppState>, Json(b): Json<Value>| async move { answer(201, crate::tags::create(&st.db, b["label"].as_str().unwrap_or(""))) }),
        )
        .route(
            "/tag/{id}",
            get(|State(st): State<AppState>, Path(id): Path<i64>| async move { found(crate::tags::find(&st.db, id)) })
                .put(|State(st): State<AppState>, Path(id): Path<i64>, Json(b): Json<Value>| async move { answer(202, crate::tags::rename(&st.db, id, b["label"].as_str().unwrap_or(""))) })
                .delete(|State(st): State<AppState>, Path(id): Path<i64>| async move { answer(200, crate::tags::delete(&st.db, id).map(|_| json!({}))) }),
        )
        .route(
            "/rootfolder",
            get(move |State(st): State<AppState>| async move { roots(&st, app) }).post(move |State(st): State<AppState>, Json(b): Json<Value>| async move { root_add(&st, app, &b) }),
        )
        .route("/rootfolder/{id}", axum::routing::delete(move |State(st): State<AppState>, Path(id): Path<i64>| async move { root_delete(&st, app, id) }))
        .route(
            "/indexer",
            get(move |State(st): State<AppState>| async move { arr::indexers(&st.db, app.name).map(|v| Json(json!(v))) })
                .post(move |State(st): State<AppState>, Json(b): Json<Value>| async move { saved(201, arr::save_indexer(&st.db, app.name, None, b)) }),
        )
        .route("/indexer/schema", get(move || async move { Json(serde_json::from_str::<Value>(app.indexer_schema).unwrap_or_else(|_| json!([]))) }))
        .route("/indexer/test", post(|| async { Json(json!({})) }))
        .route("/indexer/testall", post(empty_list))
        .route(
            "/indexer/{id}",
            get(move |State(st): State<AppState>, Path(id): Path<i64>| async move { found(arr::indexer(&st.db, app.name, id)) })
                .put(move |State(st): State<AppState>, Path(id): Path<i64>, Json(b): Json<Value>| async move { saved(202, arr::save_indexer(&st.db, app.name, Some(id), b)) })
                .delete(move |State(st): State<AppState>, Path(id): Path<i64>| async move {
                    match arr::delete_indexer(&st.db, app.name, id) {
                        Ok(true) => Json(json!({})).into_response(),
                        Ok(false) => not_found(),
                        Err(e) => fail(e),
                    }
                }),
        )
        .route("/command", get(move |State(st): State<AppState>| async move { arr::commands(&st.db, app.name).map(|v| Json(json!(v))) }))
        .route("/command/{id}", get(move |State(st): State<AppState>, Path(id): Path<i64>| async move { found(arr::command(&st.db, app.name, id)) }))
}

/// `status` with the value, or the error.
fn answer(status: u16, r: Result<Value>) -> Response {
    match r {
        Ok(v) => with_status(status, v),
        Err(e) => fail(e),
    }
}

/// The value, 404 in the apps' shape when there is none, or the error.
fn found(r: Result<Option<Value>>) -> Response {
    match r {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// A saved indexer with `status`, a refusal in the apps' shape, or the error.
fn saved(status: u16, r: Result<std::result::Result<Value, Response>>) -> Response {
    match r {
        Ok(Ok(v)) => with_status(status, v),
        Ok(Err(refused)) => refused,
        Err(e) => fail(e),
    }
}

/// `GET queue/status`: how many downloads are on their way, and whether any has a problem.
fn queue_status(st: &AppState, app: &App) -> Json<Value> {
    let q = (app.queue)(st);
    let has = |s: &str| q.iter().any(|r| r["trackedDownloadStatus"] == s);
    Json(json!({ "totalCount": q.len(), "count": q.len(), "unknownCount": 0, "errors": has("error"), "warnings": has("warning"),
        "unknownErrors": false, "unknownWarnings": false }))
}

/// `PUT qualitydefinition/update`: several definitions at once.
async fn definitions_put(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    let mut out = Vec::new();
    for d in b.as_array().cloned().unwrap_or_default() {
        let id = d["id"].as_i64().unwrap_or(0);
        match crate::quality::update_definition(&st.db, id, d) {
            Ok(v) => out.push(v),
            Err(e) => return fail(e),
        }
    }
    with_status(202, json!(out))
}

/// `GET diskspace`: the disks of the app's root folders and the data folder.
fn diskspace(st: &AppState, app: &App) -> Result<Json<Value>> {
    let mut paths = Vec::new();
    for kind in app.root_kinds {
        paths.extend(crate::library::folders(&st.db, Some(kind))?.iter().map(|r| r.s("path")));
    }
    paths.push(st.data_dir.to_string_lossy().to_string());
    Ok(Json(json!(arr::disk_space(&paths))))
}

/// A root folder in the apps' shape.
fn root_view(r: &Row) -> Value {
    let path = r.s("path");
    let p = std::path::Path::new(&path);
    json!({ "path": path, "accessible": p.is_dir(), "freeSpace": crate::library::free_space(p), "unmappedFolders": [], "id": r.i("id") })
}

/// `GET rootfolder`: the app's root folders.
fn roots(st: &AppState, app: &App) -> Result<Json<Value>> {
    let mut out = Vec::new();
    for kind in app.root_kinds {
        out.extend(crate::library::folders(&st.db, Some(kind))?.iter().map(root_view));
    }
    Ok(Json(json!(out)))
}

/// `POST rootfolder`: add a root folder of the app's first kind (or give the one with that path).
fn root_add(st: &AppState, app: &App, b: &Value) -> Response {
    let raw = b["path"].as_str().unwrap_or("");
    let path = match crate::library::clean_path(raw) {
        Ok(p) => p,
        Err(_) => return validation("Path", "Invalid Path", json!(raw), "PathValidator"),
    };
    if !std::path::Path::new(&path).is_dir() {
        return validation("Path", "Folder does not exist", json!(path), "FolderValidator");
    }
    let r = (|| {
        if st.db.one("SELECT id FROM root_folders WHERE path=?", p![path])?.is_none() {
            st.db.execute(
                "INSERT INTO root_folders (kind, path, created_at) VALUES (?,?,?)",
                p![app.root_kinds[0], path, crate::auth::now()],
            )?;
        }
        st.db.one("SELECT * FROM root_folders WHERE path=?", p![path])
    })();
    match r {
        Ok(Some(row)) => with_status(201, root_view(&row)),
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// `DELETE rootfolder/{id}`: forget one of the app's root folders (its files stay).
fn root_delete(st: &AppState, app: &App, id: i64) -> Response {
    let r = st.db.one("SELECT kind FROM root_folders WHERE id=?", p![id]);
    match r {
        Ok(Some(row)) if app.root_kinds.contains(&row.s("kind").as_str()) => {
            match st.db.execute("DELETE FROM root_folders WHERE id=?", p![id]) {
                Ok(_) => Json(json!({})).into_response(),
                Err(e) => fail(e),
            }
        }
        Ok(_) => not_found(),
        Err(e) => fail(e),
    }
}
