//! Requests and the Seerr API, checked against real Seerr answers (`tests/contracts/seerr/`, recorded from
//! Jellyseerr 2.7.3 with a movie request and a TV request in its database): every field Seerr sends, Marquee
//! sends too, with the same kind of value. TMDB and AniList are stand-in servers on this machine.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// The API key the tests use.
const KEY: &str = "seerr-test-key";

/// A recorded Seerr answer.
fn real(name: &str) -> Value {
    let path = format!("{}/tests/contracts/seerr/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap()
}

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
                    None => panic!("Marquee's answer has no `{here}` (Seerr sends {tv})"),
                }
            }
        }
        (Value::Array(o), Value::Array(t)) => {
            if let (Some(a), Some(b)) = (o.first(), t.first()) {
                assert_shape(a, b, &format!("{at}[0]"));
            }
        }
        (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) | (Value::Bool(_), Value::Bool(_)) => {}
        _ => panic!("`{at}` is {ours} in Marquee's answer but {theirs} in Seerr's"),
    }
}

/// TMDB's answer for The Matrix.
fn matrix() -> Value {
    json!({
        "id": 603, "imdb_id": "tt0133093", "title": "The Matrix", "original_title": "The Matrix", "original_language": "en",
        "overview": "A computer hacker learns the truth.", "release_date": "1999-03-30", "runtime": 136, "status": "Released",
        "genres": [{"id": 28, "name": "Action"}], "poster_path": "/matrix.jpg", "backdrop_path": "/matrix-bg.jpg",
        "vote_average": 8.2, "vote_count": 26000, "popularity": 85.5,
        "belongs_to_collection": {"id": 2344, "name": "The Matrix Collection"},
        "videos": {"results": [{"site": "YouTube", "type": "Trailer", "key": "vKQi3bBA1y8", "name": "Trailer", "size": 1080}]},
        "release_dates": {"results": [{"iso_3166_1": "US", "release_dates": [{"type": 3, "release_date": "1999-03-30T00:00:00.000Z", "certification": "R"}]}]},
        "external_ids": {"imdb_id": "tt0133093"}
    })
}

/// TMDB's answer for Severance (seasons 0, 1 and 2, all aired).
fn severance() -> Value {
    json!({
        "id": 95396, "name": "Severance", "original_name": "Severance", "original_language": "en",
        "overview": "Mark leads a team of office workers.", "first_air_date": "2022-02-17", "status": "Returning Series",
        "episode_run_time": [55], "networks": [{"name": "Apple TV+"}], "genres": [{"id": 18, "name": "Drama"}],
        "poster_path": "/sev.jpg", "backdrop_path": "/sev-bg.jpg", "vote_average": 8.4, "vote_count": 2000, "number_of_seasons": 2,
        "seasons": [{"season_number": 0, "episode_count": 1}, {"season_number": 1, "episode_count": 2}, {"season_number": 2, "episode_count": 2}],
        "external_ids": {"tvdb_id": 371980, "imdb_id": "tt11280740"},
        "content_ratings": {"results": [{"iso_3166_1": "US", "rating": "TV-MA"}]},
        "keywords": {"results": [{"id": 1, "name": "office"}]}
    })
}

/// TMDB's answer for an anime (Frieren: Japanese animation).
fn frieren() -> Value {
    json!({
        "id": 209867, "name": "Frieren: Beyond Journey's End", "original_name": "葬送のフリーレン", "original_language": "ja",
        "first_air_date": "2023-09-29", "status": "Returning Series", "genres": [{"id": 16, "name": "Animation"}],
        "seasons": [{"season_number": 1}], "external_ids": {"tvdb_id": 424536}, "poster_path": "/frieren.jpg"
    })
}

/// One season's episodes from TMDB.
fn season(n: i64) -> Value {
    let eps: Vec<Value> = (1..=2)
        .map(|e| {
            json!({ "id": n * 100 + e, "season_number": n, "episode_number": e, "name": format!("Episode {e}"),
            "air_date": format!("{}-02-1{e}", 2021 + n), "runtime": 55 })
        })
        .collect();
    json!({ "season_number": n, "episodes": eps })
}

/// A TMDB and AniList stand-in on a free port; its address.
async fn fake_services() -> String {
    let page = |results: Vec<Value>| axum::Json(json!({ "page": 1, "total_pages": 1, "total_results": results.len(), "results": results }));
    let app = Router::new()
        .route(
            "/movie/{id}",
            get(|Path(id): Path<i64>| async move {
                if id == 603 {
                    (StatusCode::OK, axum::Json(matrix()))
                } else {
                    (StatusCode::NOT_FOUND, axum::Json(json!({ "status_code": 34 })))
                }
            }),
        )
        .route(
            "/tv/{id}",
            get(|Path(id): Path<i64>| async move {
                match id {
                    95396 => (StatusCode::OK, axum::Json(severance())),
                    209867 => (StatusCode::OK, axum::Json(frieren())),
                    _ => (StatusCode::NOT_FOUND, axum::Json(json!({ "status_code": 34 }))),
                }
            }),
        )
        .route("/tv/{id}/season/{n}", get(|Path((_, n)): Path<(i64, i64)>| async move { axum::Json(season(n)) }))
        .route("/discover/movie", get(move || async move { page(vec![matrix(), json!({ "id": 604, "title": "The Matrix Reloaded" })]) }))
        .route("/discover/tv", get(move || async move { page(vec![severance()]) }))
        .route(
            "/trending/all/day",
            get(move || async move {
                let mut m = matrix();
                m["media_type"] = json!("movie");
                let mut s = severance();
                s["media_type"] = json!("tv");
                page(vec![m, s])
            }),
        )
        .route(
            "/search/multi",
            get(move || async move {
                let mut m = matrix();
                m["media_type"] = json!("movie");
                page(vec![m, json!({ "id": 6384, "media_type": "person", "name": "Keanu Reeves", "known_for": [] })])
            }),
        )
        .route(
            "/anilist",
            post(|| async {
                axum::Json(json!({ "data": { "Page": { "media": [
                    { "id": 154587, "title": { "romaji": "Sousou no Frieren", "english": "Frieren: Beyond Journey's End" }, "format": "TV",
                      "status": "FINISHED", "episodes": 28, "startDate": { "year": 2023, "month": 9, "day": 29 } },
                    { "id": 182255, "title": { "romaji": "Sousou no Frieren 2nd Season" }, "format": "TV",
                      "status": "NOT_YET_RELEASED", "startDate": { "year": 2026, "month": 1, "day": 16 } }
                ] } } }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A fresh server with the API key, the stand-ins, movie, TV and anime root folders, and the accounts `sam`
/// (can request: 2 movies and 1 season a week) and `vic` (read-only).
async fn server(name: &str) -> (Router, server::AppState) {
    let dir = std::env::temp_dir().join(format!("marquee-seerr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    for k in ["movies", "tv", "anime"] {
        std::fs::create_dir_all(dir.join("media").join(k)).unwrap();
    }
    let fake = fake_services().await;
    let st = server::AppState::open(&dir.join("data"), web, move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TMDB_API_KEY" => Some("tmdb-test-key".into()),
        "MARQUEE_TMDB_URL" => Some(fake.clone()),
        "MARQUEE_REQUEST_ANILIST_URL" => Some(format!("{fake}/anilist")),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, Some("owner".into()), Some("correct horse battery".into())).unwrap();
    let app = server::app(st.clone());
    for (kind, folder) in [("movie", "movies"), ("tv", "tv"), ("anime", "anime")] {
        let path = dir.join("media").join(folder).display().to_string();
        let (s, v) = native(&app, "POST", "/api/root-folders", json!({ "kind": kind, "path": path })).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let sam = json!({ "username": "sam", "password": "sam-password-1", "email": "sam@example.com", "role": "requester",
        "movie_quota_limit": 2, "movie_quota_days": 7, "tv_quota_limit": 1, "tv_quota_days": 14 });
    assert_eq!(native(&app, "POST", "/api/users", sam).await.0, StatusCode::OK);
    let vic = json!({ "username": "vic", "password": "vic-password-1", "role": "viewer" });
    assert_eq!(native(&app, "POST", "/api/users", vic).await.0, StatusCode::OK);
    (app, st)
}

/// Marquee's own API with the key (as the owner).
async fn native(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    send(app, method, path, None, Some(body)).await
}

/// One call to `/api/v1<path>` with the key, as the account `user` (`X-Api-User`) or the owner; the status and
/// the JSON body.
async fn call(app: &Router, method: &str, path: &str, user: Option<i64>, body: Option<Value>) -> (StatusCode, Value) {
    send(app, method, &format!("/api/v1{path}"), user, body).await
}

/// One call with the key.
async fn send(app: &Router, method: &str, path: &str, user: Option<i64>, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(path).header("X-Api-Key", KEY);
    if let Some(u) = user {
        b = b.header("X-Api-User", u.to_string());
    }
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The id of the account `name`.
async fn id_of(app: &Router, name: &str) -> i64 {
    let (_, v) = call(app, "GET", "/user?take=100", None, None).await;
    v["results"].as_array().unwrap().iter().find(|u| u["username"] == name).unwrap()["id"].as_i64().unwrap()
}

#[tokio::test]
async fn the_server_answers_like_seerr_before_and_after_signing_in() {
    let (app, _) = server("status").await;
    let r = app.clone().oneshot(Request::get("/api/v1/request").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "requests need a sign-in");
    let r = app.clone().oneshot(Request::get("/api/v1/settings/public").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK, "the public settings don't");
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    let public: Value = serde_json::from_slice(&bytes).unwrap();
    assert_shape(&public, &real("settings_public"), "settings/public");
    assert_eq!(public["applicationTitle"], "Marquee");
    for (path, file) in [("/status", "status"), ("/status/appdata", "status_appdata"), ("/auth/me", "auth_me")] {
        let (s, v) = call(&app, "GET", path, None, None).await;
        assert_eq!(s, StatusCode::OK, "{path}");
        assert_shape(&v, &real(file), path);
    }
    let (_, me) = call(&app, "GET", "/auth/me", None, None).await;
    assert_eq!((me["username"].as_str(), me["permissions"].as_i64()), (Some("owner"), Some(2)));
    let sam = id_of(&app, "sam").await;
    let (_, me) = call(&app, "GET", "/auth/me", Some(sam), None).await;
    assert_eq!((me["username"].as_str(), me["permissions"].as_i64()), (Some("sam"), Some(32)), "X-Api-User acts as that account");
    let (s, e) = call(&app, "GET", "/request/999", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(e, json!({ "message": "Request not found.", "detail": "Request not found." }), "Seerr's error and the kit's");
}

#[tokio::test]
async fn a_request_goes_from_asked_to_approved_to_available() {
    let (app, st) = server("flow").await;
    let sam = id_of(&app, "sam").await;
    let vic = id_of(&app, "vic").await;

    // a read-only account can't ask
    let (s, e) = call(&app, "POST", "/request", Some(vic), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");

    // sam asks for The Matrix and season 1 of Severance; both wait for approval
    let (s, movie) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    assert_eq!(s, StatusCode::CREATED, "{movie}");
    assert_eq!((movie["status"].as_i64(), movie["media"]["status"].as_i64()), (Some(1), Some(2)), "pending request, pending media");
    assert_eq!(movie["statusWords"], "Waiting for approval");
    let (s, _) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "the same movie twice");
    let (s, tv) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "tv", "mediaId": 95396, "seasons": [1] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{tv}");
    assert_eq!(tv["seasons"][0]["seasonNumber"], 1);
    let (s, e) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "tv", "mediaId": 95396, "seasons": [2] }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "one season every 14 days: {e}");
    assert!(e["message"].as_str().unwrap().contains("limit of 1 seasons every 14 days"));
    let (_, quota) = call(&app, "GET", &format!("/user/{sam}/quota"), Some(sam), None).await;
    assert_shape(&quota, &real("user_2_quota"), "quota");
    assert_eq!(quota["movie"], json!({ "days": 7, "limit": 2, "used": 1, "remaining": 1, "restricted": false }));
    assert_eq!(quota["tv"]["restricted"], true);

    // sam can't approve; the owner can
    let mid = movie["id"].as_i64().unwrap();
    let tid = tv["id"].as_i64().unwrap();
    assert_eq!(call(&app, "POST", &format!("/request/{mid}/approve"), Some(sam), None).await.0, StatusCode::FORBIDDEN);
    let (_, pending) = call(&app, "GET", "/request?filter=pending&take=20&skip=0&sort=added", None, None).await;
    assert_eq!(pending["pageInfo"]["results"], 2);
    let (s, approved) = call(&app, "POST", &format!("/request/{mid}/approve"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{approved}");
    assert_eq!((approved["status"].as_i64(), approved["media"]["status"].as_i64()), (Some(2), Some(3)), "approved, processing");
    assert_eq!(approved["modifiedBy"]["username"], "owner");
    assert_shape(&approved, &real("request_1"), "request/1");
    let (_, movies) = native(&app, "GET", "/api/movies", Value::Null).await;
    assert_eq!(movies[0]["tmdb_id"], 603, "approving added the movie to the library");
    assert_eq!(movies[0]["monitored"], true);
    let (_, tv) = call(&app, "POST", &format!("/request/{tid}/approve"), None, None).await;
    assert_eq!(tv["status"], 2);
    assert_shape(&tv, &real("request_2_approve"), "request/2/approve");
    let (_, series) = native(&app, "GET", "/api/series", Value::Null).await;
    let seasons: Vec<(i64, bool)> = series[0]["seasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["season"].as_i64().unwrap(), s["monitored"].as_bool().unwrap()))
        .collect();
    assert_eq!(seasons, vec![(0, false), (1, true), (2, false)], "only the season asked for is looked for");

    // the lists and counts have Seerr's shape
    let (_, list) = call(&app, "GET", "/request", None, None).await;
    assert_shape(&list, &real("request"), "request");
    assert_eq!(list["results"][0]["id"].as_i64(), Some(tid), "newest first");
    let (_, mine) = call(&app, "GET", &format!("/user/{sam}/requests"), Some(sam), None).await;
    assert_shape(&mine, &real("user_2_requests"), "user/requests");
    let (_, count) = call(&app, "GET", "/request/count", None, None).await;
    assert_shape(&count, &real("request_count"), "request/count");
    assert_eq!(
        count,
        json!({ "total": 2, "movie": 1, "tv": 1, "pending": 0, "approved": 2, "declined": 0, "processing": 2, "available": 0 })
    );
    let (_, media) = call(&app, "GET", "/media", None, None).await;
    assert_shape(&media, &real("media"), "media");
    let (_, processing) = call(&app, "GET", "/media?filter=processing", None, None).await;
    assert_shape(&processing, &real("media_filter_processing"), "media?filter=processing");
    let (_, users) = call(&app, "GET", "/user", None, None).await;
    assert_shape(&users, &real("user"), "user");
    let (_, one) = call(&app, "GET", &format!("/user/{sam}"), None, None).await;
    assert_shape(&one, &real("user_2"), "user/2");
    assert_eq!(one["requestCount"], 2);
    let (_, main) = call(&app, "GET", &format!("/user/{sam}/settings/main"), None, None).await;
    assert_shape(&main, &real("user_2_settings_main"), "settings/main");
    let (_, perms) = call(&app, "GET", &format!("/user/{sam}/settings/permissions"), None, None).await;
    assert_eq!(perms, real("user_2_settings_permissions"));
    assert_eq!(call(&app, "GET", "/user", Some(sam), None).await.0, StatusCode::FORBIDDEN, "only admins list accounts");

    // the movie's file arrives: the request is done
    kit_server_core::Db::execute(&st.db, "UPDATE movies SET movie_file_id=1 WHERE tmdb_id=603", vec![]).unwrap();
    server::requests::complete_available(&st).await.unwrap();
    let (_, done) = call(&app, "GET", &format!("/request/{mid}"), Some(sam), None).await;
    assert_eq!((done["status"].as_i64(), done["media"]["status"].as_i64()), (Some(5), Some(5)), "completed, available");
    assert_eq!(done["statusWords"], "Available");
    let (_, count) = call(&app, "GET", "/request/count", None, None).await;
    assert_eq!(count["available"], 1);
    let (s, _) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "it is already there");
}

#[tokio::test]
async fn admins_decline_change_and_remove_and_people_remove_their_own() {
    let (app, _) = server("decide").await;
    let sam = id_of(&app, "sam").await;
    let (_, r) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    let id = r["id"].as_i64().unwrap();
    let (_, declined) = call(&app, "POST", &format!("/request/{id}/decline"), None, None).await;
    assert_eq!(declined["status"], 3);
    assert_eq!(declined["statusWords"], "Declined");
    let (s, again) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    assert_eq!(s, StatusCode::CREATED, "a declined request doesn't block asking again: {again}");
    let again = again["id"].as_i64().unwrap();
    let (s, changed) = call(&app, "PUT", &format!("/request/{again}"), None, Some(json!({ "mediaType": "movie", "profileId": 4 }))).await;
    assert_eq!(s, StatusCode::OK, "{changed}");
    assert_eq!(changed["profileId"], 4);
    assert_eq!(call(&app, "PUT", &format!("/request/{again}"), Some(sam), Some(json!({ "profileId": 1 }))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        call(&app, "DELETE", &format!("/request/{again}"), Some(sam), None).await.0,
        StatusCode::NO_CONTENT,
        "your own waiting request"
    );
    assert_eq!(call(&app, "DELETE", &format!("/request/{id}"), Some(sam), None).await.0, StatusCode::FORBIDDEN, "not once declined");
    let (_, list) = call(&app, "GET", "/request?filter=declined", Some(sam), None).await;
    assert_eq!(list["pageInfo"]["results"], 1);

    // an admin's own request is approved at once; an auto-approved account's too
    let (_, mine) = call(&app, "POST", "/request", None, Some(json!({ "mediaType": "tv", "mediaId": 95396, "seasons": "all" }))).await;
    assert_eq!(mine["status"], 2, "{mine}");
    assert_eq!(mine["seasons"].as_array().unwrap().len(), 2, "all regular seasons");
    let (_, p) = call(&app, "POST", &format!("/user/{sam}/settings/permissions"), None, Some(json!({ "permissions": 32 | 128 }))).await;
    assert_eq!(p["permissions"], 32 | 256 | 512);
    assert_eq!(call(&app, "DELETE", &format!("/request/{id}"), None, None).await.0, StatusCode::NO_CONTENT);
    let (_, auto) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    assert_eq!(auto["status"], 2, "auto-approved: {auto}");
    assert_eq!(auto["modifiedBy"]["username"], "sam");

    // limits from Seerr's settings page, and a title marked by hand
    let (_, main) =
        call(&app, "POST", &format!("/user/{sam}/settings/main"), None, Some(json!({ "movieQuotaLimit": 9, "movieQuotaDays": 30 }))).await;
    assert_eq!((main["movieQuotaLimit"].as_i64(), main["movieQuotaDays"].as_i64()), (Some(9), Some(30)));
    let (s, _) = call(&app, "POST", &format!("/user/{sam}/settings/main"), Some(sam), Some(json!({ "movieQuotaLimit": 99 }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "people don't raise their own limits");
    let media_id = auto["media"]["id"].as_i64().unwrap();
    let (_, m) = call(&app, "POST", &format!("/media/{media_id}/available"), None, None).await;
    assert_eq!(m["status"], 5);
    assert_eq!(call(&app, "DELETE", &format!("/media/{media_id}"), None, None).await.0, StatusCode::NO_CONTENT);
    let (_, list) = call(&app, "GET", "/request?mediaType=movie", None, None).await;
    assert_eq!(list["pageInfo"]["results"], 0, "removing the title removes its requests");
}

#[tokio::test]
async fn discover_and_search_come_from_tmdb_with_what_was_asked_for() {
    let (app, _) = server("discover").await;
    let sam = id_of(&app, "sam").await;
    call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "movie", "mediaId": 603 }))).await;
    let (s, page) = call(&app, "GET", "/discover/movies?page=1", Some(sam), None).await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!((page["page"].as_i64(), page["totalResults"].as_i64()), (Some(1), Some(2)));
    let first = &page["results"][0];
    for k in ["id", "mediaType", "title", "posterPath", "releaseDate", "voteAverage", "originalLanguage", "overview"] {
        assert!(first.get(k).is_some(), "Seerr's MovieResult has `{k}`: {first}");
    }
    assert_eq!(first["mediaType"], "movie");
    assert_eq!(first["mediaInfo"]["status"], 2, "The Matrix was asked for");
    assert!(page["results"][1].get("mediaInfo").is_none(), "nobody asked for Reloaded");
    let (_, tv) = call(&app, "GET", "/discover/tv", Some(sam), None).await;
    assert_eq!((tv["results"][0]["mediaType"].as_str(), tv["results"][0]["name"].as_str()), (Some("tv"), Some("Severance")));
    assert!(tv["results"][0].get("firstAirDate").is_some());
    let (_, trending) = call(&app, "GET", "/discover/trending", Some(sam), None).await;
    let kinds: Vec<&str> = trending["results"].as_array().unwrap().iter().map(|r| r["mediaType"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["movie", "tv"]);
    let (_, found) = call(&app, "GET", "/search?query=matrix", Some(sam), None).await;
    assert_eq!(found["results"][1]["mediaType"], "person");
    let (_, empty) = call(&app, "GET", "/search?query=", Some(sam), None).await;
    assert_eq!(empty["totalResults"], 0);
    let (_, details) = call(&app, "GET", "/movie/603", Some(sam), None).await;
    assert_eq!(details["collection"]["name"], "The Matrix Collection");
    assert_eq!(details["relatedVideos"][0]["key"], "vKQi3bBA1y8");
    assert_eq!(details["mediaInfo"]["status"], 2);
    let (_, show) = call(&app, "GET", "/tv/95396", Some(sam), None).await;
    assert_eq!(show["numberOfSeasons"], 2);
    assert_eq!(show["seasons"][1]["seasonNumber"], 1);
    assert_eq!(show["keywords"][0]["name"], "office");
    let (s, e) = call(&app, "GET", "/movie/1", Some(sam), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{e}");
    assert!(e["message"].as_str().is_some());
}

#[tokio::test]
async fn anime_requests_carry_the_language_choice_and_anilist_seasons() {
    let (app, _) = server("anime").await;
    let sam = id_of(&app, "sam").await;
    let (_, main) = call(&app, "POST", &format!("/user/{sam}/settings/main"), Some(sam), Some(json!({ "animeAudio": "dub" }))).await;
    assert_eq!((main["animeAudio"].as_str(), main["animeSubtitles"].as_str()), (Some("dub"), Some("en")), "{main}");
    let (s, r) = call(&app, "POST", "/request", Some(sam), Some(json!({ "mediaType": "tv", "mediaId": 209867, "seasons": [1] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!((r["anime"].as_bool(), r["animeAudio"].as_str(), r["animeSubtitles"].as_str()), (Some(true), Some("dub"), Some("en")));
    let (s, e) = call(&app, "PUT", &format!("/request/{}", r["id"]), None, Some(json!({ "animeAudio": "loud" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    let (_, ok) = call(&app, "POST", &format!("/request/{}/approve", r["id"]), None, None).await;
    assert_eq!(ok["status"], 2, "{ok}");
    let (_, series) = native(&app, "GET", "/api/series", Value::Null).await;
    assert_eq!(series[0]["kind"], "anime", "anime goes to the anime folder");
    let (s, a) = call(&app, "GET", "/tv/209867/anilist", Some(sam), None).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let entries = a["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "each season is its own entry");
    assert_eq!((entries[0]["airing"].as_str(), entries[1]["airing"].as_str()), (Some("Finished"), Some("Not yet aired")));
}
