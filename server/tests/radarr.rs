//! The Radarr v3 API, checked against real Radarr 6.4 answers (`tests/contracts/radarr/`, recorded from
//! Radarr 6.4.4 with the same movies): every field Radarr sends, Marquee sends too, with the same kind of value.
//! TMDB is a stand-in server on this machine.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// The API key the tests use.
const KEY: &str = "radarr-test-key";

/// A recorded Radarr answer.
fn real(name: &str) -> Value {
    let path = format!("{}/tests/contracts/radarr/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap()
}

/// Fields Radarr sends only when it knows them (dates, links, IMDb ratings), not part of the check.
const OPTIONAL: [&str; 6] = ["ratings.imdb", "website", "youTubeTrailerId", "collection", "physicalRelease", "digitalRelease"];

/// Checks that `ours` has every field of `theirs` with the same kind of value (any value where theirs is null;
/// lists compared by their first item).
fn assert_shape(ours: &Value, theirs: &Value, at: &str) {
    match (ours, theirs) {
        (_, Value::Null) => {}
        (Value::Object(o), Value::Object(t)) => {
            for (k, tv) in t {
                let here = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                match o.get(k) {
                    Some(ov) => assert_shape(ov, tv, &here),
                    None if OPTIONAL.iter().any(|x| here.ends_with(x)) => {}
                    None => panic!("Marquee's answer has no `{here}` (Radarr sends {tv})"),
                }
            }
        }
        (Value::Array(o), Value::Array(t)) => {
            if let (Some(a), Some(b)) = (o.first(), t.first()) {
                assert_shape(a, b, &format!("{at}[0]"));
            }
        }
        (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) | (Value::Bool(_), Value::Bool(_)) => {}
        _ => panic!("`{at}` is {ours} in Marquee's answer but {theirs} in Radarr's"),
    }
}

/// TMDB's answer for The Matrix (`/movie/603` with release dates, videos and external ids).
fn matrix_details() -> Value {
    json!({
        "id": 603, "imdb_id": "tt0133093", "title": "The Matrix", "original_title": "The Matrix", "original_language": "en",
        "overview": "Set in the 22nd century, The Matrix tells the story of a computer hacker.",
        "release_date": "1999-03-30", "runtime": 136, "status": "Released", "homepage": "http://www.warnerbros.com/matrix",
        "genres": [{"id": 28, "name": "Action"}, {"id": 878, "name": "Science Fiction"}],
        "production_companies": [{"name": "Village Roadshow Pictures"}],
        "belongs_to_collection": {"id": 2344, "name": "The Matrix Collection"},
        "poster_path": "/f89U3ADr1oiB1s9GkdPOEpXUk5H.jpg", "backdrop_path": "/tlm8UkiQsitc8rSuIAscQDCnP8d.jpg",
        "vote_average": 8.2, "vote_count": 26000, "popularity": 85.5,
        "videos": {"results": [{"site": "YouTube", "type": "Trailer", "key": "vKQi3bBA1y8"}]},
        "release_dates": {"results": [{"iso_3166_1": "US", "release_dates": [
            {"type": 3, "release_date": "1999-03-30T00:00:00.000Z", "certification": "R"},
            {"type": 5, "release_date": "1999-09-21T00:00:00.000Z", "certification": ""},
            {"type": 4, "release_date": "2001-11-20T00:00:00.000Z", "certification": ""}
        ]}]}
    })
}

/// A TMDB stand-in on a free port; its address.
async fn fake_tmdb() -> String {
    let app = Router::new()
        .route(
            "/movie/{id}",
            get(|axum::extract::Path(id): axum::extract::Path<i64>| async move {
                if id == 603 {
                    (StatusCode::OK, axum::Json(matrix_details()))
                } else {
                    (StatusCode::NOT_FOUND, axum::Json(json!({ "status_code": 34 })))
                }
            }),
        )
        .route(
            "/search/movie",
            get(|| async {
                let mut m = matrix_details();
                m.as_object_mut().unwrap().retain(|k, _| {
                    [
                        "id",
                        "title",
                        "original_title",
                        "original_language",
                        "overview",
                        "release_date",
                        "poster_path",
                        "backdrop_path",
                        "vote_average",
                        "vote_count",
                        "popularity",
                    ]
                    .contains(&k.as_str())
                });
                axum::Json(json!({ "page": 1, "results": [m], "total_results": 1 }))
            }),
        )
        .route("/find/{imdb}", get(|| async { axum::Json(json!({ "movie_results": [{"id": 603}] })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A fresh server with the API key, a TMDB key, the stand-in TMDB and a movie root folder.
async fn server(name: &str) -> (Router, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("marquee-radarr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::create_dir_all(dir.join("media/movies")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let tmdb = fake_tmdb().await;
    let st = server::AppState::open(&dir.join("data"), web, move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TMDB_API_KEY" => Some("tmdb-test-key".into()),
        "MARQUEE_TMDB_URL" => Some(tmdb.clone()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    (server::app(st), dir)
}

/// One request to `/radarr/api/v3<path>` with the key; the status and the JSON body.
async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(format!("/radarr/api/v3{path}")).header("X-Api-Key", KEY);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Adds The Matrix the way Seerr and Radarr's own page do.
async fn add_matrix(app: &Router, root: &str) -> Value {
    let body = json!({ "title": "The Matrix", "tmdbId": 603, "year": 1999, "qualityProfileId": 4, "rootFolderPath": root,
        "monitored": true, "minimumAvailability": "released", "tags": [], "addOptions": { "searchForMovie": false } });
    let (s, v) = call(app, "POST", "/movie", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

#[tokio::test]
async fn the_key_is_needed_and_the_server_answers_like_radarr() {
    let (app, _) = server("status").await;
    let r = app.clone().oneshot(Request::get("/radarr/api/v3/movie").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r =
        app.clone().oneshot(Request::get(format!("/radarr/api/v3/system/status?apikey={KEY}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = app.clone().oneshot(Request::get("/radarr/ping").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let (s, v) = call(&app, "GET", "/system/status", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_shape(&v, &real("system_status"), "");
    assert_eq!(v["appName"], "Radarr");
    for (path, file) in [
        ("/health", "health"),
        ("/queue?page=1&pageSize=20", "queue"),
        ("/queue/status", "queue_status"),
        ("/queue/details", "queue_details"),
        ("/history?page=1&pageSize=20", "history"),
        ("/wanted/cutoff?page=1&pageSize=20", "wanted_cutoff"),
        ("/config/naming", "config_naming"),
        ("/language", "language"),
        ("/customformat", "customformat"),
        ("/notification", "notification"),
        ("/downloadclient", "downloadclient"),
        ("/importlist", "importlist"),
        ("/exclusions", "exclusions"),
    ] {
        let (s, v) = call(&app, "GET", path, None).await;
        assert_eq!(s, StatusCode::OK, "{path}");
        assert_shape(&v, &real(file), path);
    }
    assert_eq!(call(&app, "GET", "/queue/status", None).await.1, real("queue_status"));
    assert_eq!(call(&app, "GET", "/language", None).await.1, real("language"));
    let (_, ui) = call(&app, "GET", "/config/ui", None).await;
    assert_shape(&ui, &real("config_ui"), "config/ui");
}

#[tokio::test]
async fn qualities_profiles_folders_and_tags_have_radarrs_shape() {
    let (app, dir) = server("setup").await;
    let (_, defs) = call(&app, "GET", "/qualitydefinition", None).await;
    assert_eq!(defs, real("qualitydefinition"), "the standard quality list, as Radarr numbers it");
    let (_, profiles) = call(&app, "GET", "/qualityprofile", None).await;
    let names: Vec<&str> = profiles.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Any", "SD", "HD-720p", "HD-1080p", "Ultra-HD", "HD - 720p/1080p"]);
    let (_, p4) = call(&app, "GET", "/qualityprofile/4", None).await;
    assert_eq!(p4, real("qualityprofile_4"));

    let root = dir.join("media/movies").to_string_lossy().to_string();
    let (s, v) = call(&app, "POST", "/rootfolder", Some(json!({ "path": root }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_shape(&call(&app, "GET", "/rootfolder", None).await.1, &real("rootfolder"), "rootfolder");
    let (s, v) = call(&app, "POST", "/rootfolder", Some(json!({ "path": dir.join("nowhere") }))).await;
    assert_eq!((s, v[0]["errorMessage"].as_str()), (StatusCode::BAD_REQUEST, Some("Folder does not exist")));
    assert_shape(&call(&app, "GET", "/diskspace", None).await.1, &real("diskspace"), "diskspace");

    let (s, t) = call(&app, "POST", "/tag", Some(json!({ "label": "Kids" }))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(t, real("tag_post"));
    assert_eq!(call(&app, "GET", "/tag", None).await.1, real("tag"));
}

#[tokio::test]
async fn prowlarr_can_push_indexers() {
    let (app, _) = server("prowlarr").await;
    let (_, schema) = call(&app, "GET", "/indexer/schema", None).await;
    let torznab = schema.as_array().unwrap().iter().find(|s| s["implementation"] == "Torznab").cloned().expect("a Torznab template");
    let mut indexer = torznab.clone();
    indexer["name"] = json!("NZBgeek (Prowlarr)");
    for f in indexer["fields"].as_array_mut().unwrap() {
        match f["name"].as_str().unwrap() {
            "baseUrl" => f["value"] = json!("http://prowlarr:9696/1/"),
            "apiKey" => f["value"] = json!("prowlarrkey"),
            "categories" => f["value"] = json!([2000, 2040]),
            _ => {}
        }
    }
    let (s, _) = call(&app, "POST", "/indexer/test", Some(indexer.clone())).await;
    assert_eq!(s, StatusCode::OK);
    let (s, added) = call(&app, "POST", "/indexer?forceSave=true", Some(indexer.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{added}");
    let (_, list) = call(&app, "GET", "/indexer", None).await;
    assert_shape(&list, &real("indexer"), "indexer");
    let key = |v: &Value| v["fields"].as_array().unwrap().iter().find(|f| f["name"] == "apiKey").unwrap()["value"].clone();
    assert_eq!(key(&list[0]), json!("********"), "a saved key is never sent back");
    // Prowlarr sends the masked key back on an update: the saved one stays
    let id = list[0]["id"].as_i64().unwrap();
    let mut changed = list[0].clone();
    changed["priority"] = json!(10);
    let (s, v) = call(&app, "PUT", &format!("/indexer/{id}"), Some(changed)).await;
    assert_eq!((s, v["priority"].as_i64()), (StatusCode::ACCEPTED, Some(10)));
    let (s, v) = call(&app, "POST", "/indexer", Some(indexer)).await;
    assert_eq!((s, v[0]["errorMessage"].as_str()), (StatusCode::BAD_REQUEST, Some("Should be unique")));
    assert_eq!(call(&app, "DELETE", &format!("/indexer/{id}"), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/indexer", None).await.1, json!([]));
}

#[tokio::test]
async fn movies_are_looked_up_added_changed_and_listed_like_radarr() {
    let (app, dir) = server("movies").await;
    let root = dir.join("media/movies").to_string_lossy().to_string();
    call(&app, "POST", "/rootfolder", Some(json!({ "path": root }))).await;

    let (s, found) = call(&app, "GET", "/movie/lookup?term=The%20Matrix", None).await;
    assert_eq!(s, StatusCode::OK, "{found}");
    assert_eq!(found[0]["tmdbId"], 603);
    assert_eq!(found[0]["folder"], "The Matrix (1999)");
    let (_, one) = call(&app, "GET", "/movie/lookup/tmdb?tmdbId=603", None).await;
    assert_eq!((one["imdbId"].as_str(), one["certification"].as_str()), (Some("tt0133093"), Some("R")));

    let added = add_matrix(&app, &root).await;
    let recorded = real("movie_1");
    assert_shape(&added, &recorded, "movie");
    for k in [
        "title",
        "year",
        "tmdbId",
        "imdbId",
        "status",
        "inCinemas",
        "physicalRelease",
        "digitalRelease",
        "releaseDate",
        "runtime",
        "certification",
        "genres",
        "isAvailable",
        "minimumAvailability",
        "sortTitle",
        "cleanTitle",
        "titleSlug",
        "studio",
        "collection",
        "originalLanguage",
        "youTubeTrailerId",
    ] {
        assert_eq!(added[k], recorded[k], "`{k}` matches Radarr's");
    }
    assert_eq!(added["path"], json!(format!("{root}/The Matrix (1999)")));

    let (s, dup) =
        call(&app, "POST", "/movie", Some(json!({ "tmdbId": 603, "qualityProfileId": 4, "rootFolderPath": root, "title": "The Matrix" })))
            .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(dup, real("movie_post_duplicate"));

    let (_, looked) = call(&app, "GET", "/movie/lookup?term=The%20Matrix", None).await;
    assert_eq!(looked[0]["id"], added["id"], "a lookup knows what is added");

    let (_, list) = call(&app, "GET", "/movie", None).await;
    assert_shape(&list, &real("movie"), "movie list");
    assert_eq!(call(&app, "GET", "/movie?tmdbId=603", None).await.1.as_array().unwrap().len(), 1);
    assert_eq!(call(&app, "GET", "/movie?tmdbId=1", None).await.1, json!([]));
    assert_eq!(call(&app, "GET", "/movie/99", None).await.0, StatusCode::NOT_FOUND);

    let mut edit = added.clone();
    edit["monitored"] = json!(false);
    let (s, v) = call(&app, "PUT", &format!("/movie/{}", added["id"]), Some(edit)).await;
    assert_eq!((s, v["monitored"].as_bool()), (StatusCode::ACCEPTED, Some(false)));
    assert_shape(&v, &real("movie_put"), "movie put");
    let (_, v) = call(&app, "PUT", "/movie/editor", Some(json!({ "movieIds": [added["id"]], "monitored": true }))).await;
    assert_eq!(v[0]["monitored"], true);

    let (_, cal) = call(&app, "GET", "/calendar?start=1999-09-01&end=1999-09-30", None).await;
    assert_eq!(cal.as_array().unwrap().len(), 1, "the disc release is in September 1999");
    assert_shape(&cal, &real("calendar"), "calendar");
    assert_eq!(call(&app, "GET", "/calendar?start=2026-11-01&end=2026-12-31", None).await.1, json!([]));
    let (_, missing) = call(&app, "GET", "/wanted/missing?page=1&pageSize=20&sortKey=title&sortDirection=ascending", None).await;
    assert_shape(&missing, &real("wanted_missing"), "wanted/missing");
    assert_eq!(missing["totalRecords"], 1);

    let (s, cmd) = call(&app, "POST", "/command", Some(json!({ "name": "RefreshMovie", "movieIds": [added["id"]] }))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_shape(&cmd, &real("command_post"), "command");
    let (s, _) = call(&app, "GET", &format!("/command/{}", cmd["id"]), None).await;
    assert_eq!(s, StatusCode::OK);

    assert_eq!(call(&app, "DELETE", &format!("/movie/{}", added["id"]), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/movie", None).await.1, json!([]));
    assert!(dir.join("media/movies").is_dir());
}

#[tokio::test]
async fn the_web_apps_movie_api_adds_from_tmdb() {
    let (app, dir) = server("native").await;
    let root = dir.join("media/movies").to_string_lossy().to_string();
    let send = |method: &str, path: &str, body: Option<Value>| {
        let b = Request::builder().method(method).uri(path).header("x-api-key", KEY);
        match body {
            Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    };
    let run = |req: Request<Body>| {
        let app = app.clone();
        async move {
            let r = app.oneshot(req).await.unwrap();
            let s = r.status();
            let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
            (s, serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null))
        }
    };
    let (s, v) = run(send("POST", "/api/movies", Some(json!({ "tmdb_id": 603 })))).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Add a movie folder in Settings → Folders first")));
    run(send("POST", "/api/root-folders", Some(json!({ "kind": "movie", "path": root })))).await;
    let (s, v) = run(send("POST", "/api/movies", Some(json!({ "tmdb_id": 603 })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["title"].as_str(), v["status"].as_str(), v["available"].as_bool()), (Some("The Matrix"), Some("released"), Some(true)));
    let (_, profiles) = run(send("GET", "/api/quality-profiles?kind=movie", None)).await;
    let hd = profiles.as_array().unwrap().iter().find(|p| p["name"] == "HD-1080p").unwrap()["id"].clone();
    assert_eq!(v["quality_profile_id"], hd, "a new movie gets HD-1080p");
    let (s, _) = run(send("POST", "/api/movies", Some(json!({ "tmdb_id": 603 })))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, found) = run(send("GET", "/api/movies/lookup?term=tt0133093", None)).await;
    assert_eq!(found[0]["id"], v["id"]);
    let (s, _) = run(send("POST", "/api/movies", Some(json!({ "tmdb_id": 5 })))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
