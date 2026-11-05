//! Anime as its own kind: the community mapping list (AniDB/AniList/MAL ↔ TheTVDB/TMDB), AniList's details for
//! request pages (seasons as separate entries, airing status), each person's language choice (dub, dual audio or
//! subtitles), and the Review queue for downloads whose real tracks don't match what was asked for.
//!
//! ```text
//! GET  /api/anime/prefs                 → my language choice {audio, dub_language, subtitles, words} (signed in)
//! PUT  /api/anime/prefs                 {audio, subtitles?} → it (signed in; the same choice as the request pages)
//! GET  /api/anime/lookup?term=…         → AniList's matches with TheTVDB/TMDB ids and season (signed in)
//! GET  /api/anime/{anilist}/seasons     → the show's seasons as separate entries, each mapped (signed in)
//! GET  /api/anime/review                → downloads waiting for a look, with the reason (admins)
//! POST /api/anime/review/{id}/keep      → imports the file as it is (admins)
//! POST /api/anime/review/{id}/reject    → turns the release down for good; the episodes are searched again (admins)
//! POST /api/anime/mapping/refresh       → {entries} fetches the mapping list now (admins)
//! GET  /api/anime/discover?list=trending|season|upcoming|top&genre=&studio=&page=
//!                                       → {results: [entry mapped, with media_type and media_status], more} (signed in)
//! GET  /api/anime/genres                → AniList's genres, for Discover's filter (signed in)
//! ```
//!
//! The Anime tab's library, calendar and downloads are in [`library`].

pub mod anilist;
pub mod language;
pub mod library;
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
    library::migrate(db)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS anime_review (
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
    .map_err(|e| anyhow::anyhow!("The anime tables can't be made: {}", e.detail))?;
    move_old_prefs(db).map_err(|e| anyhow::anyhow!("The old anime language choices can't be moved: {}", e.detail))
}

/// Moves the language choices of the first anime release (table `anime_prefs`) into the requests' table
/// (`request_prefs`), where the request pages keep them, and drops the old table. Nothing is lost: a choice made
/// on the request pages wins over the old one.
fn move_old_prefs(db: &SqliteDb) -> Result<()> {
    if db.one("SELECT name FROM sqlite_master WHERE type='table' AND name='anime_prefs'", p![])?.is_none() {
        return Ok(());
    }
    for r in db.query("SELECT * FROM anime_prefs", p![])? {
        db.execute(
            "INSERT OR IGNORE INTO request_prefs (user_id, anime_audio, anime_subtitles) VALUES (?,?,?)",
            p![r.i("user_id"), r.s("audio"), language::two_letter(&r.s("subtitles"))],
        )?;
    }
    db.execute_batch("DROP TABLE anime_prefs")
}

/// A choice from the requests' words: audio `dub`, `dual` or `sub`, and the subtitles' language. Dubs are in
/// English for now (requests don't ask for the dub's language).
fn choice_of(audio: &str, subtitles: &str) -> Choice {
    Choice { audio: Audio::parse(audio).unwrap_or(Audio::Dual), dub_language: "eng".into(), subtitles: language::normalize(subtitles) }
}

/// Someone's default language choice, kept with their requests (dual audio with English subtitles when they made
/// none, like the request pages).
pub fn choice_for(db: &SqliteDb, user_id: i64) -> Choice {
    db.one("SELECT anime_audio, anime_subtitles FROM request_prefs WHERE user_id=?", p![user_id])
        .ok()
        .flatten()
        .map(|r| choice_of(&r.s("anime_audio"), &r.s("anime_subtitles")))
        .unwrap_or_else(|| choice_of("dual", "en"))
}

/// The language the newest request for a series (by its TMDB id) asked for; `None` when nobody requested it with
/// a language choice, so its files aren't checked.
pub fn choice_for_series(db: &SqliteDb, tmdb_id: i64) -> Option<Choice> {
    db.one(
        "SELECT r.anime_audio, r.anime_subtitles FROM requests r JOIN request_media m ON m.id=r.media_id
         WHERE m.kind='tv' AND m.tmdb_id=? AND r.anime_audio!='' ORDER BY r.id DESC LIMIT 1",
        p![tmdb_id],
    )
    .ok()
    .flatten()
    .map(|r| choice_of(&r.s("anime_audio"), &r.s("anime_subtitles")))
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
    /// the subtitles' language (ISO 639: `en` or `eng`); unchanged when left out
    pub subtitles: Option<String>,
}

/// `GET /api/anime/prefs`.
async fn prefs_get(State(st): State<AppState>, me: Me) -> Json<Value> {
    Json(choice_json(&choice_for(&st.db, me.account.id)))
}

/// `PUT /api/anime/prefs`: the same choice the request pages set.
async fn prefs_put(State(st): State<AppState>, me: Me, Json(b): Json<Prefs>) -> Result<Json<Value>> {
    let audio = Audio::parse(&b.audio).ok_or_else(|| err(400, "Pick Dub only, Dual audio or Sub only"))?;
    let old = choice_for(&st.db, me.account.id);
    let subs = language::two_letter(&b.subtitles.unwrap_or(old.subtitles));
    crate::requests::set_anime_prefs(&st.db, me.account.id, audio.word(), &subs)?;
    Ok(Json(choice_json(&choice_for(&st.db, me.account.id))))
}

/// An AniList entry with its mapping and, when its series (or film) is added, Marquee's id for it. `media_type`
/// is what a request for it asks for (`movie` for films, else `tv`), and `media_status` where that request stands
/// (Seerr's numbers: 2 requested, 3 approved, 4 partly there, 5 available; null when nobody asked).
fn mapped(st: &AppState, m: &anilist::Media) -> Value {
    let e = mapping::by_anilist(&st.db, m.id).ok().flatten();
    let added = e.as_ref().and_then(|e| e.tvdb).and_then(|t| crate::series::by_tvdb(&st.db, t).ok().flatten()).map(|s| s.id);
    let film = m.format == "MOVIE";
    let tmdb = e.as_ref().and_then(|e| if film { e.tmdb_movie } else { e.tmdb_tv });
    let movie_id =
        e.as_ref().and_then(|e| e.tmdb_movie).filter(|_| film).and_then(|t| crate::movies::by_tmdb(&st.db, t).ok().flatten()).map(|m| m.id);
    let kind = if film { "movie" } else { "tv" };
    let status = tmdb
        .and_then(|t| crate::requests::discover::media_info(&st.db, kind, t).ok().flatten())
        .and_then(|v| v["status"].as_i64())
        .filter(|s| *s > 1);
    json!({
        "media_type": kind, "media_status": status, "movie_id": movie_id,
        "genres": m.genres, "studios": m.studios, "score": m.score,
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

/// `GET /api/anime/discover` query.
#[derive(Deserialize)]
pub struct DiscoverQuery {
    /// `trending` (default), `season`, `upcoming` or `top`
    pub list: Option<String>,
    /// one of AniList's genres
    pub genre: Option<String>,
    /// a studio's name
    pub studio: Option<String>,
    /// the page, from 1
    pub page: Option<i64>,
}

/// `GET /api/anime/discover`.
async fn discover(State(st): State<AppState>, _me: Me, Query(q): Query<DiscoverQuery>) -> Result<Json<Value>> {
    let list = anilist::List::parse(q.list.as_deref().unwrap_or(""))
        .ok_or_else(|| err(400, "Pick trending, this season, upcoming or top rated"))?;
    let now = chrono::Utc::now();
    let today = (chrono::Datelike::month(&now), chrono::Datelike::year(&now));
    let (found, more) = anilist::AniList::from(&st)
        .browse(list, q.genre.as_deref().unwrap_or(""), q.studio.as_deref().unwrap_or(""), q.page.unwrap_or(1), today)
        .await?;
    Ok(Json(json!({ "results": found.iter().map(|m| mapped(&st, m)).collect::<Vec<_>>(), "more": more })))
}

/// `GET /api/anime/genres`.
async fn genres(State(st): State<AppState>, _me: Me) -> Result<Json<Value>> {
    Ok(Json(json!(anilist::AniList::from(&st).genres().await?)))
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
    let download = crate::downloads::find(&st.db, r.i("download_id"))?;
    match (state, download) {
        ("kept", Some(d)) => crate::downloads::import_kept(&st, d.id).await?,
        ("rejected", Some(d)) => crate::downloads::reject(&st, &d, &format!("rejected in the Review queue ({})", r.s("reason"))).await?,
        ("rejected", None) => {
            if let Some(sid) = r.get("series_id").and_then(Value::as_i64) {
                crate::downloads::spawn_search(&st, crate::downloads::Search::Series(sid, None));
            }
        }
        _ => {}
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
        .route("/api/anime/discover", get(discover))
        .route("/api/anime/genres", get(genres))
        .merge(library::router())
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
