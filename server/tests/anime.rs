//! Anime: language choices, the mapping list, AniList lookups mapped to TheTVDB and TMDB, and the Review queue.
//! AniList and the mapping list are stand-ins on this machine.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tower::ServiceExt;

/// The API key in the tests.
const KEY: &str = "anime-test-key";

/// AniList's entry for a Frieren season.
fn frieren(id: i64, title: &str, year: i64, status: &str) -> Value {
    json!({ "id": id, "idMal": id + 1, "title": {"romaji": title, "english": title}, "format": "TV", "status": status,
        "episodes": 28, "season": "FALL", "seasonYear": year, "startDate": {"year": year}, "coverImage": {"large": "https://img/x.jpg"},
        "nextAiringEpisode": null, "description": "An elf mage." })
}

/// Stand-ins for AniList (GraphQL) and the mapping list; their addresses.
async fn fakes() -> (String, String) {
    let app = Router::new()
        .route(
            "/graphql",
            post(|Json(b): Json<Value>| async move {
                let q = b["query"].as_str().unwrap_or("");
                if q.contains("GenreCollection") {
                    return Json(json!({ "data": { "GenreCollection": ["Action", "Fantasy", "Hentai"] } }));
                }
                if q.contains("id_in") {
                    let mut s2 = frieren(182255, "Frieren Season 2", 2026, "RELEASING");
                    s2["nextAiringEpisode"] = json!({ "episode": 5, "airingAt": 4_000_000_000i64 });
                    return Json(json!({ "data": { "Page": { "media": [frieren(154587, "Frieren: Beyond Journey's End", 2023, "FINISHED"), s2] } } }));
                }
                if q.contains("Studio(") {
                    let mut a = frieren(154587, "Frieren: Beyond Journey's End", 2023, "FINISHED");
                    a["genres"] = json!(["Fantasy"]);
                    let mut b = frieren(1, "Other", 2020, "FINISHED");
                    b["genres"] = json!(["Action"]);
                    return Json(json!({ "data": { "Studio": { "name": "Madhouse", "media": { "nodes": [a, b] } } } }));
                }
                if q.contains("Page") {
                    Json(json!({ "data": { "Page": { "media": [frieren(154587, "Frieren: Beyond Journey's End", 2023, "FINISHED")] } } }))
                } else {
                    let mut root = frieren(154587, "Frieren: Beyond Journey's End", 2023, "FINISHED");
                    let mut s2 = frieren(182255, "Frieren Season 2", 2026, "RELEASING");
                    s2["nextAiringEpisode"] = json!({ "episode": 5, "airingAt": 1_800_000_000 });
                    let movie = json!({ "id": 9, "format": "MOVIE", "title": {"romaji": "Movie"} });
                    root["relations"] = json!({ "edges": [
                        { "relationType": "SEQUEL", "node": s2 },
                        { "relationType": "SIDE_STORY", "node": movie }
                    ] });
                    Json(json!({ "data": { "Media": root } }))
                }
            }),
        )
        .route(
            "/anime-list-full.json",
            get(|| async {
                Json(json!([
                    {"type": "TV", "anidb_id": 17617, "anilist_id": 154587, "mal_id": 52991, "tvdb_id": 424536, "themoviedb_id": {"tv": 209867}, "season": {"tvdb": 1}},
                    {"type": "TV", "anidb_id": 18000, "anilist_id": 182255, "tvdb_id": 424536, "themoviedb_id": {"tv": 209867}, "season": {"tvdb": 2}},
                    {"type": "MOVIE", "anidb_id": 1}
                ]))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/graphql"), format!("http://{addr}/anime-list-full.json"))
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
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 22).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn anime_choices_mapping_lookups_and_review() {
    let dir = std::env::temp_dir().join(format!("marquee-anime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("static")).unwrap();
    std::fs::write(dir.join("static/index.html"), "<!doctype html>").unwrap();
    let (anilist, list) = fakes().await;
    let st = server::AppState::open(&dir.join("data"), dir.join("static"), move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_REQUEST_ANILIST_URL" => Some(anilist.clone()),
        "MARQUEE_ANIME_LIST_URL" => Some(list.clone()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    let app = server::app(st.clone());

    let (_, c) = call(&app, "GET", "/api/anime/prefs", None).await;
    assert_eq!(c["audio"], "dual", "the request pages' default: {c}");
    let (s, c) = call(&app, "PUT", "/api/anime/prefs", Some(json!({ "audio": "sub", "subtitles": "eng" }))).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["words"], "Sub only, English subtitles");
    let kept =
        kit_server_core::Db::one(&st.db, "SELECT anime_audio, anime_subtitles FROM request_prefs WHERE user_id=1", kit_server_core::p![])
            .unwrap()
            .unwrap();
    assert_eq!((kept["anime_audio"].as_str(), kept["anime_subtitles"].as_str()), (Some("sub"), Some("en")), "kept with the requests");
    let (s, v) = call(&app, "PUT", "/api/anime/prefs", Some(json!({ "audio": "loud" }))).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Pick Dub only, Dual audio or Sub only")));

    let (s, v) = call(&app, "POST", "/api/anime/mapping/refresh", None).await;
    assert_eq!((s, v["entries"].as_i64()), (StatusCode::OK, Some(3)));

    let (_, found) = call(&app, "GET", "/api/anime/lookup?term=frieren", None).await;
    assert_eq!(found[0]["title"], "Frieren: Beyond Journey's End");
    assert_eq!(
        (found[0]["tvdb_id"].as_i64(), found[0]["tmdb_id"].as_i64(), found[0]["tvdb_season"].as_i64()),
        (Some(424536), Some(209867), Some(1))
    );
    assert_eq!(found[0]["status"], "Finished");

    let (_, seasons) = call(&app, "GET", "/api/anime/154587/seasons", None).await;
    let titles: Vec<&str> = seasons.as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Frieren: Beyond Journey's End", "Frieren Season 2"], "seasons are separate entries; side stories aren't seasons");
    assert_eq!((seasons[1]["status"].as_str(), seasons[1]["tvdb_season"].as_i64()), (Some("Airing"), Some(2)));
    assert_eq!(seasons[1]["next_episode"]["episode"], 5);

    let wanted = server::anime::language::Choice { audio: server::anime::language::Audio::Dub, ..Default::default() };
    let id = server::anime::send_to_review(
        &st.db,
        1,
        None,
        "[Grp] Frieren - 01",
        "/data/downloads/f.mkv",
        "It has Japanese audio, but English audio was asked for",
        &wanted,
    )
    .unwrap();
    let (_, q) = call(&app, "GET", "/api/anime/review", None).await;
    assert_eq!(
        (q[0]["reason"].as_str(), q[0]["wanted"].as_str()),
        (Some("It has Japanese audio, but English audio was asked for"), Some("Dub only (English)"))
    );
    assert_eq!(call(&app, "POST", &format!("/api/anime/review/{id}/keep"), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/api/anime/review", None).await.1, json!([]));
    assert_eq!(call(&app, "POST", &format!("/api/anime/review/{id}/keep"), None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_anime_tab_lists_seasons_discover_calendar_and_downloads() {
    let dir = std::env::temp_dir().join(format!("marquee-anime-tab-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("static")).unwrap();
    std::fs::create_dir_all(dir.join("media/anime")).unwrap();
    std::fs::write(dir.join("static/index.html"), "<!doctype html>").unwrap();
    let (anilist, list) = fakes().await;
    let st = server::AppState::open(&dir.join("data"), dir.join("static"), move |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_REQUEST_ANILIST_URL" => Some(anilist.clone()),
        "MARQUEE_ANIME_LIST_URL" => Some(list.clone()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    let app = server::app(st.clone());
    assert_eq!(call(&app, "POST", "/api/anime/mapping/refresh", None).await.0, StatusCode::OK);
    let root = dir.join("media/anime").to_string_lossy().to_string();
    let (s, v) = call(&app, "POST", "/api/root-folders", Some(json!({ "kind": "anime", "path": root }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // an anime series with two seasons, the second still airing
    let info = server::tmdb::SeriesInfo {
        tmdb_id: 209867,
        tvdb_id: 424536,
        title: "Frieren".into(),
        year: 2023,
        tmdb_status: "Returning Series".into(),
        genres: vec!["Animation".into()],
        original_language: "ja".into(),
        seasons: vec![1, 2],
        ..Default::default()
    };
    let ep = |season: i64, episode: i64, date: &str| server::tmdb::EpisodeInfo {
        tmdb_id: season * 100 + episode,
        season,
        episode,
        title: format!("Episode {episode}"),
        air_date: Some(date.into()),
        ..Default::default()
    };
    let soon = (chrono::Utc::now() + chrono::Duration::days(2)).format("%Y-%m-%d").to_string();
    let eps = vec![ep(1, 1, "2023-09-29"), ep(1, 2, "2023-10-06"), ep(2, 1, "2026-01-10"), ep(2, 2, &soon)];
    let profile = server::quality::default_profile(&st.db, "tv").unwrap();
    let o = server::series::AddOptions {
        quality_profile_id: profile,
        root_folder: root.clone(),
        path: None,
        monitored: true,
        monitor: "all".into(),
        series_type: "anime".into(),
        season_folder: true,
        tags: vec![],
        seasons: None,
    };
    let show = server::series::add(&st.db, &info, eps, &o).unwrap();
    // a plain TV series stays out of the Anime tab
    let tv = server::tmdb::SeriesInfo { tmdb_id: 1, tvdb_id: 2, title: "The Bear".into(), seasons: vec![1], ..Default::default() };
    let o_tv = server::series::AddOptions { series_type: "standard".into(), ..o.clone() };
    server::series::add(&st.db, &tv, vec![ep(1, 1, &soon)], &o_tv).unwrap();

    let (s, lib) = call(&app, "GET", "/api/anime/library", None).await;
    assert_eq!(s, StatusCode::OK, "{lib}");
    let titles: Vec<&str> = lib.as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Frieren Season 2", "Frieren: Beyond Journey's End"], "each season is its own entry, named by AniList");
    let s2 = &lib[0];
    assert_eq!((s2["season"].as_i64(), s2["status"].as_str(), s2["series_id"].as_i64()), (Some(2), Some("Airing"), Some(show.id)));
    assert_eq!(s2["next_episode"]["episode"], 5);
    assert_eq!(s2["poster"], "https://img/x.jpg", "AniList's poster");
    let kept = kit_server_core::Db::one(&st.db, "SELECT COUNT(*) AS n FROM anime_anilist", kit_server_core::p![]).unwrap().unwrap();
    assert_eq!(kept["n"].as_i64(), Some(2), "AniList's answers are kept");

    // the Sonarr API still sees an anime series with absolute numbers
    let (_, sonarr) = call(&app, "GET", "/sonarr/api/v3/series", None).await;
    let frieren = sonarr.as_array().unwrap().iter().find(|x| x["tvdbId"] == 424536).unwrap();
    assert_eq!(frieren["seriesType"], "anime", "{sonarr}");
    let (_, eps) = call(&app, "GET", &format!("/sonarr/api/v3/episode?seriesId={}", show.id), None).await;
    let absolute: Vec<i64> = eps.as_array().unwrap().iter().filter_map(|e| e["absoluteEpisodeNumber"].as_i64()).collect();
    assert_eq!(absolute, [1, 2, 3, 4], "{eps}");

    let (s, cal) = call(&app, "GET", "/api/anime/calendar", None).await;
    assert_eq!(s, StatusCode::OK, "{cal}");
    assert_eq!(cal.as_array().unwrap().len(), 1, "The Bear's episode isn't anime: {cal}");
    assert_eq!((cal[0]["series_kind"].as_str(), cal[0]["absolute"].as_i64()), (Some("anime"), Some(4)));
    assert_eq!(call(&app, "GET", "/api/anime/activity", None).await.1, json!([]));

    let (s, d) = call(&app, "GET", "/api/anime/discover?list=season", None).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["results"][0]["series_id"].as_i64(), Some(show.id), "already in the library: {d}");
    assert_eq!(d["results"][0]["media_type"], "tv");
    let (_, d) = call(&app, "GET", "/api/anime/discover?list=top&studio=Madhouse&genre=Fantasy", None).await;
    let names: Vec<&str> = d["results"].as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(names, ["Frieren: Beyond Journey's End"], "the studio's anime, narrowed to the genre");
    let (s, v) = call(&app, "GET", "/api/anime/discover?list=sometime", None).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Pick trending, this season, upcoming or top rated")));
    assert_eq!(call(&app, "GET", "/api/anime/genres", None).await.1, json!(["Action", "Fantasy"]));
}
