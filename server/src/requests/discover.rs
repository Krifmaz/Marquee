//! Finding something to ask for: trending, popular and upcoming movies and TV, search, and one title's details,
//! all from TMDB (`movies::tmdb`), in Seerr's shape (TMDB's names in camelCase) with `mediaInfo` added when the
//! title was asked for or is in the library, so every client shows Requested, Processing or Available.
//!
//! The routes are Seerr 3.5.1's `routes/discover.ts`:
//!
//! ```text
//! GET /api/v1/discover/movies                     TMDB discover with Seerr's filters, plus the keywords asked for
//! GET /api/v1/discover/movies/language/{lang}     movies first made in one language
//! GET /api/v1/discover/movies/genre/{id}          movies of one genre
//! GET /api/v1/discover/movies/studio/{id}         movies of one studio
//! GET /api/v1/discover/movies/upcoming            movies coming out from today
//! GET /api/v1/discover/tv                         the same for series
//! GET /api/v1/discover/tv/language/{lang}         series first made in one language
//! GET /api/v1/discover/tv/genre/{id}              series of one genre
//! GET /api/v1/discover/tv/network/{id}            series of one network
//! GET /api/v1/discover/tv/upcoming                series starting from today
//! GET /api/v1/discover/trending                   trending (`mediaType` all|movie|tv, `timeWindow` day|week)
//! GET /api/v1/discover/keyword/{id}/movies        movies with one keyword
//! GET /api/v1/discover/genreslider/movie          each movie genre with backdrops, by name
//! GET /api/v1/discover/genreslider/tv             each series genre with backdrops, by name
//! GET /api/v1/discover/watchlist                  the Plex watchlist (empty until watchlist sync is ported)
//! ```

use super::seerr::{media_json, Answer, Fail, Seerr};
use super::{media_by_tmdb, upsert_media, TitleFacts};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{err, Result, SqliteDb};
use serde_json::{json, Map, Value};

/// `poster_path` → `posterPath`.
pub fn camel_key(k: &str) -> String {
    let mut out = String::with_capacity(k.len());
    let mut up = false;
    for c in k.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// TMDB's answer with every key in camelCase, as Seerr sends it (`watch/providers` becomes `watchProviders`).
pub fn camel(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut m = Map::new();
            for (k, x) in o {
                let key = if k == "watch/providers" { "watchProviders".to_string() } else { camel_key(k) };
                m.insert(key, camel(x));
            }
            Value::Object(m)
        }
        Value::Array(a) => Value::Array(a.iter().map(camel).collect()),
        x => x.clone(),
    }
}

/// The `mediaInfo` of the title `kind`/`tmdb_id`: Seerr's media object when somebody asked for it or the library
/// has it (a title row is made for a library title the first time it is shown), else `None`.
pub fn media_info(db: &SqliteDb, kind: &str, tmdb_id: i64) -> Result<Option<Value>> {
    if let Some(m) = media_by_tmdb(db, kind, tmdb_id)? {
        return Ok(Some(media_json(db, &m, true)?));
    }
    let facts = if kind == "movie" {
        crate::movies::by_tmdb(db, tmdb_id)?.map(|m| TitleFacts {
            title: m.info.title.clone(),
            year: m.info.year,
            poster: m.info.poster.clone(),
            imdb_id: m.info.imdb_id.clone(),
            anime: super::is_anime(&m.info.genres, &m.info.original_language),
            ..TitleFacts::default()
        })
    } else {
        crate::series::by_tmdb(db, tmdb_id)?.map(|s| TitleFacts {
            anime: s.kind == "anime",
            title: s.info.title.clone(),
            year: s.info.year,
            poster: s.info.poster.clone(),
            imdb_id: s.info.imdb_id.clone(),
            tvdb_id: Some(s.info.tvdb_id).filter(|v| *v > 0),
            seasons: vec![],
        })
    };
    match facts {
        Some(f) => Ok(Some(media_json(db, &upsert_media(db, kind, tmdb_id, &f)?, true)?)),
        None => Ok(None),
    }
}

/// One TMDB result (movie, TV or person) in Seerr's shape; `kind` is its kind when TMDB doesn't say (discover
/// answers).
pub fn result(db: &SqliteDb, v: &Value, kind: Option<&str>) -> Result<Value> {
    let mut r = camel(v);
    let k = v["media_type"].as_str().or(kind).unwrap_or("movie").to_string();
    r["mediaType"] = json!(k);
    if k == "movie" || k == "tv" {
        if let Some(id) = v["id"].as_i64() {
            if let Some(info) = media_info(db, &k, id)? {
                r["mediaInfo"] = info;
            }
        }
    }
    Ok(r)
}

/// A TMDB page (`{page, total_pages, total_results, results}`) in Seerr's shape.
pub fn page(db: &SqliteDb, v: &Value, kind: Option<&str>) -> Result<Value> {
    let mut results = Vec::new();
    for x in v["results"].as_array().into_iter().flatten() {
        results.push(result(db, x, kind)?);
    }
    Ok(json!({
        "page": v["page"].as_i64().unwrap_or(1),
        "totalPages": v["total_pages"].as_i64().unwrap_or(1),
        "totalResults": v["total_results"].as_i64().unwrap_or(results.len() as i64),
        "results": results,
    }))
}

/// The query parameters Seerr passes on to TMDB: the page (1 when missing) and the language, when given.
pub fn base_query(page: Option<i64>, language: Option<&str>) -> Vec<(&'static str, String)> {
    let mut q = vec![("page", page.unwrap_or(1).clamp(1, 500).to_string()), ("include_adult", "false".to_string())];
    if let Some(l) = language.filter(|l| !l.is_empty()) {
        q.push(("language", l.to_string()));
    }
    q
}

/// TMDB's `path` with `query` as a Seerr page of `kind`.
pub async fn tmdb_page(st: &AppState, path: &str, query: &[(&str, String)], kind: Option<&str>) -> Result<Value> {
    let v = crate::movies::tmdb(st).get(path, query).await?;
    page(&st.db, &v, kind)
}

/// Today, `YYYY-MM-DD`, for "upcoming".
pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// One movie's details in Seerr's shape.
pub async fn movie(st: &AppState, tmdb_id: i64, language: Option<&str>) -> Result<Value> {
    let mut q = vec![("append_to_response", "credits,external_ids,videos,release_dates,watch/providers".to_string())];
    q.extend(base_query(None, language).into_iter().filter(|(k, _)| *k == "language"));
    let v = crate::movies::tmdb(st).get(&format!("/movie/{tmdb_id}"), &q).await?;
    let mut d = camel(&v);
    if let Some(o) = d.as_object_mut() {
        o.remove("belongsToCollection");
        o.remove("releaseDates");
        o.remove("videos");
    }
    d["mediaType"] = json!("movie");
    d["collection"] = camel(&v["belongs_to_collection"]);
    d["releases"] = camel(&v["release_dates"]);
    d["relatedVideos"] = videos(&v);
    d["mediaInfo"] = media_info(&st.db, "movie", tmdb_id)?.unwrap_or(Value::Null);
    if d["mediaInfo"].is_null() {
        d.as_object_mut().map(|o| o.remove("mediaInfo"));
    }
    Ok(d)
}

/// One series' details in Seerr's shape.
pub async fn tv(st: &AppState, tmdb_id: i64, language: Option<&str>) -> Result<Value> {
    let mut q = vec![("append_to_response", "credits,external_ids,content_ratings,keywords,videos,watch/providers".to_string())];
    q.extend(base_query(None, language).into_iter().filter(|(k, _)| *k == "language"));
    let v = crate::movies::tmdb(st).get(&format!("/tv/{tmdb_id}"), &q).await?;
    let mut d = camel(&v);
    if let Some(o) = d.as_object_mut() {
        o.remove("videos");
        o.remove("numberOfSeasons");
    }
    d["mediaType"] = json!("tv");
    d["numberOfSeasons"] = v["number_of_seasons"].clone();
    d["keywords"] = camel(&v["keywords"]["results"]);
    d["relatedVideos"] = videos(&v);
    d["mediaInfo"] = media_info(&st.db, "tv", tmdb_id)?.unwrap_or(Value::Null);
    if d["mediaInfo"].is_null() {
        d.as_object_mut().map(|o| o.remove("mediaInfo"));
    }
    Ok(d)
}

/// TMDB's videos as Seerr's `relatedVideos`: `{url, key, name, size, type, site}`.
pub fn videos(v: &Value) -> Value {
    let list: Vec<Value> = v["videos"]["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let key = x["key"].as_str().unwrap_or("");
            let url = if x["site"] == "YouTube" { format!("https://www.youtube.com/watch?v={key}") } else { String::new() };
            json!({ "url": url, "key": key, "name": x["name"], "size": x["size"], "type": x["type"], "site": x["site"] })
        })
        .collect();
    json!(list)
}

/// The query a Seerr discover call was given (`page`, `genre`, `studio`, `keywords`, `sortBy`…).
pub type Filters = std::collections::HashMap<String, String>;

/// One query value, when it is there and not empty.
fn given<'a>(q: &'a Filters, k: &str) -> Option<&'a str> {
    q.get(k).map(String::as_str).map(str::trim).filter(|v| !v.is_empty())
}

/// TMDB's language for an answer: the `language` asked for, else English (Seerr's `req.locale`).
pub fn locale(q: &Filters) -> String {
    given(q, "locale").unwrap_or("en").to_string()
}

/// TMDB's date range: when only one end is given, Seerr fills the other (1900-01-01, or 18 months ahead) because
/// TMDB needs both.
fn date_range(out: &mut Vec<(&'static str, String)>, gte_key: &'static str, lte_key: &'static str, gte: Option<&str>, lte: Option<&str>) {
    let future = (chrono::Utc::now() + chrono::Duration::days(547)).format("%Y-%m-%d").to_string();
    let day = |d: &str| d.get(..10).unwrap_or(d).to_string();
    match (gte, lte) {
        (Some(g), Some(l)) => out.extend([(gte_key, day(g)), (lte_key, day(l))]),
        (Some(g), None) => out.extend([(gte_key, day(g)), (lte_key, future)]),
        (None, Some(l)) => out.extend([(gte_key, "1900-01-01".to_string()), (lte_key, day(l))]),
        (None, None) => {}
    }
}

/// The filters both movie and TV discover pass on to TMDB, by Seerr's names → TMDB's.
const SHARED: [(&str, &str); 11] = [
    ("genre", "with_genres"),
    ("keywords", "with_keywords"),
    ("excludeKeywords", "without_keywords"),
    ("withRuntimeGte", "with_runtime.gte"),
    ("withRuntimeLte", "with_runtime.lte"),
    ("voteAverageGte", "vote_average.gte"),
    ("voteAverageLte", "vote_average.lte"),
    ("voteCountGte", "vote_count.gte"),
    ("voteCountLte", "vote_count.lte"),
    ("watchProviders", "with_watch_providers"),
    ("watchRegion", "watch_region"),
];

/// TMDB's `/discover/movie` query for Seerr's filters (`getDiscoverMovies`): `language` is the original language
/// filter, the answer's language is English.
pub fn movie_query(q: &Filters) -> Vec<(&'static str, String)> {
    let page: i64 = given(q, "page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let mut out = vec![
        ("sort_by", given(q, "sortBy").unwrap_or("popularity.desc").to_string()),
        ("page", page.clamp(1, 500).to_string()),
        ("include_adult", "false".to_string()),
        ("include_video", "true".to_string()),
        ("language", locale(q)),
    ];
    if let Some(l) = given(q, "language").filter(|l| *l != "all") {
        out.push(("with_original_language", l.to_string()));
    }
    date_range(
        &mut out,
        "primary_release_date.gte",
        "primary_release_date.lte",
        given(q, "primaryReleaseDateGte"),
        given(q, "primaryReleaseDateLte"),
    );
    if let Some(v) = given(q, "studio") {
        out.push(("with_companies", v.to_string()));
    }
    for (ours, theirs) in SHARED.iter().chain(
        [
            ("certification", "certification"),
            ("certificationGte", "certification.gte"),
            ("certificationLte", "certification.lte"),
            ("certificationCountry", "certification_country"),
        ]
        .iter(),
    ) {
        if let Some(v) = given(q, ours) {
            out.push((theirs, v.to_string()));
        }
    }
    out
}

/// TMDB's `/discover/tv` query for Seerr's filters (`getDiscoverTv`).
pub fn tv_query(q: &Filters) -> Vec<(&'static str, String)> {
    let page: i64 = given(q, "page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let mut out = vec![
        ("sort_by", given(q, "sortBy").unwrap_or("popularity.desc").to_string()),
        ("page", page.clamp(1, 500).to_string()),
        ("language", locale(q)),
        ("include_null_first_air_dates", "false".to_string()),
    ];
    if let Some(l) = given(q, "language").filter(|l| *l != "all") {
        out.push(("with_original_language", l.to_string()));
    }
    date_range(&mut out, "first_air_date.gte", "first_air_date.lte", given(q, "firstAirDateGte"), given(q, "firstAirDateLte"));
    for (ours, theirs) in SHARED.iter().chain(
        [
            ("network", "with_networks"),
            ("status", "with_status"),
            ("certification", "certification"),
            ("certificationGte", "certification.gte"),
            ("certificationLte", "certification.lte"),
            ("certificationCountry", "certification_country"),
        ]
        .iter(),
    ) {
        if let Some(v) = given(q, ours) {
            out.push((theirs, v.to_string()));
        }
    }
    out
}

/// TMDB's genres of `kind` (`movie` or `tv`), `[{id, name}]`.
pub async fn genres(st: &AppState, kind: &str, language: &str) -> Result<Vec<Value>> {
    let v = crate::movies::tmdb(st).get(&format!("/genre/{kind}/list"), &[("language", language.to_string())]).await?;
    Ok(v["genres"].as_array().cloned().unwrap_or_default())
}

/// Seerr's genre slider: each genre of `kind` with the backdrops of its most popular titles, by name.
pub async fn genre_slider(st: &AppState, kind: &str, language: &str) -> Result<Value> {
    let t = crate::movies::tmdb(st);
    let mut out = Vec::new();
    for g in genres(st, kind, language).await? {
        let id = g["id"].as_i64().unwrap_or(0);
        let mut q = vec![("sort_by", "popularity.desc".to_string()), ("page", "1".to_string()), ("with_genres", id.to_string())];
        if kind == "movie" {
            q.push(("include_adult", "false".to_string()));
        }
        let page = t.get(&format!("/discover/{kind}"), &q).await?;
        let backdrops: Vec<Value> =
            page["results"].as_array().into_iter().flatten().filter_map(|r| r["backdrop_path"].as_str()).map(|b| json!(b)).collect();
        out.push(json!({ "id": id, "name": g["name"], "backdrops": backdrops }));
    }
    out.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
    Ok(json!(out))
}

/// The routes.
pub fn router() -> Router<AppState> {
    let v1 = Router::new()
        .route("/discover/movies", get(movies))
        .route("/discover/movies/language/{language}", get(movies_in_language))
        .route("/discover/movies/genre/{id}", get(movies_of_genre))
        .route("/discover/movies/studio/{id}", get(movies_of_studio))
        .route("/discover/movies/upcoming", get(upcoming_movies))
        .route("/discover/tv", get(series))
        .route("/discover/tv/language/{language}", get(series_in_language))
        .route("/discover/tv/genre/{id}", get(series_of_genre))
        .route("/discover/tv/network/{id}", get(series_of_network))
        .route("/discover/tv/upcoming", get(upcoming_series))
        .route("/discover/trending", get(trending))
        .route("/discover/keyword/{id}/movies", get(movies_with_keyword))
        .route("/discover/genreslider/movie", get(movie_genre_slider))
        .route("/discover/genreslider/tv", get(series_genre_slider))
        .route("/discover/watchlist", get(watchlist));
    Router::new().nest("/api/v1", v1)
}

/// Seerr's words when TMDB can't be reached for a route; what TMDB said goes to the log.
fn unable(what: &'static str) -> impl FnOnce(kit_server_core::Error) -> Fail {
    move |e| {
        tracing::warn!("discover: {what}: {}", e.detail);
        Fail(err(500, what))
    }
}

/// A Seerr page with `extra` fields (the language, genre, studio… the page is about) added.
fn with(mut page: Value, extra: Value) -> Answer {
    if let (Some(o), Some(x)) = (page.as_object_mut(), extra.as_object()) {
        for (k, v) in x {
            o.insert(k.clone(), v.clone());
        }
    }
    Ok(Json(page).into_response())
}

/// The filters for a sub-page (language, genre, studio, network): its page, the answer's language (`language`,
/// else English) and the one filter.
fn narrowed(q: &Filters, key: &str, value: &str) -> Filters {
    let mut f = Filters::new();
    if let Some(p) = given(q, "page") {
        f.insert("page".into(), p.into());
    }
    f.insert("locale".into(), given(q, "language").unwrap_or("en").into());
    f.insert(key.into(), value.into());
    f
}

/// TMDB's keywords for `ids` (comma separated), leaving out any TMDB doesn't know.
async fn keywords(st: &AppState, ids: Option<&str>) -> Vec<Value> {
    let mut out = Vec::new();
    for id in ids.unwrap_or("").split(',').map(str::trim).filter(|i| !i.is_empty()) {
        if let Ok(k) = crate::movies::tmdb(st).get(&format!("/keyword/{id}"), &[]).await {
            out.push(k);
        }
    }
    out
}

/// TMDB's language `code` (`iso_639_1`), or Seerr's 404.
async fn language(st: &AppState, code: &str) -> std::result::Result<Value, Fail> {
    let all = crate::movies::tmdb(st).get("/configuration/languages", &[]).await.map_err(unable("Unable to retrieve languages."))?;
    all.as_array()
        .into_iter()
        .flatten()
        .find(|l| l["iso_639_1"].as_str() == Some(code))
        .cloned()
        .ok_or_else(|| Fail(err(404, "Language not found.")))
}

/// Genre `id` of `kind`, or Seerr's 404.
async fn genre(st: &AppState, kind: &str, id: &str, lang: &str) -> std::result::Result<Value, Fail> {
    let all = genres(st, kind, lang).await.map_err(unable("Unable to retrieve genres."))?;
    all.into_iter()
        .find(|g| g["id"].as_i64().map(|n| n.to_string()).as_deref() == Some(id))
        .ok_or_else(|| Fail(err(404, "Genre not found.")))
}

/// A studio or network as Seerr maps it (`mapProductionCompany`, `mapNetwork`).
fn company(v: &Value) -> Value {
    json!({
        "id": v["id"],
        "name": v["name"],
        "originCountry": v["origin_country"],
        "description": v["description"],
        "headquarters": v["headquarters"],
        "homepage": v["homepage"],
        "logoPath": v["logo_path"],
    })
}

/// `GET /discover/movies`.
async fn movies(State(st): State<AppState>, _s: Seerr, Query(q): Query<Filters>) -> Answer {
    let page =
        tmdb_page(&st, "/discover/movie", &movie_query(&q), Some("movie")).await.map_err(unable("Unable to retrieve popular movies."))?;
    with(page, json!({ "keywords": keywords(&st, given(&q, "keywords")).await }))
}

/// `GET /discover/movies/language/{language}`.
async fn movies_in_language(State(st): State<AppState>, _s: Seerr, Path(code): Path<String>, Query(q): Query<Filters>) -> Answer {
    let lang = language(&st, &code).await?;
    let f = narrowed(&q, "language", &code);
    let page = tmdb_page(&st, "/discover/movie", &movie_query(&f), Some("movie"))
        .await
        .map_err(unable("Unable to retrieve movies by language."))?;
    with(page, json!({ "language": lang }))
}

/// `GET /discover/movies/genre/{id}`.
async fn movies_of_genre(State(st): State<AppState>, _s: Seerr, Path(id): Path<String>, Query(q): Query<Filters>) -> Answer {
    let f = narrowed(&q, "genre", &id);
    let g = genre(&st, "movie", &id, &locale(&f)).await?;
    let page =
        tmdb_page(&st, "/discover/movie", &movie_query(&f), Some("movie")).await.map_err(unable("Unable to retrieve movies by genre."))?;
    with(page, json!({ "genre": g }))
}

/// `GET /discover/movies/studio/{id}`.
async fn movies_of_studio(State(st): State<AppState>, _s: Seerr, Path(id): Path<i64>, Query(q): Query<Filters>) -> Answer {
    let failed = "Unable to retrieve movies by studio.";
    let studio = crate::movies::tmdb(&st).get(&format!("/company/{id}"), &[]).await.map_err(unable(failed))?;
    let f = narrowed(&q, "studio", &id.to_string());
    let page = tmdb_page(&st, "/discover/movie", &movie_query(&f), Some("movie")).await.map_err(unable(failed))?;
    with(page, json!({ "studio": company(&studio) }))
}

/// `GET /discover/movies/upcoming`: from today to 18 months ahead.
async fn upcoming_movies(State(st): State<AppState>, _s: Seerr, Query(q): Query<Filters>) -> Answer {
    let f = narrowed(&q, "primaryReleaseDateGte", &today());
    let page =
        tmdb_page(&st, "/discover/movie", &movie_query(&f), Some("movie")).await.map_err(unable("Unable to retrieve upcoming movies."))?;
    with(page, json!({}))
}

/// `GET /discover/tv`.
async fn series(State(st): State<AppState>, _s: Seerr, Query(q): Query<Filters>) -> Answer {
    let page = tmdb_page(&st, "/discover/tv", &tv_query(&q), Some("tv")).await.map_err(unable("Unable to retrieve popular series."))?;
    with(page, json!({ "keywords": keywords(&st, given(&q, "keywords")).await }))
}

/// `GET /discover/tv/language/{language}`.
async fn series_in_language(State(st): State<AppState>, _s: Seerr, Path(code): Path<String>, Query(q): Query<Filters>) -> Answer {
    let lang = language(&st, &code).await?;
    let f = narrowed(&q, "language", &code);
    let page = tmdb_page(&st, "/discover/tv", &tv_query(&f), Some("tv")).await.map_err(unable("Unable to retrieve series by language."))?;
    with(page, json!({ "language": lang }))
}

/// `GET /discover/tv/genre/{id}`.
async fn series_of_genre(State(st): State<AppState>, _s: Seerr, Path(id): Path<String>, Query(q): Query<Filters>) -> Answer {
    let f = narrowed(&q, "genre", &id);
    let g = genre(&st, "tv", &id, &locale(&f)).await?;
    let page = tmdb_page(&st, "/discover/tv", &tv_query(&f), Some("tv")).await.map_err(unable("Unable to retrieve series by genre."))?;
    with(page, json!({ "genre": g }))
}

/// `GET /discover/tv/network/{id}`.
async fn series_of_network(State(st): State<AppState>, _s: Seerr, Path(id): Path<i64>, Query(q): Query<Filters>) -> Answer {
    let failed = "Unable to retrieve series by network.";
    let network = crate::movies::tmdb(&st).get(&format!("/network/{id}"), &[]).await.map_err(unable(failed))?;
    let f = narrowed(&q, "network", &id.to_string());
    let page = tmdb_page(&st, "/discover/tv", &tv_query(&f), Some("tv")).await.map_err(unable(failed))?;
    with(page, json!({ "network": company(&network) }))
}

/// `GET /discover/tv/upcoming`: from today to 18 months ahead.
async fn upcoming_series(State(st): State<AppState>, _s: Seerr, Query(q): Query<Filters>) -> Answer {
    let f = narrowed(&q, "firstAirDateGte", &today());
    let page = tmdb_page(&st, "/discover/tv", &tv_query(&f), Some("tv")).await.map_err(unable("Unable to retrieve upcoming series."))?;
    with(page, json!({}))
}

/// `GET /discover/trending?mediaType=all|movie|tv&timeWindow=day|week`.
async fn trending(State(st): State<AppState>, _s: Seerr, Query(q): Query<Filters>) -> Answer {
    let kind = match given(&q, "mediaType") {
        Some("movie") => Some("movie"),
        Some("tv") => Some("tv"),
        _ => None,
    };
    let window = if given(&q, "timeWindow") == Some("week") { "week" } else { "day" };
    let page = given(&q, "page").and_then(|p| p.parse::<i64>().ok());
    let t = base_query(page, Some(given(&q, "language").unwrap_or("en")));
    let path = format!("/trending/{}/{window}", kind.unwrap_or("all"));
    with(tmdb_page(&st, &path, &t, kind).await.map_err(unable("Unable to retrieve trending items."))?, json!({}))
}

/// `GET /discover/keyword/{id}/movies`.
async fn movies_with_keyword(State(st): State<AppState>, _s: Seerr, Path(id): Path<i64>, Query(q): Query<Filters>) -> Answer {
    let f = narrowed(&q, "keywords", &id.to_string());
    let page = tmdb_page(&st, "/discover/movie", &movie_query(&f), Some("movie"))
        .await
        .map_err(unable("Unable to retrieve movies by keyword."))?;
    with(page, json!({}))
}

/// `GET /discover/genreslider/movie`.
async fn movie_genre_slider(State(st): State<AppState>, _s: Seerr, Query(q): Query<Filters>) -> Answer {
    let lang = given(&q, "language").unwrap_or("en").to_string();
    Ok(Json(genre_slider(&st, "movie", &lang).await.map_err(unable("Unable to retrieve movie genre slider."))?).into_response())
}

/// `GET /discover/genreslider/tv`.
async fn series_genre_slider(State(st): State<AppState>, _s: Seerr, Query(q): Query<Filters>) -> Answer {
    let lang = given(&q, "language").unwrap_or("en").to_string();
    Ok(Json(genre_slider(&st, "tv", &lang).await.map_err(unable("Unable to retrieve series genre slider."))?).into_response())
}

/// `GET /discover/watchlist`: Seerr's answer for somebody with no Plex watchlist (watchlist sync isn't ported yet).
async fn watchlist(_s: Seerr) -> Answer {
    Ok(Json(json!({ "page": 1, "totalPages": 1, "totalResults": 0, "results": [] })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmdb_names_become_seerrs() {
        assert_eq!(camel_key("poster_path"), "posterPath");
        assert_eq!(camel_key("genre_ids"), "genreIds");
        assert_eq!(camel_key("id"), "id");
        let v = camel(&json!({ "first_air_date": "2022-02-17", "origin_country": ["US"], "seasons": [{ "season_number": 1 }],
            "watch/providers": {} }));
        assert_eq!(
            v,
            json!({ "firstAirDate": "2022-02-17", "originCountry": ["US"], "seasons": [{ "seasonNumber": 1 }],
            "watchProviders": {} })
        );
        assert_eq!(base_query(None, Some("en")), vec![("page", "1".into()), ("include_adult", "false".into()), ("language", "en".into())]);
        let mut q = Filters::new();
        q.insert("primaryReleaseDateLte".into(), "2020-05-01T00:00:00.000Z".into());
        q.insert("language".into(), "ja".into());
        q.insert("studio".into(), "10342".into());
        let m = movie_query(&q);
        assert!(m.contains(&("primary_release_date.gte", "1900-01-01".into())), "TMDB needs both ends");
        assert!(m.contains(&("primary_release_date.lte", "2020-05-01".into())));
        assert!(m.contains(&("with_original_language", "ja".into())) && m.contains(&("with_companies", "10342".into())));
        assert!(m.contains(&("language", "en".into())), "the answer's language is English");
        let vids = videos(&json!({ "videos": { "results": [{ "site": "YouTube", "key": "abc", "type": "Trailer" }] } }));
        assert_eq!(vids[0]["url"], "https://www.youtube.com/watch?v=abc");
    }
}
