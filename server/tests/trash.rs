//! TRaSH Guides by default: the first setup (profiles, custom formats, sizes, naming for movies, TV and anime),
//! what the Radarr and Sonarr APIs show of it, the wizard, the sync, and that a person's own edits are never
//! overwritten without asking. Each test runs on a data folder of its own; GitHub is a stand-in.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use server::trash::snapshot::Snapshot;
use server::trash::sync::Source;
use server::AppState;
use tower::ServiceExt;

/// The API key the tests use (the owner).
const KEY: &str = "trash-test-key";

/// A fresh server on a data folder of its own, with TRaSH's automatic setup off (the test runs it).
async fn server(name: &str) -> (AppState, Router) {
    let dir = std::env::temp_dir().join(format!("marquee-trash-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html>").unwrap();
    let st = AppState::open(&dir.join("data"), web, |n| match n {
        "APP_API_KEY" => Some(KEY.to_string()),
        "MARQUEE_TRASH_AUTO" => Some("false".into()),
        _ => None,
    })
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some("correct horse battery".into())).unwrap();
    (st.clone(), server::app(st))
}

/// One request with the API key; the status and the JSON body.
async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(path).header("x-api-key", KEY);
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let s = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The names in a list of objects.
fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect()
}

/// A stand-in for GitHub serving `snap` as its newest commit.
struct Fake(Snapshot);

impl Source for Fake {
    async fn tree(&self) -> kit_server_core::Result<server::trash::sync::Tree> {
        Ok((
            self.0.commit.clone(),
            self.0.date.clone(),
            self.0.files.iter().map(|(k, f)| (format!("docs/json/{k}"), f.sha.clone())).collect(),
        ))
    }

    async fn file(&self, _commit: &str, path: &str) -> kit_server_core::Result<String> {
        let key = path.strip_prefix("docs/json/").unwrap();
        Ok(self.0.files[key].body.to_string())
    }
}

/// The built-in snapshot as a newer commit, with `edit` applied to the file at `key` (its blob id changes).
fn newer(key: &str, edit: impl FnOnce(&mut Value)) -> Snapshot {
    let mut s = Snapshot::bundled();
    s.commit = "f".repeat(40);
    s.date = "2099-01-01T00:00:00Z".into();
    let f = s.files.get_mut(key).unwrap();
    edit(&mut f.body);
    f.sha = "changed".into();
    s
}

/// The file key of the custom format named `name` in `app`.
fn format_key(app: &str, name: &str) -> String {
    let s = Snapshot::bundled();
    let prefix = format!("{app}/cf/");
    s.files.iter().find(|(k, f)| k.starts_with(&prefix) && f.body["name"] == name).map(|(k, _)| k.clone()).unwrap()
}

#[tokio::test]
async fn the_first_setup_makes_trash_the_default_for_movies_tv_and_anime() {
    let (st, app) = server("setup").await;
    let (made, waiting) = server::trash::setup(&st.db, server::trash::Choices::default()).unwrap();
    assert!(made > 20, "{made} items");
    assert_eq!(waiting, 0);

    // movies: TRaSH's profiles replace the stock ones, and a new movie gets HD Bluray + WEB
    let (_, profiles) = call(&app, "GET", "/radarr/api/v3/qualityprofile?apikey=x", None).await;
    assert_eq!(names(&profiles), ["HD Bluray + WEB", "[Anime] Remux-1080p"]);
    let hd = &profiles[0];
    let id = hd["id"].as_i64().unwrap();
    assert_eq!(server::quality::default_profile(&st.db, "movie").unwrap(), id);
    assert!(hd["formatItems"].as_array().unwrap().iter().any(|f| f["name"] == "HD Bluray Tier 01" && f["score"].as_i64().unwrap() > 0));
    let (_, cfs) = call(&app, "GET", "/radarr/api/v3/customformat", None).await;
    let cf_names = names(&cfs);
    for f in hd["formatItems"].as_array().unwrap() {
        assert!(cf_names.contains(&f["name"].as_str().unwrap().to_string()), "{} is listed in customformat", f["name"]);
    }
    assert!(cfs[0]["specifications"][0]["fields"][0]["name"].is_string(), "fields in the APIs' shape");

    // the qualities are Radarr's, best last, and the cutoff is one of them
    let items = hd["items"].as_array().unwrap();
    assert_eq!(items.last().unwrap()["quality"]["name"], "Bluray-1080p");
    assert_eq!(hd["cutoff"], items.last().unwrap()["quality"]["id"]);

    // TV and anime
    let (_, tv) = call(&app, "GET", "/sonarr/api/v3/qualityprofile", None).await;
    assert_eq!(names(&tv), ["WEB-1080p"]);
    assert_eq!(server::quality::default_profile(&st.db, "tv").unwrap(), tv[0]["id"].as_i64().unwrap());
    let anime = server::quality::profiles(&st.db, "anime").unwrap();
    assert_eq!(names(&json!(anime)), ["[Anime] Remux-1080p"]);
    let anime_formats: Vec<String> =
        anime[0]["formatItems"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect();
    for want in ["Anime BD Tier 01", "Anime Dual Audio", "Dubs Only", "Anime Web Tier 01"] {
        assert!(anime_formats.contains(&want.to_string()), "the anime profile has {want}");
    }
    let (_, tv_cfs) = call(&app, "GET", "/sonarr/api/v3/customformat", None).await;
    assert!(names(&tv_cfs).contains(&"Anime Dual Audio".to_string()), "anime shares Sonarr's custom formats");

    // sizes: TRaSH's, anime its own
    let (_, defs) = call(&app, "GET", "/sonarr/api/v3/qualitydefinition", None).await;
    let trash_tv = Snapshot::bundled().files["sonarr/quality-size/series.json"].body.clone();
    let row = &trash_tv["qualities"][0];
    let ours = defs.as_array().unwrap().iter().find(|d| d["quality"]["name"] == row["quality"]).unwrap();
    assert_eq!(ours["minSize"].as_f64(), row["min"].as_f64());
    let anime_defs = server::quality::definitions(&st.db, "anime").unwrap();
    let trash_anime = Snapshot::bundled().files["sonarr/quality-size/anime.json"].body.clone();
    let row = &trash_anime["qualities"][0];
    let ours = anime_defs.iter().find(|d| d["quality"]["name"] == row["quality"]).unwrap();
    assert_eq!(ours["minSize"].as_f64(), row["min"].as_f64());

    // naming: TRaSH's Jellyfin naming
    let (_, n) = call(&app, "GET", "/radarr/api/v3/config/naming", None).await;
    assert!(n["movieFolderFormat"].as_str().unwrap().contains("tmdbid-"), "{n}");
    assert_eq!(n["renameMovies"], true);
    let (_, n) = call(&app, "GET", "/sonarr/api/v3/config/naming", None).await;
    assert!(n["seriesFolderFormat"].as_str().unwrap().contains("tvdbid-"), "{n}");
    assert!(n["animeEpisodeFormat"].as_str().unwrap().contains("{absolute:000}"));

    // the change list says what was added
    let (_, changes) = call(&app, "GET", "/api/trash/changes", None).await;
    assert!(changes.as_array().unwrap().iter().all(|c| c["status"] == "added"));
}

#[tokio::test]
async fn the_wizard_picks_profiles_and_the_status_shows_them() {
    let (_, app) = server("wizard").await;
    let (s, v) = call(&app, "GET", "/api/trash", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["setUp"], false);
    assert!(v["options"]["movie"].as_array().unwrap().len() == 6, "{}", v["options"]);
    let choices = json!({
        "server": "plex",
        "movie": {"resolution": "both", "source": "remux"},
        "tv": {"resolution": "2160p", "source": "web"},
        "anime": false
    });
    let (s, v) = call(&app, "PUT", "/api/trash/choices", Some(choices.clone())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, profiles) = call(&app, "GET", "/radarr/api/v3/qualityprofile", None).await;
    assert_eq!(names(&profiles), ["Remux + WEB 1080p", "Remux + WEB 2160p"]);
    let (_, v) = call(&app, "GET", "/api/trash", None).await;
    assert_eq!(v["setUp"], true);
    assert_eq!(v["choices"], choices);
    assert_eq!(v["defaults"]["movie"]["name"], "Remux + WEB 1080p");
    assert_eq!(v["defaults"]["tv"]["name"], "WEB-2160p");
    let (_, n) = call(&app, "GET", "/radarr/api/v3/config/naming", None).await;
    assert!(n["movieFolderFormat"].as_str().unwrap().contains("{tmdb-{TmdbId}}"), "Plex naming: {n}");

    // picking again adds without touching what's there
    let (_, _) = call(&app, "PUT", "/api/trash/choices", Some(json!({"server": "plex", "movie": {"resolution": "1080p", "source": "web"}, "tv": {"resolution": "1080p", "source": "web"}, "anime": false}))).await;
    let (_, profiles) = call(&app, "GET", "/radarr/api/v3/qualityprofile", None).await;
    assert_eq!(names(&profiles), ["Remux + WEB 1080p", "Remux + WEB 2160p", "WEB 1080p"]);
    let (_, v) = call(&app, "GET", "/api/trash", None).await;
    assert_eq!(v["defaults"]["movie"]["name"], "WEB 1080p");
}

#[tokio::test]
async fn a_sync_updates_untouched_items_and_asks_about_edited_ones() {
    let (st, app) = server("sync").await;
    server::trash::setup(&st.db, server::trash::Choices::default()).unwrap();

    // someone edits the movie profile through the Radarr API
    let (_, profiles) = call(&app, "GET", "/radarr/api/v3/qualityprofile", None).await;
    let mut mine = profiles[0].clone();
    let id = mine["id"].as_i64().unwrap();
    mine["minFormatScore"] = json!(25);
    let (s, _) = call(&app, "PUT", &format!("/radarr/api/v3/qualityprofile/{id}"), Some(mine)).await;
    assert_eq!(s, StatusCode::ACCEPTED);

    // TRaSH raises WEB Tier 01's score (both the movie profile and nothing else use it) and changes a TV size
    let key = format_key("radarr", "WEB Tier 01");
    let snap = newer(&key, |cf| cf["trash_scores"]["default"] = json!(1234));
    let mut snap2 = snap.clone();
    let sizes = snap2.files.get_mut("sonarr/quality-size/series.json").unwrap();
    sizes.body["qualities"][0]["min"] = json!(11.5);
    sizes.sha = "changed-too".into();
    let out = server::trash::sync_from(&st.db, &Fake(snap2)).await.unwrap();
    assert_eq!(out["new"], true);
    assert_eq!(out["waiting"], 1, "{out}");

    // the edited profile is untouched and waits
    let (_, now) = call(&app, "GET", &format!("/radarr/api/v3/qualityprofile/{id}"), None).await;
    assert_eq!(now["minFormatScore"], 25);
    let (_, waiting) = call(&app, "GET", "/api/trash/changes?status=waiting", None).await;
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    let change = &waiting[0];
    assert_eq!(change["name"], "HD Bluray + WEB");
    let lines: Vec<&str> = change["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
    assert!(lines.iter().any(|l| l.starts_with("WEB Tier 01:") && l.ends_with("→ 1234")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("Minimum score")), "{lines:?}");

    // the untouched TV sizes were updated
    let defs = server::quality::definitions(&st.db, "tv").unwrap();
    let q = Snapshot::bundled().files["sonarr/quality-size/series.json"].body["qualities"][0]["quality"].clone();
    assert_eq!(defs.iter().find(|d| d["quality"]["name"] == q).unwrap()["minSize"].as_f64(), Some(11.5));

    // a second sync of the same commit changes nothing
    let again = server::trash::sync_from(&st.db, &Fake(Snapshot::load(&st.db))).await.unwrap();
    assert_eq!(again["new"], false);

    // the admin takes TRaSH's version
    let cid = change["id"].as_i64().unwrap();
    let (s, v) = call(&app, "POST", &format!("/api/trash/changes/{cid}/take"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, now) = call(&app, "GET", &format!("/radarr/api/v3/qualityprofile/{id}"), None).await;
    assert_eq!(now["minFormatScore"], 0);
    assert!(now["formatItems"].as_array().unwrap().iter().any(|f| f["name"] == "WEB Tier 01" && f["score"] == 1234));
    let (s, _) = call(&app, "POST", &format!("/api/trash/changes/{cid}/keep"), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "already decided");
}

#[tokio::test]
async fn kept_edits_are_not_asked_about_again_and_deleted_items_stay_deleted() {
    let (st, app) = server("keep").await;
    server::trash::setup(&st.db, server::trash::Choices::default()).unwrap();
    let (_, profiles) = call(&app, "GET", "/sonarr/api/v3/qualityprofile", None).await;
    let mut mine = profiles[0].clone();
    let id = mine["id"].as_i64().unwrap();
    mine["upgradeAllowed"] = json!(false);
    call(&app, "PUT", &format!("/sonarr/api/v3/qualityprofile/{id}"), Some(mine)).await;
    // the person deletes the anime movie profile
    let (_, movie) = call(&app, "GET", "/radarr/api/v3/qualityprofile", None).await;
    let anime_movie = movie.as_array().unwrap().iter().find(|p| p["name"] == "[Anime] Remux-1080p").unwrap()["id"].as_i64().unwrap();
    let (s, _) = call(&app, "DELETE", &format!("/radarr/api/v3/qualityprofile/{anime_movie}"), None).await;
    assert_eq!(s, StatusCode::OK);

    let key = format_key("sonarr", "WEB Tier 01");
    let snap = newer(&key, |cf| cf["trash_scores"]["default"] = json!(4321));
    let key_r = format_key("radarr", "Anime BD Tier 01");
    let mut snap = snap;
    let f = snap.files.get_mut(&key_r).unwrap();
    f.body["trash_scores"]["anime-radarr"] = json!(9999);
    f.sha = "x".into();
    let out = server::trash::sync_from(&st.db, &Fake(snap)).await.unwrap();
    assert_eq!(out["waiting"], 1, "{out}");
    let (_, waiting) = call(&app, "GET", "/api/trash/changes?status=waiting", None).await;
    let cid = waiting[0]["id"].as_i64().unwrap();
    let (s, _) = call(&app, "POST", &format!("/api/trash/changes/{cid}/keep"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, now) = call(&app, "GET", &format!("/sonarr/api/v3/qualityprofile/{id}"), None).await;
    assert_eq!(now["upgradeAllowed"], false);
    let (_, movie) = call(&app, "GET", "/radarr/api/v3/qualityprofile", None).await;
    assert!(!names(&movie).contains(&"[Anime] Remux-1080p".to_string()), "a deleted profile stays deleted");

    // applying the same TRaSH version again (the wizard) doesn't ask again
    let (_, v) = call(&app, "PUT", "/api/trash/choices", Some(serde_json::to_value(server::trash::Choices::default()).unwrap())).await;
    assert_eq!(v["waiting"], 0, "{v}");
}

#[tokio::test]
async fn a_broken_github_answer_is_reported_and_changes_nothing() {
    struct Down;
    impl Source for Down {
        async fn tree(&self) -> kit_server_core::Result<server::trash::sync::Tree> {
            Err(kit_server_core::err(502, "GitHub can't be reached: test"))
        }
        async fn file(&self, _: &str, _: &str) -> kit_server_core::Result<String> {
            unreachable!()
        }
    }
    let (st, app) = server("down").await;
    server::trash::setup(&st.db, server::trash::Choices::default()).unwrap();
    let e = server::trash::sync_from(&st.db, &Down).await.unwrap_err();
    assert!(e.detail.contains("GitHub can't be reached"));
    let (_, v) = call(&app, "GET", "/api/trash", None).await;
    assert_eq!(v["lastError"], "GitHub can't be reached: test");
    assert_eq!(v["commit"], Snapshot::bundled().commit);
}
