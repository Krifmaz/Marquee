//! The Sonarr v3 API, checked against real Sonarr 4.0 answers (`tests/contracts/sonarr/`, recorded from
//! Sonarr 4.0.20 with the same series): every field Sonarr sends, Marquee sends too, with the same kind of value.
//! TMDB is a stand-in server on this machine.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// The API key the tests use.
const KEY: &str = "sonarr-test-key";

/// A recorded Sonarr answer.
fn real(name: &str) -> Value {
    let path = format!("{}/tests/contracts/sonarr/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap()
}

/// Fields Sonarr sends only when it knows them (TVmaze ids, finale dates), not part of the check.
const OPTIONAL: [&str; 3] = ["finaleType", "lastAired", "previousAiring"];

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
                    None => panic!("Marquee's answer has no `{here}` (Sonarr sends {tv})"),
                }
            }
        }
        (Value::Array(o), Value::Array(t)) => {
            if let (Some(a), Some(b)) = (o.first(), t.first()) {
                assert_shape(a, b, &format!("{at}[0]"));
            }
        }
        (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) | (Value::Bool(_), Value::Bool(_)) => {}
        _ => panic!("`{at}` is {ours} in Marquee's answer but {theirs} in Sonarr's"),
    }
}

/// TMDB's answer for The Bear (`/tv/136315` with external ids and content ratings).
fn bear_details() -> Value {
    json!({
        "id": 136315, "name": "The Bear", "original_name": "The Bear", "original_language": "en",
        "overview": "A young chef from the fine dining world returns to Chicago to run his family sandwich shop.",
        "first_air_date": "2022-06-23", "status": "Returning Series", "episode_run_time": [30],
        "networks": [{"name": "FX"}], "genres": [{"id": 35, "name": "Comedy"}, {"id": 18, "name": "Drama"}],
        "poster_path": "/sHFlbKS3WLqMnp9t2ghADIJFnuQ.jpg", "backdrop_path": "/fl4ZRmqSsJ5Rd1Gm1Fv2Bd1PJ5s.jpg",
        "vote_average": 8.4, "vote_count": 1200,
        "seasons": [{"season_number": 0}, {"season_number": 1}, {"season_number": 2}],
        "external_ids": {"tvdb_id": 403245, "imdb_id": "tt14452776", "tvrage_id": null},
        "content_ratings": {"results": [{"iso_3166_1": "US", "rating": "TV-MA"}]}
    })
}

/// TMDB's answer for one season of The Bear.
fn bear_season(n: i64) -> Value {
    let eps: Vec<Value> = match n {
        0 => vec![(1, "Behind the Scenes", "2022-06-01")],
        1 => vec![(1, "System", "2022-06-23"), (2, "Hands", "2022-06-23")],
        2 => vec![(1, "Beef", "2026-11-12"), (2, "Pasta", "2026-12-10")],
        _ => vec![],
    }
    .into_iter()
    .map(|(e, title, date)| {
        json!({ "id": n * 100 + e, "season_number": n, "episode_number": e, "name": title, "air_date": date,
            "runtime": 30, "overview": "Episode overview." })
    })
    .collect();
    json!({ "season_number": n, "episodes": eps })
}

/// A TMDB stand-in on a free port; its address.
async fn fake_tmdb() -> String {
    use axum::extract::Path;
    let app = Router::new()
        .route(
            "/tv/{id}",
            get(|Path(id): Path<i64>| async move {
                if id == 136315 {
                    (StatusCode::OK, axum::Json(bear_details()))
                } else {
                    (StatusCode::NOT_FOUND, axum::Json(json!({ "status_code": 34 })))
                }
            }),
        )
        .route("/tv/{id}/season/{n}", get(|Path((_, n)): Path<(i64, i64)>| async move { axum::Json(bear_season(n)) }))
        .route(
            "/search/tv",
            get(|| async {
                let mut m = bear_details();
                m.as_object_mut().unwrap().retain(|k, _| {
                    [
                        "id",
                        "name",
                        "original_name",
                        "original_language",
                        "overview",
                        "first_air_date",
                        "poster_path",
                        "backdrop_path",
                        "vote_average",
                        "vote_count",
                    ]
                    .contains(&k.as_str())
                });
                axum::Json(json!({ "page": 1, "results": [m], "total_results": 1 }))
            }),
        )
        .route(
            "/find/{id}",
            get(|Path(id): Path<String>| async move {
                let hit = id == "403245" || id == "tt14452776";
                axum::Json(json!({ "movie_results": [], "tv_results": if hit { json!([{"id": 136315}]) } else { json!([]) } }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A fresh server with the API key, a TMDB key, the stand-in TMDB and a TV root folder.
async fn server(name: &str) -> (Router, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("marquee-sonarr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::create_dir_all(dir.join("media/tv")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let tmdb = fake_tmdb().await;
    let st = server::AppState::open(&dir.join("data"), web, move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TMDB_API_KEY" => Some("tmdb-test-key".into()),
        "MARQUEE_TMDB_URL" => Some(tmdb.clone()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    (server::app(st), dir)
}

/// One request to `/sonarr/api/v3<path>` with the key; the status and the JSON body.
async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(format!("/sonarr/api/v3{path}")).header("X-Api-Key", KEY);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The id of Sonarr's "HD-1080p" profile in Marquee.
async fn hd_profile(app: &Router) -> i64 {
    let (_, p) = call(app, "GET", "/qualityprofile", None).await;
    p.as_array().unwrap().iter().find(|x| x["name"] == "HD-1080p").unwrap()["id"].as_i64().unwrap()
}

/// Adds The Bear the way Seerr and Sonarr's own page do.
async fn add_bear(app: &Router, root: &str) -> Value {
    let body = json!({ "title": "The Bear", "tvdbId": 403245, "qualityProfileId": hd_profile(app).await, "languageProfileId": 1,
        "rootFolderPath": root, "monitored": true, "seasonFolder": true, "seriesType": "standard", "tags": [],
        "seasons": [{"seasonNumber": 0, "monitored": false}, {"seasonNumber": 1, "monitored": true}, {"seasonNumber": 2, "monitored": true}],
        "addOptions": { "monitor": "all", "searchForMissingEpisodes": false, "searchForCutoffUnmetEpisodes": false } });
    let (s, v) = call(app, "POST", "/series", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

#[tokio::test]
async fn the_key_is_needed_and_the_server_answers_like_sonarr() {
    let (app, _) = server("status").await;
    let r = app.clone().oneshot(Request::get("/sonarr/api/v3/series").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r =
        app.clone().oneshot(Request::get(format!("/sonarr/api/v3/system/status?apikey={KEY}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = app.clone().oneshot(Request::get("/sonarr/ping").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let (_, v) = call(&app, "GET", "/system/status", None).await;
    assert_shape(&v, &real("system_status"), "");
    assert_eq!((v["appName"].as_str(), v["version"].as_str()), (Some("Sonarr"), Some("4.0.20.3014")));
    for (path, file) in [
        ("/health", "health"),
        ("/queue?page=1&pageSize=20", "queue"),
        ("/queue/status", "queue_status"),
        ("/queue/details", "queue_details"),
        ("/history?page=1&pageSize=20", "history"),
        ("/wanted/cutoff?page=1&pageSize=20", "wanted_cutoff"),
        ("/config/naming", "config_naming"),
        ("/config/ui", "config_ui"),
        ("/language", "language"),
        ("/customformat", "customformat"),
        ("/notification", "notification"),
        ("/downloadclient", "downloadclient"),
        ("/importlist", "importlist"),
        ("/diskspace", "diskspace"),
    ] {
        let (s, v) = call(&app, "GET", path, None).await;
        assert_eq!(s, StatusCode::OK, "{path}");
        assert_shape(&v, &real(file), path);
    }
    assert_eq!(call(&app, "GET", "/language", None).await.1, real("language"));
    assert_eq!(call(&app, "GET", "/queue/status", None).await.1, real("queue_status"));
}

#[tokio::test]
async fn qualities_profiles_folders_and_tags_have_sonarrs_shape() {
    let (app, dir) = server("setup").await;
    // Sonarr's quality list; the ids are Marquee's own (movies and TV share one table)
    let no_ids = |v: Value| -> Vec<Value> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|d| {
                let mut d = d.clone();
                d.as_object_mut().unwrap().remove("id");
                d
            })
            .collect()
    };
    assert_eq!(no_ids(call(&app, "GET", "/qualitydefinition", None).await.1), no_ids(real("qualitydefinition")));
    let (_, profiles) = call(&app, "GET", "/qualityprofile", None).await;
    let names: Vec<&str> = profiles.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    let theirs: Vec<String> = real("qualityprofile").as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, theirs);
    assert_shape(&profiles, &real("qualityprofile"), "qualityprofile");
    let hd = hd_profile(&app).await;
    let (_, one) = call(&app, "GET", &format!("/qualityprofile/{hd}"), None).await;
    let mut want = real("qualityprofile_4");
    want["id"] = json!(hd);
    assert_eq!(one, want);

    let root = dir.join("media/tv").to_string_lossy().to_string();
    let (s, v) = call(&app, "POST", "/rootfolder", Some(json!({ "path": root }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_shape(&call(&app, "GET", "/rootfolder", None).await.1, &real("rootfolder"), "rootfolder");

    let (s, t) = call(&app, "POST", "/tag", Some(json!({ "label": "kids" }))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(t, real("tag_post"));
    assert_eq!(call(&app, "GET", "/tag", None).await.1, real("tag"));
    let (_, idx) = call(&app, "GET", "/indexer/schema", None).await;
    assert_shape(&idx, &real("indexer_schema"), "indexer/schema");
}

#[tokio::test]
async fn series_are_looked_up_added_changed_and_listed_like_sonarr() {
    let (app, dir) = server("series").await;
    let root = dir.join("media/tv").to_string_lossy().to_string();
    call(&app, "POST", "/rootfolder", Some(json!({ "path": root }))).await;

    let (s, found) = call(&app, "GET", "/series/lookup?term=The%20Bear", None).await;
    assert_eq!(s, StatusCode::OK, "{found}");
    assert_eq!(found[0]["tmdbId"], 136315);
    let (_, by_tvdb) = call(&app, "GET", "/series/lookup?term=tvdb:403245", None).await;
    assert_eq!((by_tvdb[0]["tvdbId"].as_i64(), by_tvdb[0]["imdbId"].as_str()), (Some(403245), Some("tt14452776")));
    assert_eq!(by_tvdb[0]["folder"], "The Bear");

    let added = add_bear(&app, &root).await;
    let recorded = real("series_1");
    assert_shape(&added, &recorded, "series");
    for k in [
        "title",
        "sortTitle",
        "status",
        "ended",
        "overview",
        "network",
        "year",
        "seasonFolder",
        "monitored",
        "monitorNewItems",
        "runtime",
        "tvdbId",
        "tmdbId",
        "imdbId",
        "seriesType",
        "cleanTitle",
        "titleSlug",
        "certification",
        "genres",
        "originalLanguage",
    ] {
        assert_eq!(added[k], recorded[k], "`{k}` matches Sonarr's");
    }
    assert_eq!(added["path"], json!(format!("{root}/The Bear")));
    let stats = &added["statistics"];
    for k in ["seasonCount", "episodeCount", "totalEpisodeCount", "episodeFileCount"] {
        assert_eq!(stats[k], recorded["statistics"][k], "statistics.{k}");
    }

    let body = json!({ "tvdbId": 403245, "qualityProfileId": hd_profile(&app).await, "rootFolderPath": root, "title": "The Bear" });
    let (s, dup) = call(&app, "POST", "/series", Some(body)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(dup, real("series_post_duplicate"));

    let (_, list) = call(&app, "GET", "/series", None).await;
    assert_shape(&list, &real("series"), "series list");
    assert_eq!(call(&app, "GET", "/series?tvdbId=403245", None).await.1.as_array().unwrap().len(), 1);
    assert_eq!(call(&app, "GET", "/series?tvdbId=1", None).await.1, json!([]));
    assert_eq!(call(&app, "GET", "/series/99", None).await.0, StatusCode::NOT_FOUND);
    let (_, looked) = call(&app, "GET", "/series/lookup?term=The%20Bear", None).await;
    assert_eq!(looked[0]["id"], added["id"], "a lookup knows what is added");

    let id = added["id"].as_i64().unwrap();
    let (_, eps) = call(&app, "GET", &format!("/episode?seriesId={id}"), None).await;
    assert_shape(&eps, &real("episode"), "episode list");
    let titles: Vec<&str> = eps.as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Behind the Scenes", "System", "Hands", "Beef", "Pasta"]);
    let monitored: Vec<bool> = eps.as_array().unwrap().iter().map(|e| e["monitored"].as_bool().unwrap()).collect();
    assert_eq!(monitored, [false, true, true, true, true], "specials stay unmonitored");
    let abs: Vec<Value> = eps.as_array().unwrap().iter().map(|e| e["absoluteEpisodeNumber"].clone()).collect();
    assert_eq!(abs, [Value::Null, json!(1), json!(2), json!(3), json!(4)]);
    let (_, s1) = call(&app, "GET", &format!("/episode?seriesId={id}&seasonNumber=1"), None).await;
    assert_eq!(s1.as_array().unwrap().len(), 2);
    let system = eps[1]["id"].as_i64().unwrap();
    let (s, one) = call(&app, "GET", &format!("/episode/{system}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_shape(&one, &real("episode_2"), "episode");

    let (s, v) = call(&app, "PUT", "/episode/monitor", Some(json!({ "episodeIds": [system], "monitored": false }))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_shape(&v, &real("episode_monitor"), "episode/monitor");
    assert_eq!(call(&app, "GET", &format!("/episode/{system}"), None).await.1["monitored"], false);
    call(&app, "PUT", "/episode/monitor", Some(json!({ "episodeIds": [system], "monitored": true }))).await;

    let mut edit = added.clone();
    edit["monitored"] = json!(false);
    let (s, v) = call(&app, "PUT", &format!("/series/{id}"), Some(edit)).await;
    assert_eq!((s, v["monitored"].as_bool()), (StatusCode::ACCEPTED, Some(false)));
    assert_shape(&v, &real("series_put"), "series put");
    let mut edit = v.clone();
    edit["monitored"] = json!(true);
    call(&app, "PUT", &format!("/series/{id}"), Some(edit)).await;

    let (_, cal) = call(&app, "GET", "/calendar?start=2026-11-01&end=2026-12-31", None).await;
    let names: Vec<&str> = cal.as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(names, ["Beef", "Pasta"]);
    assert_shape(&cal, &real("calendar"), "calendar");
    let (_, cal) = call(&app, "GET", "/calendar?start=2026-11-01&end=2026-12-31&includeSeries=true", None).await;
    assert_shape(&cal, &real("calendar_d2c8"), "calendar with series");
    let (_, missing) = call(&app, "GET", "/wanted/missing?page=1&pageSize=20", None).await;
    assert_shape(&missing, &real("wanted_missing"), "wanted/missing");
    assert_eq!(missing["totalRecords"], 2, "only aired, monitored episodes are missing");
    assert_eq!(missing["records"][0]["title"], real("wanted_missing")["records"][0]["title"]);
    let (_, missing) = call(&app, "GET", "/wanted/missing?page=1&pageSize=20&includeSeries=true", None).await;
    assert_shape(&missing, &real("wanted_missing_b9ee"), "wanted/missing with series");

    let (s, cmd) = call(&app, "POST", "/command", Some(json!({ "name": "RefreshSeries", "seriesIds": [id] }))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_shape(&cmd, &real("command_post"), "command");

    assert_eq!(call(&app, "DELETE", &format!("/series/{id}?deleteFiles=true"), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/series", None).await.1, json!([]));
    assert_eq!(call(&app, "GET", &format!("/episode?seriesId={id}"), None).await.1, json!([]));
    assert!(dir.join("media/tv").is_dir(), "deleting never removes files");
}

#[tokio::test]
async fn the_web_apps_series_api_adds_from_tmdb() {
    let (app, dir) = server("native").await;
    let root = dir.join("media/tv").to_string_lossy().to_string();
    let run = |method: &str, path: &str, body: Option<Value>| {
        let b = Request::builder().method(method).uri(path).header("x-api-key", KEY);
        let req = match body {
            Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let app = app.clone();
        async move {
            let r = app.oneshot(req).await.unwrap();
            let s = r.status();
            let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
            (s, serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null))
        }
    };
    let (s, v) = run("POST", "/api/series", Some(json!({ "tmdb_id": 136315 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    run("POST", "/api/root-folders", Some(json!({ "kind": "tv", "path": root }))).await;
    let (s, v) = run("POST", "/api/series", Some(json!({ "tvdb_id": 403245 }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["title"], "The Bear");
    let (s, _) = run("POST", "/api/series", Some(json!({ "tmdb_id": 136315 }))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let id = v["id"].as_i64().unwrap();
    let (s, eps) = run("GET", &format!("/api/series/{id}/episodes"), None).await;
    assert_eq!(s, StatusCode::OK, "{eps}");
    assert_eq!(eps.as_array().unwrap().len(), 5);
    let (_, found) = run("GET", "/api/series/lookup?term=tt14452776", None).await;
    assert_eq!(found[0]["id"], v["id"]);
}
