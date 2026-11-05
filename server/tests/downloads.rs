//! Downloads through Lantern, end to end: a movie and a TV season are searched for in a stand-in Lantern, the best
//! releases are grabbed, show in the Radarr and Sonarr queues, and once Lantern says they are done (by calling
//! back, or when asked) the files are hardlinked into the library under Radarr's and Sonarr's names.

use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// Marquee's API key in the tests.
const KEY: &str = "downloads-test-key";

/// What the stand-in Lantern was asked: the grabs (as it answers them) and the downloads it was told are imported.
#[derive(Default)]
struct Hub {
    /// every grab, as Lantern's hub keeps it
    grabs: Vec<Value>,
    /// Lantern ids Marquee said are imported
    imported: Vec<String>,
    /// the search queries
    searches: Vec<HashMap<String, String>>,
}

/// A release as Lantern's Prowlarr API sends it.
fn rel(title: &str, gb: f64, seeders: i64) -> Value {
    json!({ "guid": format!("guid-{title}"), "title": title, "size": (gb * 1_073_741_824.0) as i64, "protocol": "torrent",
        "indexer": "Tracker", "indexerId": 1, "seeders": seeders, "leechers": 1, "age": 3,
        "downloadUrl": format!("http://lantern:9696/1/download?link={title}") })
}

/// A Lantern stand-in on a free port; its address and what it was asked.
async fn fake_lantern() -> (String, Arc<Mutex<Hub>>) {
    let hub = Arc::new(Mutex::new(Hub::default()));
    let (h1, h2, h3, h4) = (hub.clone(), hub.clone(), hub.clone(), hub.clone());
    let app = Router::new()
        .route(
            "/api/v1/search",
            get(move |Query(q): Query<HashMap<String, String>>| {
                let hub = h1.clone();
                async move {
                    hub.lock().unwrap().searches.push(q.clone());
                    let list = match q.get("type").map(String::as_str) {
                        Some("movie") => vec![
                            rel("The.Matrix.1999.720p.HDTV.x264-GRP", 4.0, 10),
                            rel("The.Matrix.1999.1080p.BluRay.x264-SPARKS", 10.0, 50),
                            rel("The.Matrix.Reloaded.2003.1080p.BluRay.x264-GRP", 10.0, 80),
                        ],
                        Some("tvsearch") if q.get("ep").is_none() => vec![rel("The.Bear.S01.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb", 2.0, 30)],
                        Some("tvsearch") => vec![rel("The.Bear.S01E01.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb", 1.0, 30)],
                        _ => vec![],
                    };
                    Json(json!(list))
                }
            }),
        )
        .route("/api/v1/hub/apps", post(|Json(b): Json<Value>| async move { Json(json!({ "id": 1, "name": b["name"] })) }))
        .route(
            "/api/v1/hub/grabs",
            post(move |Json(b): Json<Value>| {
                let hub = h2.clone();
                async move {
                    let mut h = hub.lock().unwrap();
                    let g = json!({ "id": format!("g{}", h.grabs.len() + 1), "ref": b["ref"], "title": b["release"]["title"],
                        "status": "queued", "progress": 0.0, "size": b["release"]["size"], "left": b["release"]["size"], "files": [],
                        "protocol": "torrent", "kind": b["kind"], "app": b["app"] });
                    h.grabs.push(g.clone());
                    (StatusCode::CREATED, Json(g))
                }
            })
            .get(move || {
                let hub = h3.clone();
                async move { Json(json!(hub.lock().unwrap().grabs)) }
            }),
        )
        .route(
            "/api/v1/hub/grabs/{id}/imported",
            post(move |Path(id): Path<String>| {
                let hub = h4.clone();
                async move {
                    hub.lock().unwrap().imported.push(id);
                    Json(json!({ "ok": true }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), hub)
}

/// A TMDB stand-in with The Matrix and The Bear.
async fn fake_tmdb() -> String {
    let app = Router::new()
        .route(
            "/movie/{id}",
            get(|| async {
                Json(json!({ "id": 603, "imdb_id": "tt0133093", "title": "The Matrix", "original_title": "The Matrix", "release_date": "1999-03-30",
                    "runtime": 136, "status": "Released", "release_dates": {"results": []} }))
            }),
        )
        .route(
            "/tv/{id}",
            get(|| async {
                Json(json!({ "id": 136315, "name": "The Bear", "original_name": "The Bear", "first_air_date": "2022-06-23", "episode_run_time": [30],
                    "seasons": [{"season_number": 1}], "external_ids": {"tvdb_id": 403245}, "status": "Returning Series" }))
            }),
        )
        .route(
            "/tv/{id}/season/{n}",
            get(|| async {
                Json(json!({ "episodes": [
                    {"id": 1, "season_number": 1, "episode_number": 1, "name": "System", "air_date": "2022-06-23", "runtime": 30},
                    {"id": 2, "season_number": 1, "episode_number": 2, "name": "Hands", "air_date": "2022-06-23", "runtime": 30}
                ] }))
            }),
        )
        .route("/find/{id}", get(|| async { Json(json!({ "tv_results": [{"id": 136315}] })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A fresh server set up with the stand-in Lantern and TMDB, and movie and TV folders.
async fn server(name: &str) -> (Router, std::path::PathBuf, Arc<Mutex<Hub>>) {
    let dir = std::env::temp_dir().join(format!("marquee-dl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    for f in ["media/movies", "media/tv", "downloads"] {
        std::fs::create_dir_all(dir.join(f)).unwrap();
    }
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let (lantern, hub) = fake_lantern().await;
    let tmdb = fake_tmdb().await;
    let st = server::AppState::open(&dir.join("data"), web, move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TMDB_API_KEY" => Some("tmdb".into()),
        "MARQUEE_TMDB_URL" => Some(tmdb.clone()),
        "MARQUEE_LANTERN_URL" => Some(lantern.clone()),
        "MARQUEE_LANTERN_API_KEY" => Some("lantern-key".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    (server::app(st), dir, hub)
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

#[cfg(unix)]
fn same(a: &std::path::Path, b: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(a).unwrap().ino() == std::fs::metadata(b).unwrap().ino()
}

#[cfg(not(unix))]
fn same(_: &std::path::Path, b: &std::path::Path) -> bool {
    b.exists()
}

#[tokio::test]
async fn a_movie_is_searched_grabbed_and_hardlinked_into_its_folder() {
    let (app, dir, hub) = server("movie").await;
    let root = dir.join("media/movies").to_string_lossy().to_string();
    call(&app, "POST", "/radarr/api/v3/rootfolder", Some(json!({ "path": root }))).await;
    let (_, profiles) = call(&app, "GET", "/radarr/api/v3/qualityprofile", None).await;
    let hd = profiles.as_array().unwrap().iter().find(|p| p["name"] == "HD-1080p").unwrap()["id"].clone();
    let (s, m) = call(
        &app,
        "POST",
        "/radarr/api/v3/movie",
        Some(json!({ "tmdbId": 603, "title": "The Matrix", "qualityProfileId": hd, "rootFolderPath": root })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{m}");
    let id = m["id"].as_i64().unwrap();

    let (_, rels) = call(&app, "GET", &format!("/api/movies/{id}/releases"), None).await;
    assert_eq!(rels[0]["release"]["title"], "The.Matrix.1999.1080p.BluRay.x264-SPARKS");
    let reloaded = rels.as_array().unwrap().iter().find(|c| c["release"]["title"].as_str().unwrap().contains("Reloaded")).unwrap();
    assert!(reloaded["rejections"][0].as_str().unwrap().contains("not this movie"), "{rels}");

    let (s, out) = call(&app, "POST", &format!("/api/movies/{id}/search"), None).await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["grabbed"][0]["title"], "The.Matrix.1999.1080p.BluRay.x264-SPARKS");
    assert_eq!(out["grabbed"][0]["quality"], "Bluray-1080p");
    {
        let h = hub.lock().unwrap();
        assert_eq!((h.grabs.len(), h.grabs[0]["ref"].as_str()), (1, Some(format!("movie:{id}").as_str())));
        assert_eq!(h.searches[0].get("imdbid").map(String::as_str), Some("tt0133093"));
    }
    let (_, again) = call(&app, "POST", &format!("/api/movies/{id}/search"), None).await;
    assert_eq!(again["reason"], "The Matrix is downloading already");

    let (_, q) = call(&app, "GET", "/radarr/api/v3/queue", None).await;
    assert_eq!((q["totalRecords"].as_i64(), q["records"][0]["movieId"].as_i64()), (Some(1), Some(id)));
    assert_eq!(q["records"][0]["downloadClient"], "Lantern");
    assert_eq!(q["records"][0]["quality"]["quality"]["name"], "Bluray-1080p");
    assert_eq!(call(&app, "GET", "/radarr/api/v3/queue/status", None).await.1["totalCount"], 1);

    // Lantern finishes it and calls back
    let done = dir.join("downloads/The.Matrix.1999.1080p.BluRay.x264-SPARKS");
    std::fs::create_dir_all(&done).unwrap();
    std::fs::write(done.join("the.matrix.1999.1080p.bluray.x264-sparks.mkv"), vec![7u8; 4096]).unwrap();
    std::fs::write(done.join("the.matrix.1999.1080p.bluray.x264-sparks.en.srt"), "1\n").unwrap();
    std::fs::write(done.join("sample.mkv"), vec![7u8; 16]).unwrap();
    let event = json!({ "event": "grab.completed", "grab": { "id": "g1", "status": "completed", "progress": 1.0,
        "files": [done.to_string_lossy()] } });
    let r = app
        .clone()
        .oneshot(
            Request::post("/api/lantern/events")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "Lantern must send Marquee's key");
    let (s, v) = call(&app, "POST", "/api/lantern/events", Some(event)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let file = dir.join("media/movies/The Matrix (1999)/The Matrix (1999) Bluray-1080p.mkv");
    assert!(file.exists(), "the movie is in its folder");
    assert!(same(&done.join("the.matrix.1999.1080p.bluray.x264-sparks.mkv"), &file), "hardlinked, not copied");
    assert!(dir.join("media/movies/The Matrix (1999)/The Matrix (1999) Bluray-1080p.en.srt").exists(), "subtitles came along");
    assert!(!dir.join("media/movies/The Matrix (1999)/sample.mkv").exists());
    let (_, m) = call(&app, "GET", &format!("/radarr/api/v3/movie/{id}"), None).await;
    assert_eq!(m["hasFile"], true);
    assert_eq!(call(&app, "GET", "/radarr/api/v3/queue", None).await.1["totalRecords"], 0);
    assert_eq!(hub.lock().unwrap().imported, vec!["g1"]);
    let (_, dl) = call(&app, "GET", "/api/downloads", None).await;
    assert_eq!(dl[0]["status"], "imported");

    // HD-1080p doesn't upgrade: nothing more is grabbed
    let (_, again) = call(&app, "POST", &format!("/api/movies/{id}/search"), None).await;
    assert_eq!(again["grabbed"], json!([]));
    assert!(again["reason"].as_str().unwrap().contains("can be used"));
}

#[tokio::test]
async fn a_whole_season_is_grabbed_as_one_and_each_episode_imported() {
    let (app, dir, hub) = server("tv").await;
    let root = dir.join("media/tv").to_string_lossy().to_string();
    call(&app, "POST", "/sonarr/api/v3/rootfolder", Some(json!({ "path": root }))).await;
    let (_, profiles) = call(&app, "GET", "/sonarr/api/v3/qualityprofile", None).await;
    let hd = profiles.as_array().unwrap().iter().find(|p| p["name"] == "HD-1080p").unwrap()["id"].clone();
    let (s, series) = call(
        &app,
        "POST",
        "/sonarr/api/v3/series",
        Some(json!({ "tvdbId": 403245, "title": "The Bear", "qualityProfileId": hd,
        "rootFolderPath": root, "seasonFolder": true, "addOptions": { "monitor": "all" } })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{series}");
    let id = series["id"].as_i64().unwrap();

    let (s, out) = call(&app, "POST", &format!("/api/series/{id}/search"), Some(json!({ "season": 1 }))).await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["grabbed"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["grabbed"][0]["title"], "The.Bear.S01.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb");
    assert_eq!(out["grabbed"][0]["episode_ids"].as_array().unwrap().len(), 2);
    let (_, q) = call(&app, "GET", "/sonarr/api/v3/queue", None).await;
    assert_eq!(q["totalRecords"], 2, "one queue record per episode, like Sonarr");
    assert_eq!(q["records"][0]["seriesId"].as_i64(), Some(id));

    // Lantern doesn't call back this time: Marquee asks
    let done = dir.join("downloads/The.Bear.S01.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb");
    std::fs::create_dir_all(&done).unwrap();
    for e in ["S01E01", "S01E02"] {
        std::fs::write(done.join(format!("The.Bear.{e}.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb.mkv")), vec![1u8; 2048]).unwrap();
    }
    {
        let mut h = hub.lock().unwrap();
        h.grabs[0]["status"] = json!("completed");
        h.grabs[0]["progress"] = json!(1.0);
        h.grabs[0]["files"] = json!([done.to_string_lossy()]);
    }
    let (s, v) = call(&app, "POST", "/api/lantern/sync", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let season = dir.join("media/tv/The Bear/Season 1");
    assert!(season.join("The Bear - S01E01 - System WEBDL-1080p.mkv").exists());
    assert!(same(
        &done.join("The.Bear.S01E02.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb.mkv"),
        &season.join("The Bear - S01E02 - Hands WEBDL-1080p.mkv")
    ));
    let (_, eps) = call(&app, "GET", &format!("/sonarr/api/v3/episode?seriesId={id}"), None).await;
    assert!(eps.as_array().unwrap().iter().all(|e| e["hasFile"] == true), "{eps}");
    assert_eq!(call(&app, "GET", "/sonarr/api/v3/queue", None).await.1["totalRecords"], 0);
    let (_, out) = call(&app, "POST", &format!("/api/series/{id}/search"), None).await;
    assert_eq!(out["reason"], "No episode of The Bear is wanted right now");
}

#[tokio::test]
async fn without_lantern_a_search_says_what_to_do() {
    let dir = std::env::temp_dir().join(format!("marquee-dl-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("static")).unwrap();
    std::fs::write(dir.join("static/index.html"), "<!doctype html>").unwrap();
    let st =
        server::AppState::open(&dir.join("data"), dir.join("static"), |n| (n == "APP_API_KEY").then(|| KEY.to_string())).await.unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    let app = server::app(st);
    let (s, v) = call(&app, "POST", "/api/lantern/sync", None).await;
    assert_eq!(
        (s, v["detail"].as_str()),
        (StatusCode::BAD_REQUEST, Some("Add Lantern's address and API key in Settings → Connections first"))
    );
}
