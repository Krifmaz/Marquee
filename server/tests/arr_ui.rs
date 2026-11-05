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
