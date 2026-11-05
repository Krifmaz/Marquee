//! Moving in from Radarr and Sonarr, end to end: stand-ins answer with real Radarr 6.4 and Sonarr 4.0 answers
//! (tests/contracts), a dry run changes nothing, the real run adds the movies and the series with their files
//! (kept where they are, through a path rewrite), and a second run skips what is there.

use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use tower::ServiceExt;

/// Marquee's API key in the tests.
const KEY: &str = "migrate-test-key";
/// The other apps' API key.
const THEIRS: &str = "their-key";

/// A recorded answer from tests/contracts.
fn contract(app: &str, name: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/contracts/{app}/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// Answers `v` when the key is right, else 401 like Radarr and Sonarr.
fn keyed(h: &HeaderMap, v: Value) -> (StatusCode, Json<Value>) {
    if h.get("x-api-key").and_then(|k| k.to_str().ok()) == Some(THEIRS) {
        (StatusCode::OK, Json(v))
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({ "message": "Unauthorized" })))
    }
}

/// Starts `app` on a free port; its address.
async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A Radarr stand-in: its movies (The Matrix with a file) and settings.
async fn fake_radarr() -> String {
    let mut movies = contract("radarr", "movie");
    movies[0]["hasFile"] = json!(true);
    movies[0]["movieFile"] = json!({ "id": 1, "movieId": 1, "path": "/movies/The Matrix (1999)/The Matrix (1999) Bluray-1080p.mkv",
        "size": 4096, "quality": { "quality": { "id": 7, "name": "Bluray-1080p" } }, "releaseGroup": "SPARKS",
        "sceneName": "The.Matrix.1999.1080p.BluRay.x264-SPARKS" });
    let app = Router::new()
        .route(
            "/api/v3/movie",
            get(move |h: HeaderMap| {
                let m = movies.clone();
                async move { keyed(&h, m) }
            }),
        )
        .route("/api/v3/qualityprofile", get(|h: HeaderMap| async move { keyed(&h, contract("radarr", "qualityprofile")) }))
        .route("/api/v3/tag", get(|h: HeaderMap| async move { keyed(&h, json!([{ "id": 1, "label": "kids" }])) }))
        .route("/api/v3/rootfolder", get(|h: HeaderMap| async move { keyed(&h, contract("radarr", "rootfolder")) }));
    serve(app).await
}

/// A Sonarr stand-in: The Bear, with a file for S01E01.
async fn fake_sonarr() -> String {
    let app = Router::new()
        .route("/api/v3/series", get(|h: HeaderMap| async move { keyed(&h, contract("sonarr", "series")) }))
        .route("/api/v3/qualityprofile", get(|h: HeaderMap| async move { keyed(&h, contract("sonarr", "qualityprofile")) }))
        .route("/api/v3/tag", get(|h: HeaderMap| async move { keyed(&h, contract("sonarr", "tag")) }))
        .route("/api/v3/rootfolder", get(|h: HeaderMap| async move { keyed(&h, contract("sonarr", "rootfolder")) }))
        .route(
            "/api/v3/episodefile",
            get(|h: HeaderMap| async move {
                keyed(&h, json!([{ "id": 7, "seriesId": 1, "seasonNumber": 1, "path": "/tv/The Bear/Season 1/The Bear - S01E01 - System WEBDL-1080p.mkv",
                    "size": 2048, "quality": { "quality": { "id": 3, "name": "WEBDL-1080p" } }, "releaseGroup": "NTb", "sceneName": "" }]))
            }),
        )
        .route(
            "/api/v3/episode",
            get(|h: HeaderMap| async move {
                let mut eps = contract("sonarr", "episode");
                eps[1]["episodeFileId"] = json!(7);
                eps[1]["hasFile"] = json!(true);
                keyed(&h, eps)
            }),
        );
    serve(app).await
}

/// A TMDB stand-in: movies can't be asked (Marquee keeps Radarr's details), The Bear has seasons 0 to 2.
async fn fake_tmdb() -> String {
    let app = Router::new()
        .route("/movie/{id}", get(|| async { (StatusCode::SERVICE_UNAVAILABLE, Json(json!({}))) }))
        .route(
            "/tv/{id}",
            get(|| async {
                Json(json!({ "id": 136315, "name": "The Bear", "original_name": "The Bear", "first_air_date": "2022-06-23",
                    "episode_run_time": [30], "seasons": [{"season_number": 0}, {"season_number": 1}, {"season_number": 2}],
                    "external_ids": {"tvdb_id": 403245}, "status": "Returning Series" }))
            }),
        )
        .route(
            "/tv/{id}/season/{n}",
            get(|Path((_, n)): Path<(i64, i64)>, Query(_q): Query<HashMap<String, String>>| async move {
                let count = if n == 0 { 1 } else { 2 };
                Json(json!({ "episodes": (1..=count).map(|e| json!({ "id": n * 10 + e, "season_number": n, "episode_number": e,
                    "name": format!("Episode {e}"), "air_date": "2022-06-23", "runtime": 30 })).collect::<Vec<_>>() }))
            }),
        )
        .route("/find/{id}", get(|| async { Json(json!({ "tv_results": [{"id": 136315}] })) }));
    serve(app).await
}

/// One request with the API key; the status and the JSON body.
async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(path).header("X-Api-Key", KEY);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn radarr_and_sonarr_move_in_with_their_files() {
    let dir = std::env::temp_dir().join(format!("marquee-migrate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("static")).unwrap();
    std::fs::write(dir.join("static/index.html"), "<!doctype html>").unwrap();
    let (movies, tv) = (dir.join("media/movies"), dir.join("media/tv"));
    std::fs::create_dir_all(movies.join("The Matrix (1999)")).unwrap();
    std::fs::create_dir_all(tv.join("The Bear/Season 1")).unwrap();
    std::fs::write(movies.join("The Matrix (1999)/The Matrix (1999) Bluray-1080p.mkv"), vec![1u8; 4096]).unwrap();
    std::fs::write(tv.join("The Bear/Season 1/The Bear - S01E01 - System WEBDL-1080p.mkv"), vec![1u8; 2048]).unwrap();
    let (radarr, sonarr, tmdb) = (fake_radarr().await, fake_sonarr().await, fake_tmdb().await);
    let st = server::AppState::open(&dir.join("data"), dir.join("static"), move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TMDB_API_KEY" => Some("tmdb".into()),
        "MARQUEE_TMDB_URL" => Some(tmdb.clone()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        "MARQUEE_ANIME_LIST_URL" => Some("off".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    let app = server::app(st);
    let (m, t) = (movies.to_string_lossy().to_string(), tv.to_string_lossy().to_string());
    let maps = json!([{ "from": "/data/media/movies", "to": m }, { "from": "/movies", "to": m }, { "from": "/data/media/tv", "to": t }, { "from": "/tv", "to": t }]);

    let (s, v) = call(&app, "POST", "/api/migrate/radarr", Some(json!({ "url": radarr, "api_key": "wrong" }))).await;
    assert_eq!(
        (s, v["detail"].as_str()),
        (StatusCode::BAD_REQUEST, Some("Radarr turned the API key down. Copy it from Radarr → Settings → General."))
    );

    // a dry run says what would happen and changes nothing
    let (s, dry) = call(&app, "POST", "/api/migrate/radarr", Some(json!({ "url": radarr, "api_key": THEIRS, "path_map": maps }))).await;
    assert_eq!(s, StatusCode::OK, "{dry}");
    assert_eq!((dry["applied"].as_bool(), dry["added"].as_i64(), dry["files"].as_i64()), (Some(false), Some(2), Some(1)), "{dry}");
    assert!(dry["notes"].to_string().contains("will be added"), "{dry}");
    assert_eq!(call(&app, "GET", "/radarr/api/v3/movie", None).await.1, json!([]));

    let (s, done) =
        call(&app, "POST", "/api/migrate/radarr", Some(json!({ "url": radarr, "api_key": THEIRS, "apply": true, "path_map": maps }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!((done["added"].as_i64(), done["problems"].as_i64()), (Some(2), Some(0)), "{done}");
    let (_, list) = call(&app, "GET", "/radarr/api/v3/movie", None).await;
    let matrix = list.as_array().unwrap().iter().find(|x| x["tmdbId"] == 603).unwrap();
    assert_eq!(
        (matrix["hasFile"].as_bool(), matrix["path"].as_str()),
        (Some(true), Some(format!("{m}/The Matrix (1999)").as_str())),
        "{matrix}"
    );
    assert!(matrix["movieFileId"].as_i64().unwrap_or(0) > 0, "{matrix}");
    let (_, tags) = call(&app, "GET", "/radarr/api/v3/tag", None).await;
    assert_eq!(tags[0]["label"], "kids");
    let (_, again) =
        call(&app, "POST", "/api/migrate/radarr", Some(json!({ "url": radarr, "api_key": THEIRS, "apply": true, "path_map": maps }))).await;
    assert_eq!((again["added"].as_i64(), again["skipped"].as_i64()), (Some(0), Some(2)), "{again}");

    let (s, done) =
        call(&app, "POST", "/api/migrate/sonarr", Some(json!({ "url": sonarr, "api_key": THEIRS, "apply": true, "path_map": maps }))).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!((done["added"].as_i64(), done["files"].as_i64()), (Some(1), Some(1)), "{done}");
    let (_, series) = call(&app, "GET", "/sonarr/api/v3/series", None).await;
    let id = series[0]["id"].as_i64().unwrap();
    assert_eq!(series[0]["path"], format!("{t}/The Bear"));
    let (_, eps) = call(&app, "GET", &format!("/sonarr/api/v3/episode?seriesId={id}"), None).await;
    let ep = |s: i64, e: i64| eps.as_array().unwrap().iter().find(|x| x["seasonNumber"] == s && x["episodeNumber"] == e).unwrap().clone();
    assert_eq!(ep(1, 1)["hasFile"], true, "{eps}");
    assert_eq!(ep(1, 2)["hasFile"], false);
    assert_eq!(ep(0, 1)["monitored"], false, "specials stay unmonitored like in Sonarr");
    assert_eq!(ep(2, 1)["monitored"], true);
}
