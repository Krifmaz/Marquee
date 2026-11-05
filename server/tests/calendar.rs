//! `GET /api/calendar`, the phone app's calendar: signed-in only, movies by their dates in the window, bad days
//! answered in plain words. TMDB isn't set up, so movies are added through the Radarr API with their dates.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// The API key the tests use.
const KEY: &str = "calendar-test-key";

/// A fresh server on a data folder of its own, with an owner `admin`, no TMDB key and the built-in profiles (no TRaSH
/// first-start set-up).
async fn server(name: &str) -> Router {
    let dir = std::env::temp_dir().join(format!("marquee-calendar-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::create_dir_all(dir.join("media/movies")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let st = server::AppState::open(&dir.join("data"), web, |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, Some("admin".into()), Some("correct horse battery".into())).unwrap();
    let app = server::app(st);
    let root = dir.join("media/movies").to_string_lossy().to_string();
    for (tmdb, title, cinemas, digital, monitored) in [
        (1, "Zebra", "2026-11-06", "2026-12-20", true),
        (2, "Apple", "2026-10-01", "2026-11-06", true),
        (3, "Hidden", "2026-11-07", "", false),
    ] {
        let mut body = json!({ "title": title, "tmdbId": tmdb, "year": 2026, "qualityProfileId": 4, "rootFolderPath": root,
            "monitored": monitored, "minimumAvailability": "released", "inCinemas": cinemas });
        if !digital.is_empty() {
            body["digitalRelease"] = json!(digital);
        }
        let req = Request::post("/radarr/api/v3/movie")
            .header("x-api-key", KEY)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let (s, v) = call(&app, req).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    app
}

/// One request; the status and the JSON body.
async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

/// Signs in as a phone app and returns its token.
async fn token(app: &Router) -> String {
    let body = json!({ "username": "admin", "password": "correct horse battery", "app": { "device_id": "test-phone" } });
    let r = Request::post("/api/auth/login").header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())).unwrap();
    let (s, v) = call(app, r).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["token"].as_str().unwrap().to_string()
}

/// `GET path` with the phone's token.
fn get(path: &str, token: &str) -> Request<Body> {
    Request::get(path).header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn the_phone_gets_movies_by_day() {
    let app = server("days").await;
    let (s, _) = call(&app, Request::get("/api/calendar").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "signed in only");

    let t = token(&app).await;
    let (s, v) = call(&app, get("/api/calendar?start=2026-11-01&end=2026-11-30", &t)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let seen: Vec<(&str, &str, &str)> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["date"].as_str().unwrap(), e["title"].as_str().unwrap(), e["release"].as_str().unwrap()))
        .collect();
    assert_eq!(seen, [("2026-11-06", "Apple", "digital"), ("2026-11-06", "Zebra", "cinemas")]);
    assert_eq!(v[0]["kind"], "movie");
    assert_eq!(v[0]["has_file"], false);
    assert!(v[0]["movie_id"].as_i64().unwrap() > 0);

    let (_, v) = call(&app, get("/api/calendar?start=2026-11-01&end=2026-11-30&unmonitored=true", &t)).await;
    assert_eq!(v.as_array().unwrap().len(), 3, "Hidden too when asked");
}

#[tokio::test]
async fn bad_days_are_explained() {
    let app = server("bad").await;
    let t = token(&app).await;
    let (s, v) = call(&app, get("/api/calendar?start=tomorrow", &t)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("2026-11-05"), "{v}");
    let (s, _) = call(&app, get("/api/calendar?start=2026-11-10&end=2026-11-01", &t)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&app, get("/api/calendar?start=2000-01-01&end=2026-01-01", &t)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
