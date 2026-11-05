//! The forked Sonarr, Radarr and Seerr web apps (`server/src/upstream_ui.rs`): who may open them, the API key
//! only for admins, and Seerr's pages found by their address. The built apps are stand-in files here.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

/// The password the tests' owner signs in with.
const PASSWORD: &str = "correct horse battery";

/// Writes a file, and its folders.
fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A fresh server with stand-in builds of the three web apps in `upstream-ui` next to the web app's folder.
async fn server(name: &str) -> (Router, server::AppState, PathBuf) {
    let dir = std::env::temp_dir().join(format!("marquee-upstream-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    put(&web.join("index.html"), "<!doctype html><title>Marquee</title>");
    let ui = dir.join("upstream-ui");
    for app in ["sonarr", "radarr"] {
        put(
            &ui.join(app).join("index.html"),
            r#"<link href="/Content/styles.css" rel="stylesheet"><script>window.App = { urlBase: '__URL_BASE__' };</script>"#,
        );
        put(&ui.join(app).join("Content/styles.css"), "body{}");
    }
    put(&ui.join("sonarr/VERSION"), "4.0.20.3014\n");
    for f in ["index.html", "404.html", "movie/[movieId].html", "_next/static/app.js"] {
        put(&ui.join("seerr").join(f), f);
    }
    let st = server::AppState::open(&dir.join("data"), web, |name| match name {
        "APP_API_KEY" => Some("test-key".to_string()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some(PASSWORD.into())).unwrap();
    (server::app(st.clone()), st, dir)
}

/// One GET with these headers; the status, the headers and the body as text.
async fn get(app: &Router, path: &str, headers: &[(&str, &str)]) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut b = Request::get(path);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let r = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let (status, h) = (r.status(), r.headers().clone());
    let body = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (status, h, String::from_utf8_lossy(&body).into_owned())
}

/// A session cookie for a new account with this role.
async fn cookie_for(app: &Router, st: &server::AppState, role: &str) -> String {
    let name = format!("u-{role}");
    if role != "owner" {
        server::auth::create_user(&st.db, &name, PASSWORD, role).unwrap();
    }
    let user = if role == "owner" { "admin".to_string() } else { name };
    let req = Request::post("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::json!({ "username": user, "password": PASSWORD }).to_string()))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    r.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn sonarr_and_radarr_are_for_admins_and_only_admins_get_the_api_key() {
    let (app, st, _) = server("arr").await;
    // signed out: to Marquee's sign-in, which comes back afterwards
    let (s, h, _) = get(&app, "/sonarr/series/the-bear", &[]).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(h[header::LOCATION], "/?next=/sonarr/series/the-bear");
    let (s, _, _) = get(&app, "/sonarr/initialize.json", &[]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // an account that may only ask for things: back to Marquee's app, and no key
    let viewer = cookie_for(&app, &st, "requester").await;
    let (s, h, _) = get(&app, "/radarr/", &[("cookie", &viewer)]).await;
    assert_eq!((s, h[header::LOCATION].to_str().unwrap()), (StatusCode::SEE_OTHER, "/"));
    let (s, _, _) = get(&app, "/radarr/initialize.json", &[("cookie", &viewer)]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // an admin: the page under its URL base, and the API key
    let owner = cookie_for(&app, &st, "owner").await;
    let (s, _, html) = get(&app, "/sonarr/calendar", &[("cookie", &owner)]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(html.contains(r#"href="/sonarr/Content/styles.css""#) && html.contains("urlBase: '/sonarr'"), "{html}");
    let (s, _, body) = get(&app, "/sonarr/initialize.json", &[("cookie", &owner)]).await;
    assert_eq!(s, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["apiRoot"], "/sonarr/api/v3");
    assert_eq!(v["apiKey"], "test-key");
    assert_eq!(v["urlBase"], "/sonarr");
    assert_eq!(v["version"], "4.0.20.3014");

    // the app's own files are not secret; an unknown API path is not a page
    let (s, _, css) = get(&app, "/radarr/Content/styles.css", &[]).await;
    assert_eq!((s, css.as_str()), (StatusCode::OK, "body{}"));
    let (s, _, _) = get(&app, "/radarr/api/v3/nothing-here", &[("cookie", &owner)]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = get(&app, "/sonarr/..%2f..%2fdata/app.db", &[("cookie", &owner)]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn seerr_pages_are_found_by_their_address() {
    let (app, _, _) = server("seerr").await;
    let (s, _, body) = get(&app, "/requests/", &[]).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "index.html"));
    let (s, _, body) = get(&app, "/requests/movie/603", &[]).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "movie/[movieId].html"));
    let (s, _, body) = get(&app, "/requests/_next/static/app.js", &[]).await;
    assert_eq!((s, body.as_str()), (StatusCode::OK, "_next/static/app.js"));
    let (s, _, body) = get(&app, "/requests/no/such/page", &[]).await;
    assert_eq!((s, body.as_str()), (StatusCode::NOT_FOUND, "404.html"));
}
