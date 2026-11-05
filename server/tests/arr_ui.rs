//! What the forked Sonarr and Radarr web apps load before they draw: localization, custom filters, indexer flags
//! and (Radarr) collections. Shapes follow upstream's controllers at the commits in `UPSTREAM.lock`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// The API key the tests use.
const KEY: &str = "arr-ui-test-key";

/// A fresh server with the API key.
async fn server(name: &str) -> Router {
    let dir = std::env::temp_dir().join(format!("marquee-arr-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let st = server::AppState::open(&dir.join("data"), web, move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::app(st)
}

/// One request with the key; the status and the JSON body.
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
async fn localization_answers_like_upstream() {
    let app = server("loc").await;
    let (s, v) = call(&app, "GET", "/sonarr/api/v3/localization", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["strings"].as_object().unwrap().len() > 1000, "Sonarr's whole en.json");
    let (_, v) = call(&app, "GET", "/radarr/api/v3/localization", None).await;
    assert!(v["Strings"].as_object().unwrap().len() > 1000, "Radarr's keys are not camelCased");
    for app_name in ["sonarr", "radarr"] {
        let (s, v) = call(&app, "GET", &format!("/{app_name}/api/v3/localization/language"), None).await;
        assert_eq!((s, v), (StatusCode::OK, json!({ "identifier": "en" })));
    }
}

#[tokio::test]
async fn custom_filters_are_kept_per_app() {
    let app = server("filters").await;
    let body = json!({ "type": "series", "label": "Ended", "filters": [{ "key": "status", "value": ["ended"], "type": "equal" }] });
    let (s, made) = call(&app, "POST", "/sonarr/api/v3/customfilter", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = made["id"].as_i64().unwrap();
    assert_eq!(made["filters"][0]["key"], "status");
    let (_, list) = call(&app, "GET", "/sonarr/api/v3/customfilter", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, other) = call(&app, "GET", "/radarr/api/v3/customfilter", None).await;
    assert_eq!(other, json!([]), "Radarr has its own filters");
    let (s, put) = call(
        &app,
        "PUT",
        &format!("/sonarr/api/v3/customfilter/{id}"),
        Some(json!({ "id": id, "type": "series", "label": "Done", "filters": [] })),
    )
    .await;
    assert_eq!((s, put["label"].clone()), (StatusCode::ACCEPTED, json!("Done")));
    let (s, _) = call(&app, "DELETE", &format!("/sonarr/api/v3/customfilter/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "GET", &format!("/sonarr/api/v3/customfilter/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn indexer_flags_and_collections() {
    let app = server("flags").await;
    let (_, sonarr) = call(&app, "GET", "/sonarr/api/v3/indexerflag", None).await;
    assert_eq!(sonarr[0], json!({ "id": 1, "name": "Freeleech", "nameLower": "freeleech" }));
    let (_, radarr) = call(&app, "GET", "/radarr/api/v3/indexerflag", None).await;
    let names: Vec<&str> = radarr.as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"PTP_Golden") && !names.contains(&"AHD_Internal"), "obsolete flags are left out");
    let (s, c) = call(&app, "GET", "/radarr/api/v3/collection", None).await;
    assert_eq!((s, c), (StatusCode::OK, json!([])));
}

#[tokio::test]
async fn signalr_hub_speaks_the_json_protocol() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let app = server("signalr").await;
    let (s, n) = call(&app, "POST", "/sonarr/signalr/messages/negotiate?negotiateVersion=1", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(n["availableTransports"][0]["transport"], "WebSockets");
    let (s, _) = call(&app, "GET", "/sonarr/signalr/messages", None).await;
    assert_ne!(s, StatusCode::NOT_FOUND);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!("ws://{addr}/sonarr/signalr/messages?id={}&access_token={KEY}", n["connectionToken"].as_str().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(Message::Text("{\"protocol\":\"json\",\"version\":1}\u{1e}".into())).await.unwrap();
    let hello = ws.next().await.unwrap().unwrap().into_text().unwrap();
    assert_eq!(hello.as_str(), "{}\u{1e}", "handshake answer");
    let version = ws.next().await.unwrap().unwrap().into_text().unwrap();
    let v: Value = serde_json::from_str(version.trim_end_matches('\u{1e}')).unwrap();
    assert_eq!(v["arguments"][0], json!({ "name": "version", "body": { "version": server::sonarr::COMPAT_VERSION } }));
    server::signalr::broadcast(server::signalr::App::Sonarr, "series", json!({ "action": "updated", "resource": { "id": 7 } }));
    let msg = ws.next().await.unwrap().unwrap().into_text().unwrap();
    let v: Value = serde_json::from_str(msg.trim_end_matches('\u{1e}')).unwrap();
    assert_eq!(v["arguments"][0]["body"]["resource"]["id"], 7);

    let bad = format!("ws://{addr}/sonarr/signalr/messages?access_token=wrong");
    assert!(tokio_tungstenite::connect_async(bad).await.is_err(), "the key is checked");
}

#[tokio::test]
async fn api_paths_match_in_any_case() {
    let app = server("case").await;
    for path in ["/sonarr/api/v3/customFilter", "/radarr/api/v3/indexerFlag", "/Sonarr/API/v3/QualityProfile", "/radarr/api/v3/rootFolder"]
    {
        let (s, _) = call(&app, "GET", path, None).await;
        assert_eq!(s, StatusCode::OK, "{path}");
    }
    let (s, _) = call(&app, "GET", "/sonarr/api/v3/customFilter/999", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "ids still reach the route");
}

/// One request with the key in the query, as calendar apps send it; the status and the text body.
async fn text(app: &Router, path: &str) -> (StatusCode, String) {
    let r = app.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
    let s = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (s, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn calendar_feeds_are_ical() {
    let app = server("feed").await;
    let (s, _) = text(&app, "/sonarr/feed/v3/calendar/Sonarr.ics").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "the key is needed");
    let (s, body) = text(&app, &format!("/sonarr/feed/v3/calendar/Sonarr.ics?apikey={KEY}")).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.starts_with("BEGIN:VCALENDAR\r\nPRODID:-//sonarr.tv//Sonarr//EN\r\n"), "{body}");
    assert!(body.contains("X-WR-CALNAME:Sonarr TV Schedule\r\n") && body.ends_with("END:VCALENDAR\r\n"));

    let root = std::env::temp_dir().join(format!("marquee-arr-ui-feed-movies-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    call(&app, "POST", "/radarr/api/v3/rootfolder", Some(json!({ "path": root }))).await;
    let soon = (chrono::Utc::now() + chrono::Duration::days(3)).format("%Y-%m-%d").to_string();
    let body = json!({ "title": "Feed Movie", "tmdbId": 999001, "year": 2026, "qualityProfileId": 4, "rootFolderPath": root,
        "monitored": true, "minimumAvailability": "released", "tags": [], "inCinemas": format!("{soon}T00:00:00Z"), "addOptions": { "searchForMovie": false } });
    let (s, v) = call(&app, "POST", "/radarr/api/v3/movie", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_i64().unwrap();
    let (_, body) = text(&app, &format!("/radarr/feed/v3/calendar/Radarr.ics?apikey={KEY}")).await;
    assert!(body.contains("PRODID:-//radarr.video//Radarr//EN\r\n"), "{body}");
    assert!(body.contains(&format!("UID:Radarr_movie_{id}_cinemas\r\n")), "{body}");
    assert!(body.contains("SUMMARY:Feed Movie (Theatrical Release)\r\n"), "{body}");
    assert!(body.contains(&format!("DTSTART;VALUE=DATE:{}\r\n", soon.replace('-', ""))), "{body}");
    let (_, only_digital) = text(&app, &format!("/radarr/feed/v3/calendar/Radarr.ics?apikey={KEY}&releaseTypes=digitalRelease")).await;
    assert!(!only_digital.contains("_cinemas"), "release types filter");
}
