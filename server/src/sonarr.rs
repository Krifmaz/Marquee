//! The Sonarr v3 API, so tools made for Sonarr (Prowlarr, Seerr, Homepage, nzb360, LunaSea) work with Marquee
//! unchanged. Base URL `http://marquee:8080/sonarr`, Marquee's API key (`X-Api-Key` or `?apikey=`).
//!
//! | Path under `/sonarr/api/v3` | What |
//! | --- | --- |
//! | `system/status` | the server (plus the shared routes of [`crate::arr_common`]) |
//! | `series`, `series/{id}`, `series/lookup`, `series/editor`, `seasonpass` | series |
//! | `episode`, `episode/{id}`, `episode/monitor`, `episodefile` | episodes |
//! | `calendar`, `wanted/missing`, `wanted/cutoff` | what airs and what is missing |
//! | `languageprofile`, `config/naming`, `config/ui`, `command` | what tools read and send |
//!
//! Answers follow Sonarr 4's shapes; `tests/contracts/sonarr/` holds real Sonarr answers they are checked
//! against. Episode air times are midnight UTC of the air date: TMDB gives dates only.

use crate::arr::{self, not_found, paged, validation, with_status, Paging};
use crate::arr_common::{fail, App};
use crate::series::{self, AddOptions, Episode, Season, Series};
use crate::tmdb::SeriesInfo;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use kit_server_core::Result;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The Sonarr version Marquee answers as (clients check it is v3 or later).
pub const COMPAT_VERSION: &str = "4.0.20.3014";

/// The app name in commands and indexers.
const APP: &str = "sonarr";

/// What the shared routes need to know about Sonarr.
pub static APP_INFO: App = App {
    name: APP,
    profile_kind: "tv",
    root_kinds: &["tv", "anime"],
    languages: include_str!("../data/sonarr/language.json"),
    indexer_schema: include_str!("../data/sonarr/indexer_schema.json"),
    queue: crate::downloads::sonarr_queue,
};

/// The routes, nested under `/sonarr`.
pub fn router(st: AppState) -> Router<AppState> {
    let v3 = Router::new()
        .route("/system/status", get(status))
        .route("/series", get(series_list).post(series_add))
        .route("/series/editor", put(series_editor))
        .route("/series/lookup", get(lookup))
        .route("/series/{id}", get(series_one).put(series_put).delete(series_delete))
        .route("/seasonpass", post(season_pass))
        .route("/episode", get(episode_list))
        .route("/episode/monitor", put(episode_monitor))
        .route("/episode/{id}", get(episode_one).put(episode_put))
        .route("/episodefile", get(|| async { Json(json!([])) }))
        .route("/calendar", get(calendar))
        .route("/wanted/missing", get(wanted_missing))
        .route("/wanted/cutoff", get(|Query(q): Query<Paging>| async move { Json(paged(&q, "episodes.airDateUtc", "descending", vec![])) }))
        .route("/history/series", get(|| async { Json(json!([])) }))
        .route("/languageprofile", get(language_profiles))
        .route("/command", post(command_add))
        .route("/config/naming", get(naming))
        .route("/config/ui", get(ui))
        .merge(crate::localization::routes(crate::localization::App::Sonarr))
        .merge(crate::custom_filters::routes("sonarr"))
        .route("/indexerflag", get(|| async { Json(crate::arr_common::indexer_flags("sonarr")) }))
        .merge(crate::arr_common::routes(&APP_INFO));
    Router::new()
        .nest("/sonarr/api/v3", v3)
        .route("/sonarr/api", get(arr::api_versions))
        .route("/sonarr/ping", get(arr::ping))
        .layer(axum::middleware::from_fn_with_state(st, arr::require_key))
}

/// `GET system/status`.
async fn status(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "appName": "Sonarr",
        "instanceName": st.app_name(),
        "version": COMPAT_VERSION,
        "buildTime": "2026-09-16T16:37:39Z",
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
        "mode": "console",
        "branch": "main",
        "authentication": "external",
        "migrationVersion": 217,
        "urlBase": "/sonarr",
        "runtimeVersion": "rust",
        "runtimeName": "marquee",
        "startTime": st.started,
        "packageVersion": crate::VERSION,
        "packageAuthor": "Marquee",
        "packageUpdateMechanism": "docker",
        "databaseVersion": "3",
        "databaseType": "sqLite",
    }))
}

/// A title as a URL slug: `the-bear`.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// A Sonarr `images` list from TMDB paths.
fn images(info: &SeriesInfo) -> Value {
    let mut out = Vec::new();
    for (kind, path) in [("poster", &info.poster), ("fanart", &info.fanart)] {
        if !path.is_empty() {
            let url = crate::movies::poster_url(path);
            out.push(json!({ "coverType": kind, "url": url, "remoteUrl": url }));
        }
    }
    json!(out)
}

/// An air date as Sonarr writes a time: `2022-06-23T00:00:00Z`.
fn air_utc(d: &str) -> String {
    crate::movies::iso_day(d)
}

/// Sonarr's statistics for a group of episodes.
fn stats(eps: &[&Episode], now: DateTime<Utc>) -> Map<String, Value> {
    let mut m = Map::new();
    let next = eps.iter().filter(|e| e.monitored && !e.aired(now)).filter_map(|e| e.air_date.clone()).min();
    let prev = eps.iter().filter(|e| e.monitored && e.aired(now)).filter_map(|e| e.air_date.clone()).max();
    if let Some(d) = next {
        m.insert("nextAiring".into(), json!(air_utc(&d)));
    }
    if let Some(d) = prev {
        m.insert("previousAiring".into(), json!(air_utc(&d)));
    }
    let files = eps.iter().filter(|e| e.has_file()).count();
    // Sonarr counts what it expects to have: monitored episodes that aired, and anything with a file
    let count = eps.iter().filter(|e| e.has_file() || (e.monitored && e.aired(now))).count();
    m.insert("episodeFileCount".into(), json!(files));
    m.insert("episodeCount".into(), json!(count));
    m.insert("totalEpisodeCount".into(), json!(eps.len()));
    m.insert("sizeOnDisk".into(), json!(0));
    m.insert("releaseGroups".into(), json!([]));
    m.insert("percentOfEpisodes".into(), json!(if count == 0 { 0.0 } else { files as f64 * 100.0 / count as f64 }));
    m
}

/// The fields every Sonarr series has, from its details.
fn info_fields(info: &SeriesInfo) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("title".into(), json!(info.title));
    o.insert("alternateTitles".into(), json!([]));
    o.insert("sortTitle".into(), json!(crate::movies::sort_title(&info.title)));
    let status = match info.tmdb_status.as_str() {
        "Ended" | "Canceled" => "ended",
        "Planned" | "Pilot" => "upcoming",
        _ => "continuing",
    };
    o.insert("status".into(), json!(status));
    o.insert("ended".into(), json!(status == "ended"));
    o.insert("overview".into(), json!(info.overview));
    o.insert("network".into(), json!(info.network));
    o.insert("airTime".into(), json!(""));
    o.insert("images".into(), images(info));
    o.insert("originalLanguage".into(), arr::language(&info.original_language));
    o.insert("year".into(), json!(info.year));
    o.insert("runtime".into(), json!(info.runtime));
    o.insert("tvdbId".into(), json!(info.tvdb_id));
    o.insert("tvRageId".into(), json!(info.tvrage_id));
    o.insert("tvMazeId".into(), json!(0));
    o.insert("tmdbId".into(), json!(info.tmdb_id));
    if let Some(d) = &info.first_aired {
        o.insert("firstAired".into(), json!(air_utc(d)));
    }
    o.insert("cleanTitle".into(), json!(crate::movies::clean_title(&info.title)));
    if !info.imdb_id.is_empty() {
        o.insert("imdbId".into(), json!(info.imdb_id));
    }
    o.insert("titleSlug".into(), json!(slug(&info.title)));
    o.insert("certification".into(), json!(info.certification));
    o.insert("genres".into(), json!(info.genres));
    o.insert("ratings".into(), json!({ "votes": info.votes, "value": info.rating }));
    o
}

/// A series in Sonarr's shape, with its episodes' statistics.
pub fn resource(s: &Series, eps: &[Episode], now: DateTime<Utc>) -> Value {
    let mut o = info_fields(&s.info);
    o.insert("status".into(), json!(s.status()));
    o.insert("ended".into(), json!(s.status() == "ended"));
    let all: Vec<&Episode> = eps.iter().collect();
    let st = stats(&all, now);
    for k in ["nextAiring", "previousAiring"] {
        if let Some(v) = st.get(k) {
            o.insert(k.into(), v.clone());
        }
    }
    if let Some(last) = eps.iter().filter_map(|e| e.air_date.clone()).max() {
        o.insert("lastAired".into(), json!(air_utc(&last)));
    }
    let seasons: Vec<Value> = s
        .seasons
        .iter()
        .map(|x| {
            let in_season: Vec<&Episode> = eps.iter().filter(|e| e.season == x.season).collect();
            json!({ "seasonNumber": x.season, "monitored": x.monitored, "statistics": Value::Object(stats(&in_season, now)) })
        })
        .collect();
    o.insert("seasons".into(), json!(seasons));
    o.insert("path".into(), json!(s.path));
    o.insert("qualityProfileId".into(), json!(s.quality_profile_id));
    o.insert("seasonFolder".into(), json!(s.season_folder));
    o.insert("monitored".into(), json!(s.monitored));
    o.insert("monitorNewItems".into(), json!(s.monitor_new_items));
    o.insert("useSceneNumbering".into(), json!(false));
    o.insert("seriesType".into(), json!(s.series_type));
    o.insert("rootFolderPath".into(), json!(s.root_folder));
    o.insert("tags".into(), json!(s.tags));
    o.insert("added".into(), json!(s.added));
    o.insert(
        "addOptions".into(),
        json!({ "searchForMissingEpisodes": false, "searchForCutoffUnmetEpisodes": false, "ignoreEpisodesWithFiles": false, "ignoreEpisodesWithoutFiles": false, "monitor": "unknown" }),
    );
    let mut total = stats(&all, now);
    total.remove("nextAiring");
    total.remove("previousAiring");
    total.insert("seasonCount".into(), json!(s.seasons.iter().filter(|x| x.season > 0).count()));
    let regular: Vec<&Episode> = eps.iter().filter(|e| e.season > 0).collect();
    let reg = stats(&regular, now);
    total.insert("episodeCount".into(), reg["episodeCount"].clone());
    total.insert("episodeFileCount".into(), reg["episodeFileCount"].clone());
    total.insert("percentOfEpisodes".into(), reg["percentOfEpisodes"].clone());
    o.insert("statistics".into(), Value::Object(total));
    o.insert("languageProfileId".into(), json!(1));
    o.insert("id".into(), json!(s.id));
    Value::Object(o)
}

/// An episode in Sonarr's shape (with its series when `with` is given).
pub fn episode_resource(e: &Episode, with: Option<&Value>) -> Value {
    let mut o = Map::new();
    o.insert("seriesId".into(), json!(e.series_id));
    o.insert("tvdbId".into(), json!(0));
    o.insert("episodeFileId".into(), json!(e.episode_file_id));
    o.insert("seasonNumber".into(), json!(e.season));
    o.insert("episodeNumber".into(), json!(e.episode));
    o.insert("title".into(), json!(e.title));
    if let Some(d) = &e.air_date {
        o.insert("airDate".into(), json!(d));
        o.insert("airDateUtc".into(), json!(air_utc(d)));
    }
    o.insert("runtime".into(), json!(e.runtime));
    o.insert("overview".into(), json!(e.overview));
    o.insert("hasFile".into(), json!(e.has_file()));
    o.insert("monitored".into(), json!(e.monitored));
    if let Some(a) = e.absolute {
        o.insert("absoluteEpisodeNumber".into(), json!(a));
    }
    o.insert("unverifiedSceneNumbering".into(), json!(false));
    if let Some(s) = with {
        o.insert("series".into(), s.clone());
    }
    o.insert("id".into(), json!(e.id));
    Value::Object(o)
}

/// Every series with its episodes, in Sonarr's shape, by id.
fn all_resources(st: &AppState, now: DateTime<Utc>) -> Result<HashMap<i64, Value>> {
    let eps = series::all_episodes(&st.db)?;
    Ok(series::all(&st.db)?
        .iter()
        .map(|s| {
            let mine: Vec<Episode> = eps.iter().filter(|e| e.series_id == s.id).cloned().collect();
            (s.id, resource(s, &mine, now))
        })
        .collect())
}

/// One series in Sonarr's shape, `None` when there is no such series.
fn one_resource(st: &AppState, id: i64, now: DateTime<Utc>) -> Result<Option<Value>> {
    match series::find(&st.db, id)? {
        Some(s) => Ok(Some(resource(&s, &series::episodes(&st.db, id)?, now))),
        None => Ok(None),
    }
}

/// `GET series` query.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesQuery {
    /// only the series with this TheTVDB id
    tvdb_id: Option<i64>,
}

/// `GET series` (`?tvdbId=` narrows it).
async fn series_list(State(st): State<AppState>, Query(q): Query<SeriesQuery>) -> Result<Json<Value>> {
    let now = Utc::now();
    let eps = series::all_episodes(&st.db)?;
    let list: Vec<Value> = series::all(&st.db)?
        .iter()
        .filter(|s| q.tvdb_id.is_none_or(|t| s.info.tvdb_id == t))
        .map(|s| {
            let mine: Vec<Episode> = eps.iter().filter(|e| e.series_id == s.id).cloned().collect();
            resource(s, &mine, now)
        })
        .collect();
    Ok(Json(json!(list)))
}

/// `GET series/{id}`.
async fn series_one(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match one_resource(&st, id, Utc::now()) {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// A lookup result in Sonarr's shape: the added series when it is added, else TMDB's details with a folder.
fn lookup_resource(st: &AppState, info: &SeriesInfo, now: DateTime<Utc>) -> Result<Value> {
    if let Some(s) = series::by_tmdb(&st.db, info.tmdb_id)? {
        return Ok(resource(&s, &series::episodes(&st.db, s.id)?, now));
    }
    let mut o = info_fields(info);
    o.insert("seasons".into(), json!(info.seasons.iter().map(|n| json!({ "seasonNumber": n, "monitored": false })).collect::<Vec<_>>()));
    o.insert("folder".into(), json!(series::folder_name(&info.title)));
    o.insert("qualityProfileId".into(), json!(0));
    o.insert("seasonFolder".into(), json!(false));
    o.insert("monitored".into(), json!(false));
    o.insert("monitorNewItems".into(), json!("all"));
    o.insert("useSceneNumbering".into(), json!(false));
    o.insert("seriesType".into(), json!("standard"));
    o.insert("tags".into(), json!([]));
    o.insert("added".into(), json!("0001-01-01T00:00:00Z"));
    o.insert("statistics".into(), json!({ "seasonCount": info.seasons.iter().filter(|n| **n > 0).count(), "episodeFileCount": 0, "episodeCount": 0, "totalEpisodeCount": 0, "sizeOnDisk": 0, "releaseGroups": [], "percentOfEpisodes": 0 }));
    o.insert("languageProfileId".into(), json!(1));
    Ok(Value::Object(o))
}

/// `GET series/lookup?term=` (`tvdb:123` finds one series by TheTVDB id, as Sonarr does).
async fn lookup(State(st): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>> {
    let term = q.get("term").cloned().unwrap_or_default();
    let now = Utc::now();
    let mut out = Vec::new();
    for info in series::search(&st, &term).await? {
        out.push(lookup_resource(&st, &info, now)?);
    }
    Ok(Json(json!(out)))
}

/// The tag ids (or other ids) of a list field.
fn ids(v: &Value) -> Vec<i64> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()
}

/// The seasons of a Sonarr body (`[{seasonNumber, monitored}]`), `None` when it has none.
fn body_seasons(v: &Value) -> Option<Vec<Season>> {
    let a = v.as_array()?;
    let out: Vec<Season> = a
        .iter()
        .filter_map(|x| Some(Season { season: x["seasonNumber"].as_i64()?, monitored: x["monitored"].as_bool().unwrap_or(false) }))
        .collect();
    (!out.is_empty()).then_some(out)
}

/// `POST series`: add a series (201 with it), Sonarr's checks and their error shapes.
async fn series_add(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    let tvdb_id = b["tvdbId"].as_i64().unwrap_or(0);
    let tmdb_hint = b["tmdbId"].as_i64().unwrap_or(0);
    if tvdb_id <= 0 && tmdb_hint <= 0 {
        return validation("TvdbId", "'Tvdb Id' must be greater than '0'.", b["tvdbId"].clone(), "GreaterThanValidator");
    }
    match series::by_tvdb(&st.db, tvdb_id) {
        Ok(Some(_)) => return validation("TvdbId", "This series has already been added", json!(tvdb_id), "SeriesExistsValidator"),
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
    let (info, eps) = match details_for_add(&st, tvdb_id, tmdb_hint, &b).await {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let o = add_options(&b, profile, root, path);
    match series::add(&st.db, &info, eps, &o) {
        Ok(s) => {
            tracing::info!("Added the series {} through the Sonarr API", s.info.title);
            if b["addOptions"]["searchForMissingEpisodes"].as_bool() == Some(true) {
                crate::downloads::spawn_search(&st, crate::downloads::Search::Series(s.id, None));
            }
            match series::episodes(&st.db, s.id) {
                Ok(eps) => with_status(201, resource(&s, &eps, Utc::now())),
                Err(e) => fail(e),
            }
        }
        Err(e) if e.status.as_u16() == 409 => {
            validation("TvdbId", "This series has already been added", json!(tvdb_id), "SeriesExistsValidator")
        }
        Err(e) => fail(e),
    }
}

/// A series' details and episodes for adding it by TheTVDB id (TMDB's, or `tmdb_hint` when the caller knows it).
/// When TMDB can't be asked, the details in the Sonarr body `b` are kept and the next refresh fills in the rest.
pub async fn details_for_add(
    st: &AppState,
    tvdb_id: i64,
    tmdb_hint: i64,
    b: &Value,
) -> kit_server_core::Result<(SeriesInfo, Vec<crate::tmdb::EpisodeInfo>)> {
    let t = crate::movies::tmdb(st);
    let tmdb_id = if tmdb_hint > 0 { Ok(Some(tmdb_hint)) } else { t.find_tvdb(tvdb_id).await };
    let fetched = match tmdb_id {
        Ok(Some(id)) => match t.series(id).await {
            Ok(info) => series::fetch_episodes(&t, &info).await.map(|eps| (info, eps)),
            Err(e) => Err(e),
        },
        Ok(None) => Err(kit_server_core::err(404, "TMDB doesn't know this series")),
        Err(e) => Err(e),
    };
    let (mut info, eps) = match fetched {
        Ok(x) => x,
        Err(e) if e.status.as_u16() == 404 || b["title"].as_str().is_none_or(str::is_empty) => return Err(e),
        Err(e) => {
            // TMDB can't be asked: keep what the caller sent, the next refresh fills in the episodes
            tracing::warn!("TMDB couldn't be asked about TheTVDB {tvdb_id} ({}); adding it with the details sent", e.detail);
            let info = SeriesInfo {
                tmdb_id: if tmdb_hint > 0 { tmdb_hint } else { -tvdb_id },
                tvdb_id,
                title: b["title"].as_str().unwrap_or("").into(),
                overview: b["overview"].as_str().unwrap_or("").into(),
                year: b["year"].as_i64().unwrap_or(0),
                seasons: body_seasons(&b["seasons"]).map(|s| s.iter().map(|x| x.season).collect()).unwrap_or_default(),
                ..SeriesInfo::default()
            };
            (info, vec![])
        }
    };
    if info.tvdb_id == 0 {
        info.tvdb_id = tvdb_id;
    }
    Ok((info, eps))
}

/// The add options from a Sonarr series body, with the profile, root folder and folder already checked.
pub fn add_options(b: &Value, profile: i64, root: String, path: Option<String>) -> AddOptions {
    let monitor = b["addOptions"]["monitor"].as_str().filter(|m| series::MONITOR.contains(m)).unwrap_or("all").to_string();
    AddOptions {
        quality_profile_id: profile,
        root_folder: root,
        path,
        monitored: b["monitored"].as_bool().unwrap_or(true),
        monitor: if monitor == "unknown" { "all".into() } else { monitor },
        series_type: b["seriesType"].as_str().filter(|t| series::SERIES_TYPES.contains(t)).unwrap_or("standard").into(),
        season_folder: b["seasonFolder"].as_bool().unwrap_or(true),
        tags: ids(&b["tags"]),
        // Seerr names the seasons it wants; Sonarr follows them unless a monitor choice is given
        seasons: if b["addOptions"]["monitor"].is_string() { None } else { body_seasons(&b["seasons"]) },
    }
}

/// Applies the editable fields of a Sonarr series body to `s` (seasons are set apart, with their episodes).
fn apply_edit(s: &mut Series, b: &Value) {
    if let Some(v) = b["monitored"].as_bool() {
        s.monitored = v;
    }
    if let Some(v) = b["qualityProfileId"].as_i64() {
        s.quality_profile_id = v;
    }
    if let Some(v) = b["seasonFolder"].as_bool() {
        s.season_folder = v;
    }
    if let Some(v) = b["seriesType"].as_str().filter(|t| series::SERIES_TYPES.contains(t)) {
        s.series_type = v.into();
    }
    if let Some(v) = b["monitorNewItems"].as_str().filter(|m| matches!(*m, "all" | "none")) {
        s.monitor_new_items = v.into();
    }
    if b["tags"].is_array() {
        s.tags = ids(&b["tags"]);
    }
    if let Some(p) = b["path"].as_str().and_then(|p| crate::library::clean_path(p).ok()) {
        s.path = p;
    }
    if let Some(r) = b["rootFolderPath"].as_str().and_then(|p| crate::library::clean_path(p).ok()) {
        s.root_folder = r;
    }
}

/// `PUT series/{id}`: change a series (202 with it); a season turned on or off turns its episodes with it.
async fn series_put(State(st): State<AppState>, Path(id): Path<i64>, Json(b): Json<Value>) -> Response {
    let mut s = match series::find(&st.db, id) {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return fail(e),
    };
    apply_edit(&mut s, &b);
    let r = match body_seasons(&b["seasons"]) {
        Some(ss) => series::set_seasons(&st.db, &mut s, &ss),
        None => series::save(&st.db, &s),
    };
    if let Err(e) = r {
        return fail(e);
    }
    match one_resource(&st, id, Utc::now()) {
        Ok(Some(v)) => with_status(202, v),
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// `PUT series/editor`: change several series at once (`seriesIds` and the fields to set).
async fn series_editor(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    let now = Utc::now();
    let mut out = Vec::new();
    for id in ids(&b["seriesIds"]) {
        let Ok(Some(mut s)) = series::find(&st.db, id) else { continue };
        let mut edit = b.clone();
        if let Some(tags) = b["tags"].as_array() {
            let want: Vec<i64> = tags.iter().filter_map(Value::as_i64).collect();
            let merged: Vec<i64> = match b["applyTags"].as_str() {
                Some("add") => s.tags.iter().copied().chain(want.iter().copied().filter(|t| !s.tags.contains(t))).collect(),
                Some("remove") => s.tags.iter().copied().filter(|t| !want.contains(t)).collect(),
                _ => want,
            };
            edit["tags"] = json!(merged);
        }
        edit["path"] = Value::Null;
        apply_edit(&mut s, &edit);
        if let Err(e) = series::save(&st.db, &s) {
            return fail(e);
        }
        if let Ok(Some(v)) = one_resource(&st, id, now) {
            out.push(v);
        }
    }
    with_status(202, json!(out))
}

/// `DELETE series/{id}`: forget a series and its episodes. Files stay on disk.
async fn series_delete(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match series::remove(&st.db, id) {
        Ok(true) => Json(json!({})).into_response(),
        Ok(false) => not_found(),
        Err(e) => fail(e),
    }
}

/// `POST seasonpass`: set series and season monitoring for several series.
async fn season_pass(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    for item in b["series"].as_array().cloned().unwrap_or_default() {
        let Some(id) = item["id"].as_i64() else { continue };
        let Ok(Some(mut s)) = series::find(&st.db, id) else { continue };
        if let Some(m) = item["monitored"].as_bool() {
            s.monitored = m;
        }
        let r = match body_seasons(&item["seasons"]) {
            Some(ss) => series::set_seasons(&st.db, &mut s, &ss),
            None => series::save(&st.db, &s),
        };
        if let Err(e) = r {
            return fail(e);
        }
    }
    with_status(202, json!({}))
}

/// `GET episode` query.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EpisodeQuery {
    /// the series
    series_id: Option<i64>,
    /// only this season
    season_number: Option<i64>,
    /// only these episodes (`1,2,3`)
    episode_ids: Option<String>,
    /// add each episode's series
    include_series: Option<bool>,
}

/// `GET episode?seriesId=` (or `episodeIds=`).
async fn episode_list(State(st): State<AppState>, Query(q): Query<EpisodeQuery>) -> Result<Json<Value>> {
    let wanted: Option<Vec<i64>> = q.episode_ids.as_deref().map(|s| s.split(',').filter_map(|i| i.trim().parse().ok()).collect());
    let eps: Vec<Episode> = match q.series_id {
        Some(id) => series::episodes(&st.db, id)?,
        None if wanted.is_some() => series::all_episodes(&st.db)?,
        None => vec![],
    };
    let all = if q.include_series == Some(true) { all_resources(&st, Utc::now())? } else { HashMap::new() };
    let list: Vec<Value> = eps
        .iter()
        .filter(|e| q.season_number.is_none_or(|n| e.season == n))
        .filter(|e| wanted.as_ref().is_none_or(|w| w.contains(&e.id)))
        .map(|e| episode_resource(e, all.get(&e.series_id)))
        .collect();
    Ok(Json(json!(list)))
}

/// `GET episode/{id}`: one episode with its series.
async fn episode_one(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    match series::episode(&st.db, id) {
        Ok(Some(e)) => {
            let s = one_resource(&st, e.series_id, Utc::now()).ok().flatten();
            let mut v = episode_resource(&e, s.as_ref());
            v["images"] = json!([]);
            Json(v).into_response()
        }
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// `PUT episode/{id}`: turn looking for one episode on or off (202 with it).
async fn episode_put(State(st): State<AppState>, Path(id): Path<i64>, Json(b): Json<Value>) -> Response {
    let monitored = b["monitored"].as_bool().unwrap_or(true);
    match series::episode(&st.db, id) {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(e) => return fail(e),
    }
    if let Err(e) = series::set_episodes_monitored(&st.db, &[id], monitored) {
        return fail(e);
    }
    match series::episode(&st.db, id) {
        Ok(Some(e)) => with_status(202, episode_resource(&e, None)),
        Ok(None) => not_found(),
        Err(e) => fail(e),
    }
}

/// `PUT episode/monitor`: `{episodeIds, monitored}` for several episodes (202 with them).
async fn episode_monitor(State(st): State<AppState>, Json(b): Json<Value>) -> Response {
    let list = ids(&b["episodeIds"]);
    if let Err(e) = series::set_episodes_monitored(&st.db, &list, b["monitored"].as_bool().unwrap_or(true)) {
        return fail(e);
    }
    let out: Vec<Value> =
        list.iter().filter_map(|id| series::episode(&st.db, *id).ok().flatten()).map(|e| episode_resource(&e, None)).collect();
    with_status(202, json!(out))
}

/// `GET calendar` query.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarQuery {
    /// from (default today)
    start: Option<String>,
    /// to (default two days from today)
    end: Option<String>,
    /// include episodes Marquee doesn't look for
    unmonitored: Option<bool>,
    /// add each episode's series
    include_series: Option<bool>,
}

/// A date or date-time of a query, as UTC.
fn query_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&Utc)).ok().or_else(|| crate::movies::day(s))
}

/// `GET calendar?start=&end=`: episodes airing in the window.
async fn calendar(State(st): State<AppState>, Query(q): Query<CalendarQuery>) -> Result<Json<Value>> {
    let now = Utc::now();
    let today = crate::movies::day(&now.format("%Y-%m-%d").to_string()).unwrap_or(now);
    let start = q.start.as_deref().and_then(query_time).unwrap_or(today);
    let end = q.end.as_deref().and_then(query_time).unwrap_or(today + Duration::days(2));
    let all_series = series::all(&st.db)?;
    let watched = |id: i64| all_series.iter().any(|s| s.id == id && s.monitored);
    let full = if q.include_series == Some(true) { all_resources(&st, now)? } else { HashMap::new() };
    let unmonitored = q.unmonitored.unwrap_or(false);
    let list: Vec<Value> = series::all_episodes(&st.db)?
        .iter()
        .filter(|e| e.airs().is_some_and(|a| a >= start && a <= end))
        .filter(|e| unmonitored || (e.monitored && watched(e.series_id)))
        .map(|e| episode_resource(e, full.get(&e.series_id)))
        .collect();
    Ok(Json(json!(list)))
}

/// `GET wanted/missing`: monitored episodes that aired and have no file, newest first.
async fn wanted_missing(State(st): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>> {
    let now = Utc::now();
    let paging = Paging {
        page: q.get("page").and_then(|v| v.parse().ok()),
        page_size: q.get("pageSize").and_then(|v| v.parse().ok()),
        sort_key: q.get("sortKey").cloned(),
        sort_direction: q.get("sortDirection").cloned(),
    };
    let include = q.get("includeSeries").map(|v| v == "true").unwrap_or(false);
    let all_series = series::all(&st.db)?;
    let watched = |id: i64| all_series.iter().any(|s| s.id == id && s.monitored);
    let mut eps: Vec<Episode> = series::all_episodes(&st.db)?
        .into_iter()
        .filter(|e| e.monitored && !e.has_file() && e.aired(now) && watched(e.series_id))
        .collect();
    eps.sort_by(|a, b| b.air_date.cmp(&a.air_date).then(a.episode.cmp(&b.episode)));
    if paging.sort_direction.as_deref() == Some("ascending") {
        eps.reverse();
    }
    let full = if include { all_resources(&st, now)? } else { HashMap::new() };
    let records = eps.iter().map(|e| episode_resource(e, full.get(&e.series_id))).collect();
    Ok(Json(paged(&paging, "episodes.airDateUtc", "descending", records)))
}

/// `GET languageprofile`: Sonarr 4 keeps one deprecated profile for older clients.
async fn language_profiles() -> Json<Value> {
    Json(json!([{
        "name": "Deprecated",
        "upgradeAllowed": true,
        "cutoff": { "id": 1, "name": "English" },
        "languages": [{ "language": { "id": 1, "name": "English" }, "allowed": true }],
        "id": 1,
    }]))
}

/// `POST command`: `RefreshSeries` fetches the series' details and episodes again from TMDB; the search
/// commands are recorded (searching arrives with the downloads step); every other command is accepted.
async fn command_add(State(st): State<AppState>, Json(mut b): Json<Value>) -> Response {
    let name = b["name"].as_str().unwrap_or("").to_string();
    if name.is_empty() {
        return validation("Name", "'Name' must not be empty.", json!(""), "NotEmptyValidator");
    }
    if name == "RefreshSeries" {
        let mut list = ids(&b["seriesIds"]);
        if let Some(one) = b["seriesId"].as_i64() {
            list.push(one);
        }
        if let Some(o) = b.as_object_mut() {
            o.insert("seriesIds".into(), json!(list));
            o.remove("seriesId");
            o.entry("isNewSeries").or_insert(json!(false));
            o.entry("isLongRunning").or_insert(json!(true));
        }
        let st2 = st.clone();
        tokio::spawn(async move {
            let targets =
                if list.is_empty() { series::all(&st2.db).map(|a| a.iter().map(|s| s.id).collect()).unwrap_or_default() } else { list };
            for id in targets {
                if let Err(e) = series::refresh(&st2, id).await {
                    tracing::warn!("Series {id} can't be refreshed: {}", e.detail);
                }
            }
        });
    }
    use crate::downloads::{spawn_search, Search};
    match name.as_str() {
        "EpisodeSearch" => spawn_search(&st, Search::Episodes(ids(&b["episodeIds"]))),
        "SeasonSearch" => {
            if let Some(id) = b["seriesId"].as_i64() {
                spawn_search(&st, Search::Series(id, b["seasonNumber"].as_i64()))
            }
        }
        "SeriesSearch" => {
            if let Some(id) = b["seriesId"].as_i64() {
                spawn_search(&st, Search::Series(id, None))
            }
        }
        "MissingEpisodeSearch" | "CutoffUnmetEpisodeSearch" => spawn_search(&st, Search::Missing { movies: false, tv: true }),
        _ => {}
    }
    match arr::record_command(&st.db, APP, &name, &b) {
        Ok(v) => with_status(201, v),
        Err(e) => fail(e),
    }
}

/// `GET config/naming`: how series folders and episode files are named.
async fn naming(State(st): State<AppState>) -> Json<Value> {
    if let Some(v) = crate::trash::naming(&st, "tv") {
        return Json(v);
    }
    Json(json!({
        "renameEpisodes": false,
        "replaceIllegalCharacters": true,
        "colonReplacementFormat": 4,
        "customColonReplacementFormat": "",
        "multiEpisodeStyle": 5,
        "standardEpisodeFormat": "{Series Title} - S{season:00}E{episode:00} - {Episode Title} {Quality Full}",
        "dailyEpisodeFormat": "{Series Title} - {Air-Date} - {Episode Title} {Quality Full}",
        "animeEpisodeFormat": "{Series Title} - S{season:00}E{episode:00} - {Episode Title} {Quality Full}",
        "seriesFolderFormat": "{Series Title}",
        "seasonFolderFormat": "Season {season}",
        "specialsFolderFormat": "Specials",
        "id": 1,
    }))
}

/// `GET config/ui`.
async fn ui() -> Json<Value> {
    Json(json!({
        "firstDayOfWeek": 1,
        "calendarWeekColumnHeader": "ddd M/D",
        "shortDateFormat": "MMM D YYYY",
        "longDateFormat": "dddd, MMMM D YYYY",
        "timeFormat": "HH:mm",
        "showRelativeDates": true,
        "enableColorImpairedMode": false,
        "theme": "auto",
        "uiLanguage": 1,
        "id": 1,
    }))
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn titles_make_slugs() {
        assert_eq!(slug("The Bear"), "the-bear");
        assert_eq!(slug("Marvel's Agents of S.H.I.E.L.D."), "marvel-s-agents-of-s-h-i-e-l-d");
    }
}
