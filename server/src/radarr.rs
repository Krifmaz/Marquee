//! The Radarr v3 API, so tools made for Radarr (Prowlarr, Seerr, Homepage, nzb360, LunaSea) work with Marquee
//! unchanged. Base URL `http://marquee:8080/radarr`, Marquee's API key (`X-Api-Key` or `?apikey=`).
//!
//! | Path under `/radarr/api/v3` | What |
//! | --- | --- |
//! | `system/status`, `health`, `diskspace` | the server |
//! | `movie`, `movie/{id}`, `movie/lookup`, `movie/lookup/tmdb`, `movie/lookup/imdb`, `movie/editor` | movies |
//! | `calendar`, `wanted/missing`, `wanted/cutoff` | what is coming and what is missing |
//! | `queue`, `queue/status`, `queue/details`, `history`, `history/movie` | downloads (filled by the downloads step) |
//! | `qualityprofile`, `qualitydefinition`, `customformat`, `language`, `tag`, `rootfolder` | setup |
//! | `indexer`, `indexer/schema`, `indexer/test` | indexers Prowlarr pushes (kept for Lantern) |
//! | `command` | Refresh and search commands |
//! | `config/naming`, `config/ui`, `notification`, `downloadclient`, `importlist`, `exclusions` | settings tools read |
//!
//! Answers follow Radarr 6's shapes; `tests/contracts/radarr/` holds real Radarr answers they are checked
//! against.

use crate::arr::{self, not_found, paged, validation, with_status, Paging};
use crate::arr_common::{fail, App};
use crate::movies::{self, iso_day, poster_url, AddOptions, Movie};
use crate::tmdb::MovieInfo;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use kit_server_core::Result;
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// The Radarr version Marquee answers as: clients check it is new enough (Prowlarr wants v3 or later).
pub const COMPAT_VERSION: &str = "6.4.4.10685";

/// The app name in commands and indexers.
const APP: &str = "radarr";

/// What the shared routes need to know about Radarr.
pub static APP_INFO: App = App {
    name: APP,
    profile_kind: "movie",
    root_kinds: &["movie"],
    languages: arr::LANGUAGES,
    indexer_schema: include_str!("../data/radarr/indexer_schema.json"),
    queue: crate::downloads::radarr_queue,
};

/// The routes, nested under `/radarr`.
pub fn router(st: AppState) -> Router<AppState> {
    let v3 = Router::new()
        .route("/system/status", get(status))
        .route("/movie", get(movie_list).post(movie_add))
        .route("/movie/editor", put(movie_editor))
        .route("/movie/lookup", get(lookup_term))
        .route("/movie/lookup/tmdb", get(lookup_tmdb))
        .route("/movie/lookup/imdb", get(lookup_imdb))
        .route("/movie/{id}", get(movie_one).put(movie_put).delete(movie_delete))
        .route("/calendar", get(calendar))
        .route("/wanted/missing", get(wanted_missing))
        .route(
            "/wanted/cutoff",
            get(|Query(q): Query<Paging>| async move { Json(paged(&q, "movieMetadata.sortTitle", "ascending", vec![])) }),
        )
        .route("/history/movie", get(|| async { Json(json!([])) }))
        .route("/command", post(command_add))
        .route("/config/naming", get(naming))
        .route("/config/ui", get(ui))
        .route("/exclusions", get(|| async { Json(json!([])) }))
        .merge(crate::localization::routes(crate::localization::App::Radarr))
        .merge(crate::custom_filters::routes("radarr"))
        .route("/indexerflag", get(|| async { Json(crate::arr_common::indexer_flags("radarr")) }))
        .route("/collection", get(|| async { Json(json!([])) }))
        .merge(crate::arr_common::routes(&APP_INFO));
    Router::new()
        .nest("/radarr/api/v3", v3)
        .route("/radarr/api", get(arr::api_versions))
        .route("/radarr/ping", get(arr::ping))
        .layer(axum::middleware::from_fn_with_state(st, arr::require_key))
}

/// `GET system/status`.
async fn status(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "appName": "Radarr",
        "instanceName": st.app_name(),
        "version": COMPAT_VERSION,
        "buildTime": "2026-09-14T15:54:38Z",
        "isDebug": false,
        "isProduction": true,
        "isAdmin": false,
        "isUserInteractive": false,
        "startupPath": "/app",
        "appData": st.data_dir.to_string_lossy(),
        "osName": std::env::consts::OS,
        "osVersion": "",
        "isNetCore": true,
        "isLinux": cfg!(target_os = "linux"),
        "isOsx": cfg!(target_os = "macos"),
        "isWindows": cfg!(target_os = "windows"),
        "isDocker": std::path::Path::new("/.dockerenv").exists(),
        "isContainerized": true,
        "mode": "console",
        "branch": "master",
        "databaseType": "sqLite",
        "databaseVersion": "3",
        "authentication": "external",
        "migrationVersion": 242,
        "urlBase": "/radarr",
        "runtimeVersion": "rust",
        "runtimeName": "marquee",
        "startTime": st.started,
        "packageVersion": crate::VERSION,
        "packageAuthor": "Marquee",
        "packageUpdateMechanism": "docker",
    }))
}

/// A Radarr `images` list from TMDB paths.
fn images(info: &MovieInfo) -> Value {
    let mut out = Vec::new();
    for (kind, path) in [("poster", &info.poster), ("fanart", &info.fanart)] {
        if !path.is_empty() {
            let url = poster_url(path);
            out.push(json!({ "coverType": kind, "url": url, "remoteUrl": url }));
        }
    }
    json!(out)
}

/// The fields every Radarr movie has, from its details (a lookup result has only these and a few more).
fn info_fields(info: &MovieInfo) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("title".into(), json!(info.title));
    o.insert("originalTitle".into(), json!(if info.original_title.is_empty() { &info.title } else { &info.original_title }));
    o.insert("originalLanguage".into(), arr::language(&info.original_language));
    o.insert("alternateTitles".into(), json!([]));
    o.insert("secondaryYearSourceId".into(), json!(0));
    o.insert("sortTitle".into(), json!(movies::sort_title(&info.title)));
    o.insert("sizeOnDisk".into(), json!(0));
    o.insert("overview".into(), json!(info.overview));
    for (k, v) in [("inCinemas", &info.in_cinemas), ("physicalRelease", &info.physical_release), ("digitalRelease", &info.digital_release)]
    {
        if let Some(d) = v {
            o.insert(k.into(), json!(iso_day(d)));
        }
    }
    if let Some(d) = [&info.physical_release, &info.digital_release].into_iter().flatten().min() {
        o.insert("releaseDate".into(), json!(iso_day(d)));
    }
    o.insert("images".into(), images(info));
    if !info.website.is_empty() {
        o.insert("website".into(), json!(info.website));
    }
    o.insert("year".into(), json!(info.year));
    if !info.youtube_trailer.is_empty() {
        o.insert("youTubeTrailerId".into(), json!(info.youtube_trailer));
    }
    o.insert("studio".into(), json!(info.studio));
    o.insert("runtime".into(), json!(info.runtime));
    o.insert("cleanTitle".into(), json!(movies::clean_title(&info.title)));
    if !info.imdb_id.is_empty() {
        o.insert("imdbId".into(), json!(info.imdb_id));
    }
    o.insert("tmdbId".into(), json!(info.tmdb_id));
    o.insert("titleSlug".into(), json!(info.tmdb_id.to_string()));
    o.insert("certification".into(), json!(info.certification));
    o.insert("genres".into(), json!(info.genres));
    o.insert("keywords".into(), json!([]));
    let mut ratings = Map::new();
    if info.votes > 0 {
        ratings.insert("tmdb".into(), json!({ "votes": info.votes, "value": info.rating, "type": "user" }));
    }
    o.insert("ratings".into(), Value::Object(ratings));
    if let Some((id, title)) = &info.collection {
        o.insert("collection".into(), json!({ "title": title, "tmdbId": id }));
    }
    o.insert("popularity".into(), json!(info.popularity));
    o
}

/// A movie in Radarr's shape.
pub fn resource(m: &Movie, now: DateTime<Utc>) -> Value {
    let mut o = info_fields(&m.info);
    o.insert("status".into(), json!(m.status(now)));
    o.insert("path".into(), json!(m.path));
    o.insert("qualityProfileId".into(), json!(m.quality_profile_id));
    o.insert("hasFile".into(), json!(m.has_file()));
    o.insert("movieFileId".into(), json!(m.movie_file_id));
    o.insert("monitored".into(), json!(m.monitored));
    o.insert("minimumAvailability".into(), json!(m.minimum_availability));
    o.insert("isAvailable".into(), json!(m.is_available(now)));
    o.insert("folderName".into(), json!(m.path));
    o.insert("rootFolderPath".into(), json!(m.root_folder));
    o.insert("tags".into(), json!(m.tags));
    o.insert("added".into(), json!(m.added));
    o.insert(
        "addOptions".into(),
        json!({ "searchForMovie": false, "addMethod": "manual", "ignoreEpisodesWithFiles": false, "ignoreEpisodesWithoutFiles": false, "monitor": "movieOnly" }),
    );
    o.insert("statistics".into(), json!({ "movieFileCount": 0, "sizeOnDisk": 0, "releaseGroups": [], "movieFileQualities": [] }));
    o.insert("id".into(), json!(m.id));
    Value::Object(o)
}

/// A lookup result in Radarr's shape: the added movie when it is added, else TMDB's details with a suggested
/// folder.
fn lookup_resource(st: &AppState, info: &MovieInfo, now: DateTime<Utc>) -> Result<Value> {
    if let Some(m) = movies::by_tmdb(&st.db, info.tmdb_id)? {
        return Ok(resource(&m, now));
    }
    let mut o = info_fields(info);
    let folder = movies::folder_name(&info.title, info.year);
    o.insert("status".into(), json!(if info.in_cinemas.is_some() { "released" } else { "announced" }));
    o.insert("folder".into(), json!(folder));
    o.insert("qualityProfileId".into(), json!(0));
    o.insert("hasFile".into(), json!(false));
    o.insert("movieFileId".into(), json!(0));
    o.insert("monitored".into(), json!(false));
    o.insert("minimumAvailability".into(), json!("tba"));
    o.insert("isAvailable".into(), json!(true));
    o.insert("tags".into(), json!([]));
    o.insert("added".into(), json!("0001-01-01T00:00:00Z"));
    o.insert("statistics".into(), json!({ "movieFileCount": 0, "sizeOnDisk": 0, "releaseGroups": [], "movieFileQualities": [] }));
    Ok(Value::Object(o))
}

/// `GET movie` query.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MovieQuery {
    /// only the movie with this TMDB id
    tmdb_id: Option<i64>,
}

/// `GET movie` (`?tmdbId=` narrows it).
async fn movie_list(State(st): State<AppState>, Query(q): Query<MovieQuery>) -> Result<Json<Value>> {
    let now = Utc::now();
    let list: Vec<Value> =
        movies::all(&st.db)?.iter().filter(|m| q.tmdb_id.is_none_or(|t| m.info.tmdb_id == t)).map(|m| resource(m, now)).collect();
    Ok(Json(json!(list)))
}

/// `GET movie/{id}`.
async fn movie_one(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match movies::find(&st.db, id) {
        Ok(Some(m)) => Json(resource(&m, Utc::now())).into_response(),
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// A movie's details for an add: TMDB's, or what the caller sent when TMDB can't be asked (no key, or
/// unreachable), so an add still works.
pub async fn details_for_add(st: &AppState, tmdb_id: i64, body: &Value) -> Result<MovieInfo> {
    match movies::tmdb(st).movie(tmdb_id).await {
        Ok(info) => Ok(info),
        Err(e) if e.status.as_u16() == 404 => Err(e),
        Err(e) => {
            let title = body["title"].as_str().unwrap_or("").to_string();
            if title.is_empty() {
                return Err(e);
            }
            tracing::warn!("TMDB couldn't be asked about {tmdb_id} ({}); adding it with the details sent", e.detail);
            let image = |kind: &str| {
                body["images"]
                    .as_array()
                    .and_then(|a| a.iter().find(|i| i["coverType"] == kind))
                    .and_then(|i| i["remoteUrl"].as_str())
                    .map(|u| u.trim_start_matches(crate::tmdb::IMAGE_BASE).to_string())
                    .unwrap_or_default()
            };
            Ok(MovieInfo {
                tmdb_id,
                imdb_id: body["imdbId"].as_str().unwrap_or("").into(),
                original_title: body["originalTitle"].as_str().unwrap_or(&title).into(),
                title,
                overview: body["overview"].as_str().unwrap_or("").into(),
                year: body["year"].as_i64().unwrap_or(0),
                runtime: body["runtime"].as_i64().unwrap_or(0),
                in_cinemas: body["inCinemas"].as_str().map(|d| d.chars().take(10).collect()),
                physical_release: body["physicalRelease"].as_str().map(|d| d.chars().take(10).collect()),
                digital_release: body["digitalRelease"].as_str().map(|d| d.chars().take(10).collect()),
                poster: image("poster"),
                fanart: image("fanart"),
                ..MovieInfo::default()
            })
        }
    }
}

/// The ids in a `tags` field.
fn tag_ids(v: &Value) -> Vec<i64> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()
}

/// `POST movie`: add a movie (201 with it), Radarr's checks and their error shapes.
async fn movie_add(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    let Some(tmdb_id) = b["tmdbId"].as_i64().filter(|i| *i > 0) else {
        return validation("TmdbId", "'Tmdb Id' must be greater than '0'.", b["tmdbId"].clone(), "GreaterThanValidator");
    };
    match movies::by_tmdb(&st.db, tmdb_id) {
        Ok(Some(_)) => return validation("TmdbId", "This movie has already been added", json!(tmdb_id), "MovieExistsValidator"),
        Ok(None) => {}
        Err(e) => return fail(e),
    }
    let profile = b["qualityProfileId"].as_i64().unwrap_or(0);
    if !matches!(crate::quality::profile(&st.db, profile), Ok(Some(_))) {
        return validation("QualityProfileId", "Quality Profile does not exist", json!(profile), "QualityProfileExistsValidator");
    }
    let path = b["path"].as_str().filter(|p| !p.is_empty()).map(str::to_string);
    let root = b["rootFolderPath"]
        .as_str()
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| path.as_deref().and_then(|p| std::path::Path::new(p).parent()).map(|p| p.to_string_lossy().to_string()));
    let Some(root) = root else {
        return validation("RootFolderPath", "'Root Folder Path' must not be empty.", json!(""), "NotEmptyValidator");
    };
    let info = match details_for_add(&st, tmdb_id, &b).await {
        Ok(i) => i,
        Err(e) => return fail(e),
    };
    let o = add_options(&b, profile, root, path);
    match movies::add(&st.db, &info, &o) {
        Ok(m) => {
            tracing::info!("Added the movie {} ({}) through the Radarr API", m.info.title, m.info.year);
            if b["addOptions"]["searchForMovie"].as_bool() == Some(true) {
                crate::downloads::spawn_search(&st, crate::downloads::Search::Movies(vec![m.id]));
            }
            with_status(201, resource(&m, Utc::now()))
        }
        Err(e) => fail(e),
    }
}

/// The add options from a Radarr movie body, with the profile and root folder already checked.
pub fn add_options(b: &Value, profile: i64, root: String, path: Option<String>) -> AddOptions {
    AddOptions {
        quality_profile_id: profile,
        root_folder: root,
        path,
        monitored: b["monitored"].as_bool().unwrap_or(true),
        minimum_availability: b["minimumAvailability"].as_str().filter(|a| movies::AVAILABILITY.contains(a)).unwrap_or("released").into(),
        tags: tag_ids(&b["tags"]),
    }
}

/// Applies the editable fields of a Radarr movie body to `m`.
fn apply_edit(m: &mut Movie, b: &Value) {
    if let Some(v) = b["monitored"].as_bool() {
        m.monitored = v;
    }
    if let Some(v) = b["qualityProfileId"].as_i64() {
        m.quality_profile_id = v;
    }
    if let Some(v) = b["minimumAvailability"].as_str().filter(|a| movies::AVAILABILITY.contains(a)) {
        m.minimum_availability = v.into();
    }
    if b["tags"].is_array() {
        m.tags = tag_ids(&b["tags"]);
    }
    if let Some(p) = b["path"].as_str().and_then(|p| crate::library::clean_path(p).ok()) {
        m.path = p;
    }
    if let Some(r) = b["rootFolderPath"].as_str().and_then(|p| crate::library::clean_path(p).ok()) {
        m.root_folder = r;
    }
}

/// `PUT movie/{id}`: change a movie (202 with it).
async fn movie_put(State(st): State<AppState>, Path(id): Path<i64>, Json(b): Json<Value>) -> Response {
    let mut m = match movies::find(&st.db, id) {
        Ok(Some(m)) => m,
        Ok(None) => return not_found(),
        Err(e) => return fail(e),
    };
    apply_edit(&mut m, &b);
    match movies::save(&st.db, &m) {
        Ok(()) => with_status(202, resource(&m, Utc::now())),
        Err(e) => fail(e),
    }
}

/// `PUT movie/editor`: change several movies at once (`movieIds` and the fields to set).
async fn movie_editor(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    let now = Utc::now();
    let mut out = Vec::new();
    for id in tag_ids(&b["movieIds"]) {
        let Ok(Some(mut m)) = movies::find(&st.db, id) else { continue };
        let mut edit = b.clone();
        // the editor's tags come with an apply mode: add, remove or replace
        if let Some(tags) = b["tags"].as_array() {
            let ids: Vec<i64> = tags.iter().filter_map(Value::as_i64).collect();
            let merged: Vec<i64> = match b["applyTags"].as_str() {
                Some("add") => m.tags.iter().copied().chain(ids.iter().copied().filter(|t| !m.tags.contains(t))).collect(),
                Some("remove") => m.tags.iter().copied().filter(|t| !ids.contains(t)).collect(),
                _ => ids,
            };
            edit["tags"] = json!(merged);
        }
        edit["path"] = Value::Null;
        apply_edit(&mut m, &edit);
        if let Err(e) = movies::save(&st.db, &m) {
            return fail(e);
        }
        out.push(resource(&m, now));
    }
    with_status(202, json!(out))
}

/// `DELETE movie/{id}`: forget a movie. Its files stay on disk (`deleteFiles` is not followed yet: Marquee
/// does not delete media files through this API).
async fn movie_delete(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match movies::remove(&st.db, id) {
        Ok(true) => Json(json!({})).into_response(),
        Ok(false) => not_found(),
        Err(e) => fail(e),
    }
}

/// `GET movie/lookup?term=`.
async fn lookup_term(State(st): State<AppState>, Query(q): Query<std::collections::HashMap<String, String>>) -> Result<Json<Value>> {
    let term = q.get("term").cloned().unwrap_or_default();
    let now = Utc::now();
    let found = movies::search(&st, &term).await?;
    let mut out = Vec::new();
    for info in &found {
        out.push(lookup_resource(&st, info, now)?);
    }
    Ok(Json(json!(out)))
}

/// `GET movie/lookup/tmdb?tmdbId=`.
async fn lookup_tmdb(State(st): State<AppState>, Query(q): Query<std::collections::HashMap<String, String>>) -> Result<Json<Value>> {
    let id: i64 = q.get("tmdbId").and_then(|i| i.parse().ok()).unwrap_or(0);
    let info = movies::tmdb(&st).movie(id).await?;
    Ok(Json(lookup_resource(&st, &info, Utc::now())?))
}

/// `GET movie/lookup/imdb?imdbId=`.
async fn lookup_imdb(State(st): State<AppState>, Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let imdb = q.get("imdbId").cloned().unwrap_or_default();
    if let Ok(Some(m)) = movies::by_imdb(&st.db, &imdb) {
        return Json(resource(&m, Utc::now())).into_response();
    }
    let t = movies::tmdb(&st);
    match t.find_imdb(&imdb).await {
        Ok(Some(id)) => match t.movie(id).await.and_then(|info| lookup_resource(&st, &info, Utc::now())) {
            Ok(v) => Json(v).into_response(),
            Err(e) => fail(e),
        },
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// `GET calendar` query.
#[derive(Deserialize)]
struct CalendarQuery {
    /// from (default today)
    start: Option<String>,
    /// to (default two days from today)
    end: Option<String>,
    /// include movies Marquee doesn't look for
    unmonitored: Option<bool>,
}

/// A date or date-time of a query, as UTC.
fn query_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&Utc)).ok().or_else(|| movies::day(s))
}

/// `GET calendar?start=&end=`: movies with a cinema, disc or digital date in the window.
async fn calendar(State(st): State<AppState>, Query(q): Query<CalendarQuery>) -> Result<Json<Value>> {
    let now = Utc::now();
    let today = movies::day(&now.format("%Y-%m-%d").to_string()).unwrap_or(now);
    let start = q.start.as_deref().and_then(query_time).unwrap_or(today);
    let end = q.end.as_deref().and_then(query_time).unwrap_or(today + Duration::days(2));
    let all = q.unmonitored.unwrap_or(false);
    let list: Vec<Value> =
        movies::all(&st.db)?.iter().filter(|m| (all || m.monitored) && m.in_window(start, end)).map(|m| resource(m, now)).collect();
    Ok(Json(json!(list)))
}

/// `GET wanted/missing`: monitored movies without a file.
async fn wanted_missing(State(st): State<AppState>, Query(q): Query<Paging>) -> Result<Json<Value>> {
    let now = Utc::now();
    let mut list: Vec<Movie> = movies::all(&st.db)?.into_iter().filter(|m| m.monitored && !m.has_file()).collect();
    if q.sort_direction.as_deref() == Some("descending") {
        list.reverse();
    }
    let records = list.iter().map(|m| resource(m, now)).collect();
    Ok(Json(paged(&q, "movieMetadata.sortTitle", "ascending", records)))
}

/// `POST command`: `RefreshMovie` fetches the movies' details again from TMDB; the search commands are
/// recorded (searching arrives with the downloads step); every other command is accepted and recorded.
async fn command_add(State(st): State<AppState>, Json(mut b): Json<Value>) -> Response {
    let name = b["name"].as_str().unwrap_or("").to_string();
    if name.is_empty() {
        return validation("Name", "'Name' must not be empty.", json!(""), "NotEmptyValidator");
    }
    if name == "RefreshMovie" {
        if let Some(o) = b.as_object_mut() {
            o.entry("isNewMovie").or_insert(json!(false));
            o.entry("isLongRunning").or_insert(json!(true));
        }
        let ids = tag_ids(&b["movieIds"]);
        let st2 = st.clone();
        tokio::spawn(async move { refresh(&st2, &ids).await });
    }
    match name.as_str() {
        "MoviesSearch" => crate::downloads::spawn_search(&st, crate::downloads::Search::Movies(tag_ids(&b["movieIds"]))),
        "MissingMoviesSearch" | "CutoffUnmetMoviesSearch" => {
            crate::downloads::spawn_search(&st, crate::downloads::Search::Missing { movies: true, tv: false })
        }
        _ => {}
    }
    match arr::record_command(&st.db, APP, &name, &b) {
        Ok(v) => with_status(201, v),
        Err(e) => fail(e),
    }
}

/// Fetches the details of the movies `ids` (every movie when empty) again from TMDB.
pub async fn refresh(st: &AppState, ids: &[i64]) {
    let Ok(list) = movies::all(&st.db) else { return };
    let t = movies::tmdb(st);
    for mut m in list.into_iter().filter(|m| ids.is_empty() || ids.contains(&m.id)) {
        match t.movie(m.info.tmdb_id).await {
            Ok(info) => {
                m.info = info;
                if let Err(e) = movies::save(&st.db, &m) {
                    tracing::warn!("The details of {} can't be saved: {}", m.info.title, e.detail);
                }
                use kit_server_core::{p, Db};
                let _ = st.db.execute("UPDATE movies SET last_info_sync=? WHERE id=?", p![movies::now_iso(), m.id]);
            }
            Err(e) => tracing::warn!("The details of {} can't be fetched again: {}", m.info.title, e.detail),
        }
    }
}

/// `GET config/naming`: how movie folders and files are named.
async fn naming(State(st): State<AppState>) -> Json<Value> {
    if let Some(v) = crate::trash::naming(&st, "movie") {
        return Json(v);
    }
    Json(json!({
        "renameMovies": false,
        "replaceIllegalCharacters": true,
        "colonReplacementFormat": "smart",
        "standardMovieFormat": "{Movie Title} ({Release Year}) {Quality Full}",
        "movieFolderFormat": "{Movie Title} ({Release Year})",
        "id": 1,
    }))
}

/// `GET config/ui`: how dates and times read (clients copy these).
async fn ui() -> Json<Value> {
    Json(json!({
        "firstDayOfWeek": 1,
        "calendarWeekColumnHeader": "ddd M/D",
        "movieRuntimeFormat": "hoursMinutes",
        "shortDateFormat": "MMM D YYYY",
        "longDateFormat": "dddd, MMMM D YYYY",
        "timeFormat": "HH:mm",
        "showRelativeDates": true,
        "enableColorImpairedMode": false,
        "movieInfoLanguage": 1,
        "uiLanguage": 1,
        "theme": "auto",
        "id": 1,
    }))
}
