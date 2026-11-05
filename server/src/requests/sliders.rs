//! Discover's sliders (the rows of the Seerr home page), ported from Seerr 3.5.1's `entity/DiscoverSlider.ts`,
//! `constants/discover.ts` and `routes/settings/discover.ts`: twelve built-in rows that can be turned off and
//! reordered, plus rows an admin adds (a TMDB keyword, genre, search, studio, network or streaming service).
//!
//! ```text
//! GET    /api/v1/settings/discover            every row, in order (anybody signed in)
//! POST   /api/v1/settings/discover            save the whole list: order and on/off (admins)
//! POST   /api/v1/settings/discover/add        add a row of your own, turned off, at the end (admins)
//! GET    /api/v1/settings/discover/reset      back to the built-in rows (admins)
//! PUT    /api/v1/settings/discover/{id}       change a row of your own (admins)
//! DELETE /api/v1/settings/discover/{id}       remove a row of your own (admins)
//! ```

use super::seerr::{perm, Answer, Fail, Seerr};
use super::stamp;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// Seerr's slider types (`DiscoverSliderType`).
pub mod kind {
    /// Recently added
    pub const RECENTLY_ADDED: i64 = 1;
    /// Recent requests
    pub const RECENT_REQUESTS: i64 = 2;
    /// Plex watchlist
    pub const PLEX_WATCHLIST: i64 = 3;
    /// Trending
    pub const TRENDING: i64 = 4;
    /// Popular movies
    pub const POPULAR_MOVIES: i64 = 5;
    /// Movie genres
    pub const MOVIE_GENRES: i64 = 6;
    /// Upcoming movies
    pub const UPCOMING_MOVIES: i64 = 7;
    /// Studios
    pub const STUDIOS: i64 = 8;
    /// Popular series
    pub const POPULAR_TV: i64 = 9;
    /// Series genres
    pub const TV_GENRES: i64 = 10;
    /// Upcoming series
    pub const UPCOMING_TV: i64 = 11;
    /// Networks
    pub const NETWORKS: i64 = 12;
}

/// The built-in rows, in Seerr's order (`defaultSliders`).
pub const DEFAULT: [i64; 12] = [
    kind::RECENTLY_ADDED,
    kind::RECENT_REQUESTS,
    kind::PLEX_WATCHLIST,
    kind::TRENDING,
    kind::POPULAR_MOVIES,
    kind::MOVIE_GENRES,
    kind::UPCOMING_MOVIES,
    kind::STUDIOS,
    kind::POPULAR_TV,
    kind::TV_GENRES,
    kind::UPCOMING_TV,
    kind::NETWORKS,
];

/// The sliders table, with the built-in rows put in when they are missing (Seerr's `bootstrapSliders`).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_sliders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            type INTEGER NOT NULL,
            sort INTEGER NOT NULL,
            built_in INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1,
            title TEXT,
            data TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The discover rows table can't be made: {}", e.detail))?;
    bootstrap(db).map_err(|e| anyhow::anyhow!("The built-in discover rows can't be added: {}", e.detail))
}

/// Adds each built-in row whose type isn't there yet.
pub fn bootstrap(db: &SqliteDb) -> kit_server_core::Result<()> {
    for (order, t) in DEFAULT.iter().enumerate() {
        if db.one("SELECT id FROM request_sliders WHERE type=?", p![*t])?.is_none() {
            let now = stamp();
            db.execute(
                "INSERT INTO request_sliders (type, sort, built_in, enabled, created_at, updated_at) VALUES (?,?,1,1,?,?)",
                p![*t, order as i64, now, now],
            )?;
        }
    }
    Ok(())
}

/// The routes.
pub fn router() -> Router<AppState> {
    let v1 = Router::new()
        .route("/settings/discover", get(list).post(save_all))
        .route("/settings/discover/add", post(add))
        .route("/settings/discover/reset", get(reset))
        .route("/settings/discover/{id}", put(change).delete(remove));
    Router::new().nest("/api/v1", v1)
}

/// A row in Seerr's shape.
fn slider_json(r: &Row) -> Value {
    let text = |k: &str| r.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
    json!({
        "id": r.i("id"),
        "type": r.i("type"),
        "order": r.i("sort"),
        "isBuiltIn": r.b("built_in"),
        "enabled": r.b("enabled"),
        "title": text("title"),
        "data": text("data"),
        "createdAt": r.s("created_at"),
        "updatedAt": r.s("updated_at"),
    })
}

/// One row, if it is there.
fn find(db: &SqliteDb, id: i64) -> kit_server_core::Result<Option<Row>> {
    db.one("SELECT * FROM request_sliders WHERE id=?", p![id])
}

/// A row as Seerr's page sends it.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Slider {
    /// the row's id (missing for a new one)
    pub id: Option<i64>,
    /// [`kind`] or one of Seerr's TMDB types (13–21)
    #[serde(rename = "type")]
    pub kind: Option<i64>,
    /// shown or not
    pub enabled: Option<bool>,
    /// the heading (rows of your own)
    pub title: Option<String>,
    /// what the row looks for, as Seerr writes it (a keyword id, a genre id, a search…)
    pub data: Option<String>,
}

/// `GET /settings/discover`: every row by its order.
async fn list(State(st): State<AppState>, _s: Seerr) -> Answer {
    let rows = st.db.query("SELECT * FROM request_sliders ORDER BY sort, id", p![])?;
    Ok(Json(json!(rows.iter().map(slider_json).collect::<Vec<_>>())).into_response())
}

/// `POST /settings/discover`: the whole list in its new order. Known rows take their place and on/off (and,
/// when they aren't built in, title, data and type); unknown ones are added. Answers what was sent, like Seerr.
async fn save_all(State(st): State<AppState>, s: Seerr, Json(body): Json<Value>) -> Answer {
    s.needs(&[perm::ADMIN])?;
    let Some(list) = body.as_array() else {
        return Err(Fail(err(400, "Invalid request body.")));
    };
    for (x, v) in list.iter().enumerate() {
        let sl: Slider = serde_json::from_value(v.clone()).unwrap_or_default();
        let now = stamp();
        match sl.id.map(|id| find(&st.db, id)).transpose()?.flatten() {
            Some(row) => {
                let built_in = row.b("built_in");
                st.db.execute(
                    "UPDATE request_sliders SET enabled=?, sort=?, title=?, data=?, type=?, updated_at=? WHERE id=?",
                    p![
                        sl.enabled.unwrap_or(row.b("enabled")) as i64,
                        x as i64,
                        if built_in { row.get("title").and_then(Value::as_str).map(str::to_string) } else { sl.title },
                        if built_in { row.get("data").and_then(Value::as_str).map(str::to_string) } else { sl.data },
                        if built_in { row.i("type") } else { sl.kind.unwrap_or(row.i("type")) },
                        now,
                        row.i("id")
                    ],
                )?;
            }
            None => {
                st.db.execute(
                    "INSERT INTO request_sliders (type, sort, built_in, enabled, title, data, created_at, updated_at)
                        VALUES (?,?,0,?,?,?,?,?)",
                    p![sl.kind.unwrap_or(0), x as i64, sl.enabled.unwrap_or(true) as i64, sl.title, sl.data, now, now],
                )?;
            }
        }
    }
    Ok(Json(body).into_response())
}

/// `POST /settings/discover/add`: a row of your own, turned off, before the others (order -1, like Seerr).
async fn add(State(st): State<AppState>, s: Seerr, Json(sl): Json<Slider>) -> Answer {
    s.needs(&[perm::ADMIN])?;
    let now = stamp();
    let id = st.db.execute(
        "INSERT INTO request_sliders (type, sort, built_in, enabled, title, data, created_at, updated_at) VALUES (?,-1,0,0,?,?,?,?)",
        p![sl.kind.unwrap_or(0), sl.title, sl.data, now, now],
    )?;
    let row = find(&st.db, id)?.ok_or_else(|| err(500, "The new row can't be read back"))?;
    Ok(Json(slider_json(&row)).into_response())
}

/// `GET /settings/discover/reset`: every row goes and the built-in ones come back.
async fn reset(State(st): State<AppState>, s: Seerr) -> Answer {
    s.needs(&[perm::ADMIN])?;
    st.db.execute("DELETE FROM request_sliders", p![])?;
    bootstrap(&st.db)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `PUT /settings/discover/{id}`: change a row's title, data and type (built-in rows stay as they are).
async fn change(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(sl): Json<Slider>) -> Answer {
    s.needs(&[perm::ADMIN])?;
    let row = find(&st.db, id)?.ok_or_else(|| err(404, "Slider not found or cannot be updated."))?;
    if !row.b("built_in") {
        st.db.execute(
            "UPDATE request_sliders SET title=?, data=?, type=?, updated_at=? WHERE id=?",
            p![sl.title, sl.data, sl.kind.unwrap_or(row.i("type")), stamp(), id],
        )?;
    }
    let row = find(&st.db, id)?.ok_or_else(|| err(404, "Slider not found or cannot be updated."))?;
    Ok(Json(slider_json(&row)).into_response())
}

/// `DELETE /settings/discover/{id}`: remove a row of your own; built-in rows can only be turned off.
async fn remove(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::ADMIN])?;
    match find(&st.db, id)? {
        Some(r) if !r.b("built_in") => {
            st.db.execute("DELETE FROM request_sliders WHERE id=?", p![id])?;
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        _ => Err(Fail(err(404, "Slider not found or cannot be deleted."))),
    }
}
