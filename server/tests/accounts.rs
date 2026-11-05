//! Shared accounts, roles and root folders: what each role may do, quotas and auto-approve, and the folders
//! movies and TV share. Each test runs on a data folder of its own.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// A fresh server on a data folder of its own, with an owner `admin` and the API key `test-key`.
async fn server(name: &str) -> (Router, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("marquee-accounts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let st = server::AppState::open(&dir.join("data"), web, |n| match n {
        "APP_API_KEY" => Some("test-key".to_string()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    (server::app(st), dir)
}

/// One request; the status and the JSON body.
async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

/// A request with `auth`: ("key", the API key) or ("bearer", an app token).
fn req(method: &str, path: &str, auth: (&str, &str), body: Option<Value>) -> Request<Body> {
    let b = Request::builder().method(method).uri(path);
    let b = match auth {
        ("key", k) => b.header("x-api-key", k),
        (_, t) => b.header(header::AUTHORIZATION, format!("Bearer {t}")),
    };
    match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

/// The owner, through the API key.
const OWNER: (&str, &str) = ("key", "test-key");

/// Signs `username` in as a phone app and returns its token.
async fn token(app: &Router, username: &str, password: &str) -> String {
    let body = json!({ "username": username, "password": password, "app": { "device_id": "test" } });
    let r = Request::post("/api/auth/login").header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())).unwrap();
    let (s, v) = call(app, r).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn admins_make_accounts_with_roles_and_limits() {
    let (app, _) = server("make").await;
    let (s, v) = call(&app, req("GET", "/api/roles", OWNER, None)).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["admin", "requester", "viewer"]);

    let new = json!({ "username": "sam", "password": "sam-password", "movie_quota_limit": 5, "auto_approve_tv": true });
    let (s, v) = call(&app, req("POST", "/api/users", OWNER, Some(new))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["role"].as_str(), v["role_label"].as_str()), (Some("requester"), Some("Can request")));
    assert_eq!((v["movie_quota_limit"].as_i64(), v["movie_quota_days"].as_i64()), (Some(5), Some(7)));
    assert_eq!((v["auto_approve_tv"].as_bool(), v["auto_approve_movies"].as_bool()), (Some(true), Some(false)));
    assert!(v.get("password_hash").is_none());

    let bad = json!({ "username": "max", "password": "max-password", "role": "owner" });
    let (s, v) = call(&app, req("POST", "/api/users", OWNER, Some(bad))).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("The role must be admin, requester or viewer")));
    let bad = json!({ "username": "max", "password": "max-password", "tv_quota_days": 0 });
    let (s, _) = call(&app, req("POST", "/api/users", OWNER, Some(bad))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v) = call(&app, req("GET", "/api/users", OWNER, None)).await;
    assert_eq!(s, StatusCode::OK);
    let names: Vec<&str> = v.as_array().unwrap().iter().map(|u| u["username"].as_str().unwrap()).collect();
    assert_eq!(names, ["admin", "sam"]);
}

#[tokio::test]
async fn requesters_and_viewers_cannot_manage_accounts() {
    let (app, _) = server("rights").await;
    for (name, role) in [("rita", "requester"), ("vic", "viewer")] {
        let body = json!({ "username": name, "password": "long-password", "role": role });
        assert_eq!(call(&app, req("POST", "/api/users", OWNER, Some(body))).await.0, StatusCode::OK);
    }
    let rita = token(&app, "rita", "long-password").await;
    let (s, v) = call(&app, req("GET", "/api/users", ("bearer", &rita), None)).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::FORBIDDEN, Some("Only an admin can do this")));
    let (s, v) = call(&app, req("GET", "/api/auth/me", ("bearer", &rita), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((v["role"].as_str(), v["can_request"].as_bool(), v["is_admin"].as_bool()), (Some("requester"), Some(true), Some(false)));
    let vic = token(&app, "vic", "long-password").await;
    let (_, v) = call(&app, req("GET", "/api/auth/me", ("bearer", &vic), None)).await;
    assert_eq!(v["can_request"].as_bool(), Some(false));
    // an account may not raise its own limits or role
    let id = v["id"].as_i64().unwrap();
    let (s, _) = call(&app, req("PATCH", &format!("/api/users/{id}"), ("bearer", &vic), Some(json!({ "role": "admin" })))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) =
        call(&app, req("PATCH", &format!("/api/users/{id}"), ("bearer", &vic), Some(json!({ "auto_approve_movies": true })))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn accounts_change_their_own_password_with_the_current_one() {
    let (app, _) = server("password").await;
    let body = json!({ "username": "pat", "password": "first-password" });
    let (_, v) = call(&app, req("POST", "/api/users", OWNER, Some(body))).await;
    let id = v["id"].as_i64().unwrap();
    let pat = token(&app, "pat", "first-password").await;
    let path = format!("/api/users/{id}");
    let wrong = json!({ "password": "second-password", "current_password": "nope" });
    let (s, v) = call(&app, req("PATCH", &path, ("bearer", &pat), Some(wrong))).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Your current password is wrong")));
    let right = json!({ "password": "second-password", "current_password": "first-password", "email": "pat@example.com" });
    let (s, v) = call(&app, req("PATCH", &path, ("bearer", &pat), Some(right))).await;
    assert_eq!((s, v["email"].as_str()), (StatusCode::OK, Some("pat@example.com")));
    token(&app, "pat", "second-password").await;
}

#[tokio::test]
async fn the_owner_is_protected_and_turned_off_accounts_are_signed_out() {
    let (app, _) = server("protect").await;
    let (_, v) = call(&app, req("GET", "/api/users", OWNER, None)).await;
    let owner = v[0]["id"].as_i64().unwrap();
    let (s, _) = call(&app, req("PATCH", &format!("/api/users/{owner}"), OWNER, Some(json!({ "role": "viewer" })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&app, req("DELETE", &format!("/api/users/{owner}"), OWNER, None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (_, v) = call(&app, req("POST", "/api/users", OWNER, Some(json!({ "username": "dee", "password": "dee-password" })))).await;
    let id = v["id"].as_i64().unwrap();
    let dee = token(&app, "dee", "dee-password").await;
    let (s, v) = call(&app, req("PATCH", &format!("/api/users/{id}"), OWNER, Some(json!({ "disabled": true })))).await;
    assert_eq!((s, v["disabled"].as_bool()), (StatusCode::OK, Some(true)));
    assert_eq!(call(&app, req("GET", "/api/auth/me", ("bearer", &dee), None)).await.0, StatusCode::UNAUTHORIZED);
    let (s, _) = call(&app, req("DELETE", &format!("/api/users/{id}"), OWNER, None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(call(&app, req("GET", &format!("/api/users/{id}"), OWNER, None)).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn root_folders_are_shared_and_must_exist() {
    let (app, dir) = server("folders").await;
    let movies = dir.join("media/movies");
    std::fs::create_dir_all(&movies).unwrap();
    let path = movies.to_string_lossy().to_string();
    let (s, v) = call(&app, req("POST", "/api/root-folders", OWNER, Some(json!({ "kind": "movie", "path": format!("{path}/") })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["path"].as_str(), v["accessible"].as_bool()), (Some(path.as_str()), Some(true)));
    assert!(v["free_space"].as_u64().is_some());
    let (s, _) = call(&app, req("POST", "/api/root-folders", OWNER, Some(json!({ "kind": "movie", "path": path })))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let missing = dir.join("nowhere").to_string_lossy().to_string();
    let (s, v) = call(&app, req("POST", "/api/root-folders", OWNER, Some(json!({ "kind": "tv", "path": missing })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().starts_with("The server can't find the folder"));
    let (s, _) = call(&app, req("POST", "/api/root-folders", OWNER, Some(json!({ "kind": "music", "path": path })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = call(&app, req("GET", "/api/root-folders", OWNER, None)).await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    let id = v[0]["id"].as_i64().unwrap();
    assert_eq!(call(&app, req("DELETE", &format!("/api/root-folders/{id}"), OWNER, None)).await.0, StatusCode::OK);
    assert!(movies.is_dir(), "removing a root folder leaves its files alone");
}
