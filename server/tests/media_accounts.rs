//! Plex and Jellyfin, and the accounts that replace jfa-go: the connection, scans and "available", accounts
//! kept matched on both servers, invites and sign-up, password reset by mail, account expiry, and signing in
//! with Jellyfin or Plex. Jellyfin, Plex and plex.tv are one stand-in server on this machine that keeps what it
//! is told; mail goes to the data folder's outbox (the mail server `file`).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::routing::{get, post, put};
use axum::Router;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// The server's API key in these tests.
const KEY: &str = "test-key";

/// What the stand-in servers were told.
#[derive(Default)]
struct Fake {
    /// Jellyfin's users: {Id, Name, Password, Policy}
    users: Vec<Value>,
    /// Jellyfin library scans
    jf_scans: usize,
    /// Plex library scans
    plex_scans: usize,
    /// plex.tv shares: {id, email, userID, username, libs}
    shares: Vec<Value>,
    /// the PIN was asked for this many times
    pin_polls: usize,
    /// the next share id
    next: usize,
}

/// The stand-in state.
type F = Arc<Mutex<Fake>>;

/// Whether a Jellyfin request carries the API key.
fn jf_key(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains(r#"Token="jf-key""#))
}

/// The stand-in Jellyfin, Plex server (`/plex`) and plex.tv (`/tv`) on a free port; its address and state.
async fn fake() -> (String, F) {
    let f: F = Arc::new(Mutex::new(Fake::default()));
    let denied = || (StatusCode::UNAUTHORIZED, axum::Json(Value::Null));
    let app = Router::new()
        // ---- Jellyfin
        .route(
            "/System/Info",
            get(move |h: HeaderMap| async move {
                if !jf_key(&h) {
                    return denied();
                }
                (StatusCode::OK, axum::Json(json!({ "ServerName": "Den", "Version": "10.10.7" })))
            }),
        )
        .route(
            "/Library/VirtualFolders",
            get(|| async {
                axum::Json(json!([
                    { "Name": "Movies", "ItemId": "lib1", "CollectionType": "movies" },
                    { "Name": "Shows", "ItemId": "lib2", "CollectionType": "tvshows" }
                ]))
            }),
        )
        .route("/Users", get(|State(f): State<F>| async move { axum::Json(json!(f.lock().unwrap().users)) }))
        .route(
            "/Users/New",
            post(|State(f): State<F>, h: HeaderMap, axum::Json(b): axum::Json<Value>| async move {
                assert!(jf_key(&h));
                let mut f = f.lock().unwrap();
                let id = format!("{:032x}", f.users.len() + 1);
                let u = json!({ "Id": id, "Name": b["Name"], "Password": b["Password"],
                                "Policy": { "IsAdministrator": false, "IsDisabled": false, "EnableAllFolders": true, "EnabledFolders": [] } });
                f.users.push(u.clone());
                axum::Json(u)
            }),
        )
        .route(
            "/Users/{id}",
            get(|State(f): State<F>, Path(id): Path<String>| async move {
                let f = f.lock().unwrap();
                match f.users.iter().find(|u| u["Id"] == id) {
                    Some(u) => (StatusCode::OK, axum::Json(u.clone())),
                    None => (StatusCode::NOT_FOUND, axum::Json(Value::Null)),
                }
            })
            .delete(|State(f): State<F>, Path(id): Path<String>| async move {
                f.lock().unwrap().users.retain(|u| u["Id"] != id);
                StatusCode::NO_CONTENT
            }),
        )
        .route(
            "/Users/{id}/Policy",
            post(|State(f): State<F>, Path(id): Path<String>, axum::Json(b): axum::Json<Value>| async move {
                assert!(b["AuthenticationProviderId"].as_str().is_some_and(|s| !s.is_empty()), "Jellyfin needs the provider ids");
                let mut f = f.lock().unwrap();
                if let Some(u) = f.users.iter_mut().find(|u| u["Id"] == id) {
                    u["Policy"] = b;
                }
                StatusCode::NO_CONTENT
            }),
        )
        .route(
            "/Users/{id}/Password",
            post(|State(f): State<F>, Path(id): Path<String>, axum::Json(b): axum::Json<Value>| async move {
                let mut f = f.lock().unwrap();
                if let Some(u) = f.users.iter_mut().find(|u| u["Id"] == id) {
                    u["Password"] = b["NewPw"].clone();
                }
                StatusCode::NO_CONTENT
            }),
        )
        .route(
            "/Users/AuthenticateByName",
            post(|State(f): State<F>, axum::Json(b): axum::Json<Value>| async move {
                let f = f.lock().unwrap();
                match f.users.iter().find(|u| u["Name"] == b["Username"] && u["Password"] == b["Pw"]) {
                    Some(u) => (StatusCode::OK, axum::Json(json!({ "User": u, "AccessToken": "t" }))),
                    None => (StatusCode::UNAUTHORIZED, axum::Json(Value::Null)),
                }
            }),
        )
        .route(
            "/Library/Refresh",
            post(|State(f): State<F>| async move {
                f.lock().unwrap().jf_scans += 1;
                StatusCode::NO_CONTENT
            }),
        )
        .route(
            "/Items",
            get(|| async {
                axum::Json(json!({ "Items": [
                    { "Id": "a", "Type": "Movie", "ProviderIds": { "Tmdb": "603", "Imdb": "tt0133093" } },
                    { "Id": "b", "Type": "Series", "ProviderIds": { "Tmdb": "1399" } },
                    { "Id": "c", "Type": "Movie", "ProviderIds": {} }
                ], "TotalRecordCount": 3 }))
            }),
        )
        // ---- the Plex server
        .route(
            "/plex/",
            get(|h: HeaderMap| async move {
                assert_eq!(h.get("x-plex-token").and_then(|v| v.to_str().ok()), Some("owner-token"));
                axum::Json(json!({ "MediaContainer": { "friendlyName": "Living room", "version": "1.41", "machineIdentifier": "machine1" } }))
            }),
        )
        .route(
            "/plex/library/sections",
            get(|| async {
                axum::Json(json!({ "MediaContainer": { "Directory": [
                    { "key": "1", "title": "Movies", "type": "movie" },
                    { "key": "2", "title": "TV", "type": "show" },
                    { "key": "3", "title": "Music", "type": "artist" }
                ] } }))
            }),
        )
        .route(
            "/plex/library/sections/{key}/refresh",
            get(|State(f): State<F>| async move {
                f.lock().unwrap().plex_scans += 1;
                StatusCode::OK
            }),
        )
        .route(
            "/plex/library/sections/{key}/all",
            get(|Path(key): Path<String>| async move {
                let m = if key == "1" {
                    json!([{ "ratingKey": "10", "Guid": [{ "id": "imdb://tt1" }, { "id": "tmdb://603" }] }, { "ratingKey": "11", "Guid": [{ "id": "tmdb://550" }] }])
                } else {
                    json!([])
                };
                axum::Json(json!({ "MediaContainer": { "Metadata": m } }))
            }),
        )
        // ---- plex.tv
        .route(
            "/tv/api/servers/machine1",
            get(|| async {
                r#"<MediaContainer><Server machineIdentifier="machine1"><Section id="101" key="1" title="Movies" type="movie"/><Section id="102" key="2" title="TV" type="show"/><Section id="103" key="3" title="Music" type="artist"/></Server></MediaContainer>"#
            }),
        )
        .route(
            "/tv/api/servers/machine1/shared_servers",
            get(|State(f): State<F>| async move {
                let f = f.lock().unwrap();
                let rows: String = f
                    .shares
                    .iter()
                    .map(|s| format!(r#"<SharedServer id="{}" username="{}" email="{}" userID="{}"/>"#, s["id"].as_str().unwrap(), s["username"].as_str().unwrap(), s["email"].as_str().unwrap(), s["userID"].as_str().unwrap()))
                    .collect();
                format!("<MediaContainer>{rows}</MediaContainer>")
            }),
        )
        .route(
            "/tv/api/v2/shared_servers",
            post(|State(f): State<F>, axum::Json(b): axum::Json<Value>| async move {
                assert_eq!(b["machineIdentifier"], "machine1");
                let mut f = f.lock().unwrap();
                f.next += 1;
                let id = f.next.to_string();
                let email = b["invitedEmail"].as_str().unwrap().to_string();
                let user_id = if email == "fan@example.com" { "555" } else { "" };
                f.shares.push(json!({ "id": id, "email": email, "userID": user_id, "username": "", "libs": b["librarySectionIds"] }));
                axum::Json(json!({ "id": f.next, "invitedId": user_id }))
            }),
        )
        .route(
            "/tv/api/servers/machine1/shared_servers/{id}",
            put(|State(f): State<F>, Path(id): Path<String>, axum::Json(b): axum::Json<Value>| async move {
                let mut f = f.lock().unwrap();
                if let Some(s) = f.shares.iter_mut().find(|s| s["id"] == id) {
                    s["libs"] = b["shared_server"]["library_section_ids"].clone();
                }
                StatusCode::OK
            })
            .delete(|State(f): State<F>, Path(id): Path<String>| async move {
                f.lock().unwrap().shares.retain(|s| s["id"] != id);
                StatusCode::OK
            }),
        )
        .route("/tv/api/v2/pins", post(|| async { axum::Json(json!({ "id": 77, "code": "abcd1234" })) }))
        .route(
            "/tv/api/v2/pins/{id}",
            get(|State(f): State<F>| async move {
                let mut f = f.lock().unwrap();
                f.pin_polls += 1;
                axum::Json(if f.pin_polls < 2 { json!({ "id": 77, "authToken": null }) } else { json!({ "id": 77, "authToken": "person-token" }) })
            }),
        )
        .route(
            "/tv/api/v2/user",
            get(|h: HeaderMap| async move {
                assert_eq!(h.get("x-plex-token").and_then(|v| v.to_str().ok()), Some("person-token"));
                axum::Json(json!({ "id": 555, "username": "plexfan", "email": "fan@example.com" }))
            }),
        )
        .route("/tv/api/v2/resources", get(|| async { axum::Json(json!([{ "clientIdentifier": "machine1", "provides": "server" }])) }))
        .with_state(f.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), f)
}

/// A fresh server on a data folder of its own, joined to the stand-ins; `creates` turns on accounts for
/// Plex and Jellyfin sign-ins.
async fn server(name: &str, creates: bool) -> (Router, server::AppState, F) {
    let dir = std::env::temp_dir().join(format!("marquee-media-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let (base, f) = fake().await;
    let st = server::AppState::open(&dir.join("data"), web, move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_JELLYFIN_URL" => Some(base.clone()),
        "MARQUEE_JELLYFIN_API_KEY" => Some("jf-key".into()),
        "MARQUEE_PLEX_URL" => Some(format!("{base}/plex")),
        "MARQUEE_PLEX_TOKEN" => Some("owner-token".into()),
        "MARQUEE_PLEX_TV_URL" => Some(format!("{base}/tv")),
        "MARQUEE_SMTP_HOST" => Some("file".into()),
        "MARQUEE_SIGNUP_PUBLIC_URL" => Some("https://marquee.example.com/".into()),
        "MARQUEE_SIGNUP_MEDIA_CREATES" => Some(creates.to_string()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    (server::app(st.clone()), st, f)
}

/// One request; the status, the JSON body and the session cookie it set ("" for none).
async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value, String) {
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let cookie = r
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("marquee_session="))
        .map(|c| c.split(';').next().unwrap().to_string())
        .unwrap_or_default();
    let b = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null), cookie)
}

/// A request as the owner (the API key), or as nobody when `owner` is false.
fn req(method: &str, path: &str, owner: bool, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(path).header("x-requested-with", "marquee");
    if owner {
        b = b.header("x-api-key", KEY);
    }
    match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

/// Signs in with a password; the status.
async fn login(app: &Router, username: &str, password: &str) -> StatusCode {
    call(app, req("POST", "/api/auth/login", false, Some(json!({ "username": username, "password": password })))).await.0
}

/// The Jellyfin user called `name`.
fn jf_user(f: &F, name: &str) -> Option<Value> {
    f.lock().unwrap().users.iter().find(|u| u["Name"] == name).cloned()
}

#[tokio::test]
async fn servers_answer_and_say_what_they_have() {
    let (app, _, f) = server("status", false).await;
    let (s, v, _) = call(&app, req("GET", "/api/media-servers", true, None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["jellyfin"]["ok"].as_bool(), v["jellyfin"]["name"].as_str()), (Some(true), Some("Den")));
    assert_eq!((v["plex"]["ok"].as_bool(), v["plex"]["name"].as_str()), (Some(true), Some("Living room")));
    assert_eq!(v["mail"]["configured"], true);

    let (_, v, _) = call(&app, req("GET", "/api/media-servers/jellyfin/libraries", true, None)).await;
    assert_eq!(v[0], json!({ "id": "lib1", "name": "Movies", "kind": "movies" }));
    let (_, v, _) = call(&app, req("GET", "/api/media-servers/plex/libraries", true, None)).await;
    assert_eq!(v.as_array().unwrap().len(), 3);
    let (s, _, _) = call(&app, req("GET", "/api/media-servers/emby/libraries", true, None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, v, _) = call(&app, req("POST", "/api/media-servers/scan", true, Some(json!({})))).await;
    assert_eq!((s, v["ok"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    let scans = {
        let g = f.lock().unwrap();
        (g.jf_scans, g.plex_scans)
    };
    assert_eq!(scans, (1, 2), "Jellyfin once, Plex's movie and TV libraries");

    let (_, v, _) = call(&app, req("POST", "/api/media-servers/refresh", true, None)).await;
    assert_eq!((v["jellyfin"]["items"].as_i64(), v["plex"]["items"].as_i64()), (Some(2), Some(2)), "{v}");
    let (_, v, _) = call(&app, req("GET", "/api/media-servers/available?kind=movie&tmdb_ids=603,550,1", true, None)).await;
    assert_eq!(v, json!({ "603": ["jellyfin", "plex"], "550": ["plex"], "1": [] }));
    let (_, v, _) = call(&app, req("GET", "/api/media-servers/available?kind=tv&tmdb_ids=1399", true, None)).await;
    assert_eq!(v, json!({ "1399": ["jellyfin"] }));
}

#[tokio::test]
async fn accounts_follow_marquee_on_jellyfin_and_plex() {
    let (app, _, f) = server("sync", false).await;
    let new = json!({ "username": "sam", "password": "sam-password", "email": "sam@example.com",
                      "media": { "jellyfin": { "libraries": ["lib1"] }, "plex": { "libraries": ["2"] } } });
    let (s, v, _) = call(&app, req("POST", "/api/users", true, Some(new))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["media_errors"], json!([]));
    let id = v["id"].as_i64().unwrap();
    let u = jf_user(&f, "sam").expect("a Jellyfin user");
    assert_eq!(u["Password"], "sam-password", "the same password");
    assert_eq!((u["Policy"]["EnableAllFolders"].as_bool(), u["Policy"]["EnabledFolders"].clone()), (Some(false), json!(["lib1"])));
    let share = f.lock().unwrap().shares[0].clone();
    assert_eq!((share["email"].as_str(), share["libs"].clone()), (Some("sam@example.com"), json!([102])), "plex.tv's id of library 2");

    let (_, v, _) = call(&app, req("GET", &format!("/api/users/{id}/media"), true, None)).await;
    assert_eq!((v["jellyfin"]["linked"].as_bool(), v["plex"]["linked"].as_bool()), (Some(true), Some(true)), "{v}");

    // a new password goes to Jellyfin
    let (s, _, _) = call(&app, req("PATCH", &format!("/api/users/{id}"), true, Some(json!({ "password": "new-password" })))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(jf_user(&f, "sam").unwrap()["Password"], "new-password");

    // turned off: off on Jellyfin, the Plex share removed; on again: back
    call(&app, req("PATCH", &format!("/api/users/{id}"), true, Some(json!({ "disabled": true })))).await;
    assert_eq!(jf_user(&f, "sam").unwrap()["Policy"]["IsDisabled"], true);
    assert!(f.lock().unwrap().shares.is_empty());
    call(&app, req("PATCH", &format!("/api/users/{id}"), true, Some(json!({ "disabled": false })))).await;
    assert_eq!(jf_user(&f, "sam").unwrap()["Policy"]["IsDisabled"], false);
    assert_eq!(f.lock().unwrap().shares.len(), 1);

    // libraries changed; an admin sees everything
    let (_, v, _) = call(
        &app,
        req(
            "PUT",
            &format!("/api/users/{id}/media"),
            true,
            Some(json!({ "jellyfin": { "libraries": "*" }, "plex": { "libraries": ["1", "2"] } })),
        ),
    )
    .await;
    assert_eq!(v["errors"], json!([]), "{v}");
    assert_eq!(jf_user(&f, "sam").unwrap()["Policy"]["EnableAllFolders"], true);
    assert_eq!(f.lock().unwrap().shares[0]["libs"], json!([101, 102]));
    // Jellyfin access off: the user is turned off, not removed
    call(&app, req("PUT", &format!("/api/users/{id}/media"), true, Some(json!({ "jellyfin": null })))).await;
    assert_eq!(jf_user(&f, "sam").unwrap()["Policy"]["IsDisabled"], true);

    // removed: removed everywhere
    let (s, v, _) = call(&app, req("DELETE", &format!("/api/users/{id}"), true, None)).await;
    assert_eq!((s, v["media_errors"].clone()), (StatusCode::OK, json!([])));
    assert!(jf_user(&f, "sam").is_none());
    assert!(f.lock().unwrap().shares.is_empty());
}

#[tokio::test]
async fn an_existing_jellyfin_user_is_linked_and_takes_the_password_at_sign_in() {
    let (app, _, f) = server("link", false).await;
    f.lock().unwrap().users.push(json!({ "Id": "aa11", "Name": "kim", "Password": "old-jellyfin",
        "Policy": { "IsAdministrator": false, "IsDisabled": false, "EnableAllFolders": false, "EnabledFolders": ["lib2"] } }));
    let (_, v, _) = call(&app, req("POST", "/api/users", true, Some(json!({ "username": "kim", "password": "kim-password" })))).await;
    let id = v["id"].as_i64().unwrap();
    let (_, v, _) = call(&app, req("GET", "/api/media-servers/jellyfin/users", true, None)).await;
    assert_eq!((v[0]["name"].as_str(), v[0]["user_id"].clone()), (Some("kim"), Value::Null));
    let (s, v, _) =
        call(&app, req("POST", &format!("/api/users/{id}/media/jellyfin/link"), true, Some(json!({ "remote_id": "aa11" })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["jellyfin"]["password_pending"].as_bool(), v["jellyfin"]["libraries"].clone()), (Some(true), json!(["lib2"])), "{v}");
    assert_eq!(login(&app, "kim", "kim-password").await, StatusCode::OK);
    assert_eq!(jf_user(&f, "kim").unwrap()["Password"], "kim-password", "one password from now on");
    assert_eq!(jf_user(&f, "kim").unwrap()["Policy"]["EnabledFolders"], json!(["lib2"]), "its libraries stay");
}

#[tokio::test]
async fn invites_make_accounts_everywhere() {
    let (app, st, f) = server("invite", false).await;
    let body = json!({ "label": "Family", "max_uses": 2, "expires_in_days": 7, "account_days": 30,
                       "jellyfin": { "libraries": ["lib1"] }, "plex": {}, "send_to": "guest@example.com" });
    let (s, inv, _) = call(&app, req("POST", "/api/invites", true, Some(body))).await;
    assert_eq!(s, StatusCode::OK, "{inv}");
    let code = inv["code"].as_str().unwrap().to_string();
    let link = format!("https://marquee.example.com/#/invite/{code}");
    assert_eq!(inv["link"], link);
    let mails = server::mail::outbox(&st);
    assert_eq!((mails.len(), mails[0].to.as_str()), (1, "guest@example.com"));
    assert!(mails[0].text.contains(&link), "{}", mails[0].text);

    let (s, v, _) = call(&app, req("GET", &format!("/api/public/invites/{code}"), false, None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["jellyfin"].as_bool(), v["plex"].as_bool(), v["uses_left"].as_i64()), (Some(true), Some(true), Some(2)));

    let (s, v, _) = call(
        &app,
        req("POST", &format!("/api/public/invites/{code}"), false, Some(json!({ "username": "ana", "password": "ana-password" }))),
    )
    .await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Plex needs the mail address of your Plex account")));

    let sign = json!({ "username": "ana", "password": "ana-password", "email": "ana@example.com" });
    let (s, v, cookie) = call(&app, req("POST", &format!("/api/public/invites/{code}"), false, Some(sign))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!cookie.is_empty(), "signed in at once");
    assert_eq!((v["username"].as_str(), v["media_errors"].clone()), (Some("ana"), json!([])));
    assert!(v["expires_at"].as_f64().unwrap() > server::auth::now() + 29.0 * 86400.0, "{v}");
    assert_eq!(jf_user(&f, "ana").unwrap()["Password"], "ana-password");
    assert_eq!(jf_user(&f, "ana").unwrap()["Policy"]["EnabledFolders"], json!(["lib1"]));
    assert_eq!(f.lock().unwrap().shares[0]["libs"], json!([101, 102, 103]));

    // a name Jellyfin has is taken
    let (s, _, _) = call(
        &app,
        req(
            "POST",
            &format!("/api/public/invites/{code}"),
            false,
            Some(json!({ "username": "ANA", "password": "x-password", "email": "b@example.com" })),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _, _) = call(
        &app,
        req(
            "POST",
            &format!("/api/public/invites/{code}"),
            false,
            Some(json!({ "username": "bo", "password": "bo-password-1", "email": "bo@example.com" })),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "names are 3 characters or more");
    let (s, v, _) = call(
        &app,
        req(
            "POST",
            &format!("/api/public/invites/{code}"),
            false,
            Some(json!({ "username": "ben", "password": "ben-password", "email": "ben@example.com" })),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, _) = call(
        &app,
        req(
            "POST",
            &format!("/api/public/invites/{code}"),
            false,
            Some(json!({ "username": "cat", "password": "cat-password", "email": "cat@example.com" })),
        ),
    )
    .await;
    assert_eq!(
        (s, v["detail"].as_str()),
        (StatusCode::GONE, Some("This invite was already used. Ask the person who sent it for a new one."))
    );

    let (_, list, _) = call(&app, req("GET", "/api/invites", true, None)).await;
    let names: Vec<&str> = list[0]["uses"].as_array().unwrap().iter().map(|u| u["username"].as_str().unwrap()).collect();
    assert_eq!((names, list[0]["usable"].as_bool()), (vec!["ana", "ben"], Some(false)));

    // a turned-off invite can't be used
    let (_, inv2, _) = call(&app, req("POST", "/api/invites", true, Some(json!({ "max_uses": 0, "expires_in_days": 0 })))).await;
    let code2 = inv2["code"].as_str().unwrap();
    call(&app, req("PATCH", &format!("/api/invites/{code2}"), true, Some(json!({ "disabled": true })))).await;
    let (s, _, _) = call(&app, req("GET", &format!("/api/public/invites/{code2}"), false, None)).await;
    assert_eq!(s, StatusCode::GONE);
    let (s, _, _) = call(&app, req("GET", "/api/public/invites/nope", false, None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = call(&app, req("POST", "/api/invites", true, Some(json!({ "role": "admin" })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an invite never makes an admin");
}

#[tokio::test]
async fn a_forgotten_password_is_reset_by_mail_here_and_on_jellyfin() {
    let (app, st, f) = server("reset", false).await;
    let new = json!({ "username": "lee", "password": "lee-password", "email": "lee@example.com", "media": { "jellyfin": {} } });
    call(&app, req("POST", "/api/users", true, Some(new))).await;
    let (s, _, _) = call(&app, req("POST", "/api/public/password/forgot", false, Some(json!({ "login": "nobody" })))).await;
    assert_eq!(s, StatusCode::OK, "never says whether an account exists");
    let (s, _, _) = call(&app, req("POST", "/api/public/password/forgot", false, Some(json!({ "login": "LEE@example.com" })))).await;
    assert_eq!(s, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mails = server::mail::outbox(&st);
    assert_eq!(mails.len(), 1, "one mail, for lee");
    let token = mails[0].text.split("/#/reset/").nth(1).unwrap().split_whitespace().next().unwrap().to_string();

    let (s, _, _) =
        call(&app, req("POST", "/api/public/password/reset", false, Some(json!({ "token": token, "password": "short" })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v, _) =
        call(&app, req("POST", "/api/public/password/reset", false, Some(json!({ "token": token, "password": "brand-new-pass" })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(login(&app, "lee", "brand-new-pass").await, StatusCode::OK);
    assert_eq!(login(&app, "lee", "lee-password").await, StatusCode::UNAUTHORIZED);
    assert_eq!(jf_user(&f, "lee").unwrap()["Password"], "brand-new-pass");
    let (s, _, _) =
        call(&app, req("POST", "/api/public/password/reset", false, Some(json!({ "token": token, "password": "again-and-again" })))).await;
    assert_eq!(s, StatusCode::GONE, "a link works once");
}

#[tokio::test]
async fn accounts_end_on_their_date_with_a_reminder_first() {
    let (app, st, f) = server("expiry", false).await;
    let new = json!({ "username": "max", "password": "max-password", "email": "max@example.com", "media": { "jellyfin": {}, "plex": {} } });
    let (_, v, _) = call(&app, req("POST", "/api/users", true, Some(new))).await;
    let id = v["id"].as_i64().unwrap();
    let (s, _, _) = call(&app, req("PUT", &format!("/api/users/{id}/expiry"), true, Some(json!({ "days": 1 })))).await;
    assert_eq!(s, StatusCode::OK);
    server::media_servers::expiry::check_now(&st).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mails = server::mail::outbox(&st);
    assert_eq!((mails.len(), mails[0].subject.as_str()), (1, "Your Marquee account ends soon"));
    server::media_servers::expiry::check_now(&st).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(server::mail::outbox(&st).len(), 1, "one reminder");

    let past = server::auth::now() - 10.0;
    call(&app, req("PUT", &format!("/api/users/{id}/expiry"), true, Some(json!({ "expires_at": past })))).await;
    server::media_servers::expiry::check_now(&st).await.unwrap();
    let (_, v, _) = call(&app, req("GET", &format!("/api/users/{id}"), true, None)).await;
    assert_eq!(v["disabled"], true, "turned off");
    assert_eq!(jf_user(&f, "max").unwrap()["Policy"]["IsDisabled"], true);
    assert!(f.lock().unwrap().shares.is_empty());
    assert_eq!(login(&app, "max", "max-password").await, StatusCode::FORBIDDEN);

    // a later date turns it on again
    call(&app, req("PUT", &format!("/api/users/{id}/expiry"), true, Some(json!({ "days": 30 })))).await;
    assert_eq!(login(&app, "max", "max-password").await, StatusCode::OK);
    assert_eq!(jf_user(&f, "max").unwrap()["Policy"]["IsDisabled"], false);
    let (s, _, _) = call(&app, req("PUT", "/api/users/1/expiry", true, Some(json!({ "days": 3 })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the owner's account never ends");
}

#[tokio::test]
async fn people_sign_in_with_jellyfin() {
    let (app, _, f) = server("jf-login", false).await;
    let (_, v, _) = call(&app, req("GET", "/api/public/sign-in-options", false, None)).await;
    assert_eq!(v, json!({ "jellyfin": true, "plex": true, "reset": true }));
    call(
        &app,
        req("POST", "/api/users", true, Some(json!({ "username": "sol", "password": "sol-password", "media": { "jellyfin": {} } }))),
    )
    .await;
    let (s, v, cookie) =
        call(&app, req("POST", "/api/auth/jellyfin", false, Some(json!({ "username": "sol", "password": "sol-password" })))).await;
    assert_eq!((s, v["username"].as_str()), (StatusCode::OK, Some("sol")), "{v}");
    assert!(!cookie.is_empty());
    let (s, _, _) = call(&app, req("POST", "/api/auth/jellyfin", false, Some(json!({ "username": "sol", "password": "wrong-one" })))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    f.lock().unwrap().users.push(json!({ "Id": "bb22", "Name": "stranger", "Password": "stranger-pw",
        "Policy": { "IsAdministrator": false, "IsDisabled": false, "EnableAllFolders": true } }));
    let (s, _, _) =
        call(&app, req("POST", "/api/auth/jellyfin", false, Some(json!({ "username": "stranger", "password": "stranger-pw" })))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "not linked, and sign-in doesn't make accounts");

    let (app, _, f) = server("jf-login-creates", true).await;
    f.lock().unwrap().users.push(json!({ "Id": "bb22", "Name": "stranger", "Password": "stranger-pw",
        "Policy": { "IsAdministrator": false, "IsDisabled": false, "EnableAllFolders": false, "EnabledFolders": ["lib2"] } }));
    let (s, v, _) =
        call(&app, req("POST", "/api/auth/jellyfin", false, Some(json!({ "username": "stranger", "password": "stranger-pw" })))).await;
    assert_eq!((s, v["role"].as_str()), (StatusCode::OK, Some("requester")), "{v}");
    assert_eq!(login(&app, "stranger", "stranger-pw").await, StatusCode::OK, "the same password works here");
}

#[tokio::test]
async fn people_sign_in_with_plex() {
    let (app, _, _) = server("plex-login", false).await;
    call(
        &app,
        req(
            "POST",
            "/api/users",
            true,
            Some(json!({ "username": "fan", "password": "fan-password", "email": "fan@example.com", "media": { "plex": {} } })),
        ),
    )
    .await;
    let (s, v, _) = call(&app, req("POST", "/api/auth/plex/start", false, None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["id"], "77");
    assert!(v["url"].as_str().unwrap().starts_with("https://app.plex.tv/auth#?clientID=marquee-"));
    let (s, v, _) = call(&app, req("POST", "/api/auth/plex/finish", false, Some(json!({ "id": "77" })))).await;
    assert_eq!((s, v["pending"].as_bool()), (StatusCode::ACCEPTED, Some(true)));
    let (s, v, cookie) = call(&app, req("POST", "/api/auth/plex/finish", false, Some(json!({ "id": "77" })))).await;
    assert_eq!((s, v["username"].as_str()), (StatusCode::OK, Some("fan")), "{v}");
    assert!(!cookie.is_empty());
}
