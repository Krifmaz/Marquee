//! Anime as its own kind: the community mapping list (AniDB/AniList/MAL ↔ TheTVDB/TMDB), AniList's details for
//! request pages (seasons as separate entries, airing status), each person's language choice (dub, dual audio or
//! subtitles), and the Review queue for downloads whose real tracks don't match what was asked for.
//!
//! ```text
//! GET  /api/anime/prefs                 → my language choice {audio, dub_language, subtitles, words} (signed in)
//! PUT  /api/anime/prefs                 {audio, dub_language?, subtitles?} → it (signed in)
//! GET  /api/anime/lookup?term=…         → AniList's matches with TheTVDB/TMDB ids and season (signed in)
//! GET  /api/anime/{anilist}/seasons     → the show's seasons as separate entries, each mapped (signed in)
//! GET  /api/anime/review                → downloads waiting for a look, with the reason (admins)
//! POST /api/anime/review/{id}/keep      → keeps the file as it is (admins)
//! POST /api/anime/review/{id}/reject    → turns it down; the episodes are searched again (admins)
//! POST /api/anime/mapping/refresh       → {entries} fetches the mapping list now (admins)
//! ```

pub mod anilist;
pub mod language;
pub mod mapping;

use crate::roles::Me;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use language::{Audio, Choice};
use serde::Deserialize;
use serde_json::{json, Value};

/// The anime tables: mapping, language choices, the Review queue.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    mapping::migrate(db)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS anime_prefs (
            user_id INTEGER PRIMARY KEY,
            audio TEXT NOT NULL DEFAULT 'sub',
            dub_language TEXT NOT NULL DEFAULT 'eng',
            subtitles TEXT NOT NULL DEFAULT 'eng'
        );
        CREATE TABLE IF NOT EXISTS anime_review (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            download_id INTEGER NOT NULL,
            series_id INTEGER,
            title TEXT NOT NULL,
            file TEXT NOT NULL,
            reason TEXT NOT NULL,
            wanted TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'waiting',
            added TEXT NOT NULL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The anime tables can't be made: {}", e.detail))
}

/// Someone's language choice (the default when they made none).
pub fn choice_for(db: &SqliteDb, user_id: i64) -> Choice {
    db.one("SELECT * FROM anime_prefs WHERE user_id=?", p![user_id])
        .ok()
        .flatten()
        .map(|r| Choice {
            audio: Audio::parse(&r.s("audio")).unwrap_or_default(),
            dub_language: r.s("dub_language"),
            subtitles: r.s("subtitles"),
        })
        .unwrap_or_default()
}

/// A choice for the web app and the requests API.
pub fn choice_json(c: &Choice) -> Value {
    json!({ "audio": c.audio, "dub_language": c.dub_language, "subtitles": c.subtitles, "words": c.words() })
}

/// Checks a downloaded file against a choice: `None` when it gives what was asked for, else why not.
pub fn verify(file: &std::path::Path, c: &Choice) -> Option<String> {
    match language::probe(file) {
        Ok(t) => language::check(c, &t),
        Err(e) => {
            tracing::warn!("{e}");
            None
        }
    }
}

/// What happens to an anime download whose tracks don't match: `research` (turn it down and search again) or
/// `review` (keep it in the Review queue for a person).
pub fn on_mismatch(st: &AppState) -> String {
    let v = st.settings.str("anime_mismatch");
    if v == "review" {
        v
    } else {
        "research".into()
    }
}

/// Puts a download in the Review queue with its reason; its id.
pub fn send_to_review(
    db: &SqliteDb,
    download_id: i64,
    series_id: Option<i64>,
    title: &str,
    file: &str,
    reason: &str,
    wanted: &Choice,
) -> Result<i64> {
    db.execute(
        "INSERT INTO anime_review (download_id, series_id, title, file, reason, wanted, added) VALUES (?,?,?,?,?,?,?)",
        p![download_id, series_id, title, file, reason, wanted.words(), crate::movies::now_iso()],
    )
}

/// `PUT /api/anime/prefs` body.
#[derive(Deserialize)]
pub struct Prefs {
    /// `dub`, `dual` or `sub`
    pub audio: String,
    /// the dub's language (ISO 639)
    pub dub_language: Option<String>,
    /// the subtitles' language, "" for none
    pub subtitles: Option<String>,
}

/// `GET /api/anime/prefs`.
async fn prefs_get(State(st): State<AppState>, me: Me) -> Json<Value> {
    Json(choice_json(&choice_for(&st.db, me.account.id)))
}

/// `PUT /api/anime/prefs`.
async fn prefs_put(State(st): State<AppState>, me: Me, Json(b): Json<Prefs>) -> Result<Json<Value>> {
    let audio = Audio::parse(&b.audio).ok_or_else(|| err(400, "Pick Dub only, Dual audio or Sub only"))?;
    let old = choice_for(&st.db, me.account.id);
    let c = Choice {
        audio,
        dub_language: b.dub_language.map(|l| language::normalize(&l)).unwrap_or(old.dub_language),
        subtitles: b.subtitles.map(|l| if l.trim().is_empty() { String::new() } else { language::normalize(&l) }).unwrap_or(old.subtitles),
    };
    let audio_word = serde_json::to_value(c.audio).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    st.db.execute(
        "INSERT INTO anime_prefs (user_id, audio, dub_language, subtitles) VALUES (?,?,?,?)
         ON CONFLICT(user_id) DO UPDATE SET audio=excluded.audio, dub_language=excluded.dub_language, subtitles=excluded.subtitles",
        p![me.account.id, audio_word, c.dub_language, c.subtitles],
    )?;
    Ok(Json(choice_json(&c)))
}

/// An AniList entry with its mapping and, when its series is added, Marquee's series id.
fn mapped(st: &AppState, m: &anilist::Media) -> Value {
    let e = mapping::by_anilist(&st.db, m.id).ok().flatten();
    let added = e.as_ref().and_then(|e| e.tvdb).and_then(|t| crate::series::by_tvdb(&st.db, t).ok().flatten()).map(|s| s.id);
    json!({
        "anilist_id": m.id, "mal_id": m.mal, "title": m.title, "romaji": m.romaji, "format": m.format, "status": m.status,
        "episodes": m.episodes, "season": m.season, "year": m.year, "poster": m.cover, "overview": m.overview,
        "next_episode": m.next_episode.map(|(n, at)| json!({ "episode": n, "airs": at })),
        "tvdb_id": e.as_ref().and_then(|e| e.tvdb), "tmdb_id": e.as_ref().and_then(|e| e.tmdb_tv.or(e.tmdb_movie)),
        "tvdb_season": e.as_ref().and_then(|e| e.season), "episode_offset": e.as_ref().map(|e| e.offset),
        "series_id": added,
    })
}

/// `GET /api/anime/lookup` query.
#[derive(Deserialize)]
pub struct Lookup {
    /// title words
    pub term: String,
}

/// `GET /api/anime/lookup`.
async fn lookup(State(st): State<AppState>, _me: Me, Query(q): Query<Lookup>) -> Result<Json<Value>> {
    if q.term.trim().is_empty() {
        return Ok(Json(json!([])));
    }
    let found = anilist::AniList::from(&st).search(q.term.trim()).await?;
    Ok(Json(json!(found.iter().map(|m| mapped(&st, m)).collect::<Vec<_>>())))
}

/// `GET /api/anime/{anilist}/seasons`.
async fn seasons(State(st): State<AppState>, _me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    let list = anilist::AniList::from(&st).seasons(id).await?;
    Ok(Json(json!(list.iter().map(|m| mapped(&st, m)).collect::<Vec<_>>())))
}

/// `GET /api/anime/review`.
async fn review_list(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    let rows = st.db.query("SELECT * FROM anime_review WHERE state='waiting' ORDER BY id DESC", p![])?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({ "id": r.i("id"), "download_id": r.i("download_id"), "title": r.s("title"), "file": r.s("file"),
            "reason": r.s("reason"), "wanted": r.s("wanted"), "added": r.s("added") }))
        .collect::<Vec<_>>())))
}

/// `POST /api/anime/review/{id}/keep` and `/reject`.
async fn review_decide(State(st): State<AppState>, me: Me, Path((id, what)): Path<(i64, String)>) -> Result<Json<Value>> {
    me.admin()?;
    let r = st
        .db
        .one("SELECT * FROM anime_review WHERE id=? AND state='waiting'", p![id])?
        .ok_or_else(|| err(404, "That isn't waiting for a look any more"))?;
    let state = match what.as_str() {
        "keep" => "kept",
        "reject" => "rejected",
        _ => return Err(err(404, "Say keep or reject")),
    };
    st.db.execute("UPDATE anime_review SET state=? WHERE id=?", p![state, id])?;
    if state == "rejected" {
        if let Some(sid) = r.get("series_id").and_then(Value::as_i64) {
            crate::downloads::spawn_search(&st, crate::downloads::Search::Series(sid, None));
        }
    }
    Ok(Json(json!({ "ok": true, "state": state })))
}

/// Fetches the mapping list and replaces the table; how many entries it holds now.
pub async fn refresh_mapping(st: &AppState) -> Result<usize> {
    let url = st.settings.str("anime_list_url");
    let url = if url.trim().is_empty() { mapping::LIST_URL.to_string() } else { url };
    let r = st
        .http
        .get(&url)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| err(502, "The anime mapping list can't be fetched right now"))?;
    if !r.status().is_success() {
        return Err(err(502, format!("The anime mapping list answered {}", r.status().as_u16())));
    }
    let v: Value = r.json().await.map_err(|_| err(502, "The anime mapping list can't be read"))?;
    let n = mapping::load(&st.db, &v)?;
    tracing::info!("The anime mapping list has {n} entries");
    Ok(n)
}

/// `POST /api/anime/mapping/refresh`.
async fn mapping_refresh(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    Ok(Json(json!({ "entries": refresh_mapping(&st).await? })))
}

/// The anime routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/anime/prefs", get(prefs_get).put(prefs_put))
        .route("/api/anime/lookup", get(lookup))
        .route("/api/anime/{id}/seasons", get(seasons))
        .route("/api/anime/review", get(review_list))
        .route("/api/anime/review/{id}/{what}", post(review_decide))
        .route("/api/anime/mapping/refresh", post(mapping_refresh))
}

/// Fetches the mapping list soon after start when it is empty, then once a day.
pub fn spawn(st: AppState) {
    if st.settings.str("anime_list_url") == "off" {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        loop {
            if let Err(e) = refresh_mapping(&st).await {
                tracing::warn!("{}", e.detail);
            }
            tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
        }
    });
}
