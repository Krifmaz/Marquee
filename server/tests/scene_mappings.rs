//! Sonarr's `SceneMappingServiceFixture` ported to Rust (DataAugmentation/Scene at the commit in UPSTREAM.lock),
//! plus checks of the table, TheXEM's name list, the daily series list and the Levenshtein distance.

use std::sync::{Arc, Mutex};

use kit_server_core::{Result, SqliteDb};
use serde_json::json;
use server::augment::scene::{levenshtein_distance, InvalidSceneMapping, MappingStore, SceneMapping, SceneMappingService, SqliteMappings};

/// What the fake store holds and how it was used.
#[derive(Default)]
struct Log {
    /// The mappings it holds.
    held: Vec<SceneMapping>,
    /// How often `all` was called.
    reads: usize,
    /// The types `clear` was called with.
    clears: Vec<String>,
    /// The lists `insert_many` was called with.
    inserts: Vec<Vec<SceneMapping>>,
}

/// An in-memory store that records its calls (the fixture's mocked repository).
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Log>>);

impl MappingStore for Fake {
    fn all(&self) -> Result<Vec<SceneMapping>> {
        let mut l = self.0.lock().unwrap();
        l.reads += 1;
        Ok(l.held.clone())
    }
    fn clear(&self, kind: &str) -> Result<()> {
        let mut l = self.0.lock().unwrap();
        l.clears.push(kind.to_string());
        l.held.retain(|m| m.kind != kind);
        Ok(())
    }
    fn insert_many(&self, list: &[SceneMapping]) -> Result<()> {
        let mut l = self.0.lock().unwrap();
        l.inserts.push(list.to_vec());
        l.held.extend(list.iter().cloned());
        Ok(())
    }
}

/// A service over a fake store holding `held`, and the store.
fn subject(held: Vec<SceneMapping>) -> (SceneMappingService, Fake) {
    let fake = Fake::default();
    fake.0.lock().unwrap().held = held;
    (SceneMappingService::new(Box::new(fake.clone())), fake)
}

/// `n` mappings filled the way NBuilder fills them (`Title1`, TheTVDB id 1, season 1, ...).
fn built(n: usize) -> Vec<SceneMapping> {
    (1..=n as i32)
        .map(|i| SceneMapping {
            title: format!("Title{i}"),
            parse_term: format!("ParseTerm{i}"),
            search_term: format!("SearchTerm{i}"),
            tvdb_id: i as i64,
            season_number: Some(i),
            scene_season_number: Some(i),
            scene_origin: format!("SceneOrigin{i}"),
            comment: format!("Comment{i}"),
            filter_regex: format!("FilterRegex{i}"),
            kind: format!("Type{i}"),
            ..SceneMapping::default()
        })
        .collect()
}

/// The fixture's five mappings with search names `Words`, `That`, `Can`, `Be`, `Cleaned` and no filters.
fn fake_mappings() -> Vec<SceneMapping> {
    let mut list = built(5);
    for (m, w) in list.iter_mut().zip(["Words", "That", "Can", "Be", "Cleaned"]) {
        m.filter_regex = String::new();
        m.search_term = w.into();
        m.parse_term = w.into();
    }
    list
}

/// A mapping with a title, cleaned name, search name, TheTVDB id and seasons.
fn mapping(title: &str, parse: &str, tvdb: i64, season: Option<i32>, scene: Option<i32>) -> SceneMapping {
    SceneMapping {
        title: title.into(),
        parse_term: parse.into(),
        search_term: title.into(),
        tvdb_id: tvdb,
        season_number: season,
        scene_season_number: scene,
        ..SceneMapping::default()
    }
}

#[test]
fn should_purge_existing_mapping_and_add_new_ones() {
    let (s, fake) = subject(vec![]);
    let fakes = fake_mappings();
    s.update_with(vec![("ServicesProvider", Ok(fakes.clone()))]);
    let l = fake.0.lock().unwrap();
    assert_eq!(l.clears, vec!["ServicesProvider"]);
    assert_eq!(l.inserts.len(), 1);
    drop(l);
    let seasons: Vec<i32> = fakes.iter().filter_map(|m| m.season_number).collect();
    for m in &fakes {
        assert!(s.get_scene_names(m.tvdb_id, &seasons, &[]).contains(&m.search_term));
        let parse = server::parser::episode::clean_series_title(&m.title);
        assert_eq!(s.find_tvdb_id(&parse, None, m.scene_season_number.unwrap_or(-1)), Ok(Some(m.tvdb_id)));
    }
}

#[test]
fn should_not_delete_if_fetch_fails() {
    let (s, fake) = subject(vec![]);
    s.update_with(vec![("ServicesProvider", Err("The connection failed".into()))]);
    let l = fake.0.lock().unwrap();
    assert!(l.clears.is_empty() && l.inserts.is_empty());
}

#[test]
fn should_not_delete_if_fetch_returns_empty_list() {
    let (s, fake) = subject(vec![]);
    s.update_with(vec![("ServicesProvider", Ok(vec![]))]);
    let l = fake.0.lock().unwrap();
    assert!(l.clears.is_empty() && l.inserts.is_empty());
}

#[test]
fn should_get_mappings_for_all_providers() {
    let (s, fake) = subject(vec![]);
    s.update_with(vec![("ServicesProvider", Ok(fake_mappings())), ("XemService", Ok(fake_mappings()))]);
    let l = fake.0.lock().unwrap();
    assert_eq!(l.clears, vec!["ServicesProvider", "XemService"]);
    assert_eq!(l.inserts.len(), 2);
}

#[test]
fn should_refresh_cache_if_cache_is_empty_when_looking_for_tvdb_id() {
    let (s, fake) = subject(vec![]);
    let _ = s.find_tvdb_id("title", None, -1);
    assert_eq!(fake.0.lock().unwrap().reads, 1);
}

#[test]
fn should_not_refresh_cache_if_cache_is_not_empty_when_looking_for_tvdb_id() {
    let (s, fake) = subject(built(1));
    s.update_with(vec![("ServicesProvider", Ok(fake_mappings()))]);
    assert_eq!(fake.0.lock().unwrap().reads, 1);
    let _ = s.find_tvdb_id("title", None, -1);
    assert_eq!(fake.0.lock().unwrap().reads, 1);
}

#[test]
fn should_not_add_mapping_with_blank_title() {
    let (s, fake) = subject(vec![]);
    let mut list = built(2);
    list[1].title = String::new();
    s.update_with(vec![("ServicesProvider", Ok(list))]);
    let l = fake.0.lock().unwrap();
    assert_eq!(l.inserts.len(), 1);
    assert!(l.inserts[0].iter().all(|m| !m.title.trim().is_empty()));
    assert_eq!(l.inserts[0].len(), 1);
}

#[test]
fn should_not_add_mapping_with_blank_search_title() {
    let (s, fake) = subject(vec![]);
    let mut list = built(2);
    list[1].search_term = String::new();
    s.update_with(vec![("ServicesProvider", Ok(list))]);
    let l = fake.0.lock().unwrap();
    assert_eq!(l.inserts.len(), 1);
    assert!(l.inserts[0].iter().all(|m| !m.search_term.trim().is_empty()));
}

#[test]
fn should_return_single_match() {
    for (parse_title, _title, expected) in [
        ("Working!!", "Working!!", 1),
        ("Working`!!", "Working`!!", 2),
        ("Working!!!", "Working!!!", 3),
        ("Working!!!!", "Working!!!", 3),
        ("Working !!", "Working!!", 1),
    ] {
        let (s, _) = subject(vec![
            mapping("Working!!", "working", 100, None, Some(1)),
            mapping("Working`!!", "working", 100, None, Some(2)),
            mapping("Working!!!", "working", 100, None, Some(3)),
        ]);
        assert_eq!(s.find_tvdb_id(parse_title, None, -1), Ok(Some(100)), "{parse_title}");
        assert_eq!(s.get_scene_season_number(parse_title, None), Ok(Some(expected)), "{parse_title}");
    }
}

#[test]
fn should_return_alternate_title_for_global_season() {
    let (s, _) = subject(vec![
        mapping("Fudanshi Koukou Seikatsu 1", "fudanshikoukouseikatsu1", 100, None, None),
        mapping("Fudanshi Koukou Seikatsu 2", "fudanshikoukouseikatsu2", 100, Some(-1), None),
        mapping("Fudanshi Koukou Seikatsu 3", "fudanshikoukouseikatsu3", 100, None, Some(-1)),
        mapping("Fudanshi Koukou Seikatsu 4", "fudanshikoukouseikatsu4", 100, Some(-1), Some(-1)),
    ]);
    assert_eq!(s.get_scene_names(100, &[10], &[10]).len(), 4);
}

#[test]
fn should_return_alternate_title_for_season() {
    let (s, _) = subject(vec![mapping("Fudanshi Koukou Seikatsu", "fudanshikoukouseikatsu", 100, Some(1), None)]);
    assert_eq!(s.get_scene_names(100, &[1], &[10]).len(), 1);
}

#[test]
fn should_not_return_alternate_title_for_season() {
    let (s, _) = subject(vec![mapping("Fudanshi Koukou Seikatsu", "fudanshikoukouseikatsu", 100, Some(1), None)]);
    assert!(s.get_scene_names(100, &[2], &[10]).is_empty());
}

#[test]
fn should_return_alternate_title_for_sceneseason() {
    let (s, _) = subject(vec![mapping("Fudanshi Koukou Seikatsu", "fudanshikoukouseikatsu", 100, None, Some(1))]);
    assert_eq!(s.get_scene_names(100, &[10], &[1]).len(), 1);
}

#[test]
fn should_not_return_alternate_title_for_sceneseason() {
    let (s, _) = subject(vec![mapping("Fudanshi Koukou Seikatsu", "fudanshikoukouseikatsu", 100, None, Some(1))]);
    assert!(s.get_scene_names(100, &[10], &[2]).is_empty());
}

#[test]
fn should_return_alternate_title_for_fairy_tail() {
    let (s, _) = subject(vec![mapping("Fairy Tail S2", "fairytails2", 100, None, Some(2))]);
    for season in [4, 5, 6, 7] {
        assert!(s.get_scene_names(100, &[season], &[20]).is_empty());
    }
    assert!(s.get_scene_names(100, &[20], &[1]).is_empty());
    assert!(!s.get_scene_names(100, &[20], &[2]).is_empty());
    assert!(s.get_scene_names(100, &[20], &[3]).is_empty());
    assert!(s.get_scene_names(100, &[20], &[4]).is_empty());
}

#[test]
fn should_return_alternate_title_for_fudanshi() {
    let (s, _) = subject(vec![mapping("Fudanshi Koukou Seikatsu", "fudanshikoukouseikatsu", 100, None, Some(1))]);
    for season in [1, 2, 3, 4] {
        assert!(s.get_scene_names(100, &[season], &[20]).is_empty());
    }
    assert!(!s.get_scene_names(100, &[1], &[1]).is_empty());
    for season in [2, 3, 4] {
        assert!(s.get_scene_names(100, &[season], &[season]).is_empty());
    }
}

#[test]
fn should_filter_by_regex() {
    let mut viva = mapping("Amareto", "amareto", 101, None, None);
    viva.filter_regex = "-Viva$".into();
    let (s, _) = subject(vec![mapping("Amareto", "amareto", 100, None, None), viva]);
    assert_eq!(s.find_tvdb_id("Amareto", Some("Amareto.S01E01.720p.WEB-DL-Viva"), -1), Ok(Some(101)));
    assert_eq!(s.find_tvdb_id("Amareto", Some("Amareto.S01E01.720p.WEB-DL-DMO"), -1), Ok(Some(100)));
}

#[test]
fn should_throw_if_multiple_mappings() {
    let (s, _) = subject(vec![mapping("Amareto", "amareto", 100, None, None), mapping("Amareto", "amareto", 101, None, None)]);
    let e = s.find_tvdb_id("Amareto", Some("Amareto.S01E01.720p.WEB-DL-Viva"), -1).unwrap_err();
    assert_eq!(e, InvalidSceneMapping { tvdb_ids: vec![100, 101], release_title: "Amareto.S01E01.720p.WEB-DL-Viva".into() });
    assert!(e.to_string().contains("conflict for tvdbids 100,101"));
}

#[test]
fn should_not_throw_if_multiple_mappings_with_same_tvdbid() {
    let (s, _) = subject(vec![mapping("Amareto", "amareto", 100, None, None), mapping("Amareto", "amareto", 100, None, None)]);
    assert_eq!(s.find_tvdb_id("Amareto", Some("Amareto.S01E01.720p.WEB-DL-Viva"), -1), Ok(Some(100)));
}

#[test]
fn should_pick_best_season() {
    let (s, _) = subject(vec![mapping("Amareto", "amareto", 100, Some(3), Some(2)), mapping("Amareto", "amareto", 101, Some(3), Some(3))]);
    assert_eq!(s.find_tvdb_id("Amareto", Some("Amareto.S01E01.720p.WEB-DL-Viva"), 4), Ok(Some(101)));
}

/// The mappings survive in `scene_mappings`, and a fresh service reads them back.
#[test]
fn sqlite_store_keeps_mappings_per_type() {
    let dir = std::env::temp_dir().join(format!("marquee-scene-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = SqliteDb::open(&dir.join("app.db")).unwrap();
    server::augment::migrate(&db).unwrap();
    let s = SceneMappingService::new(Box::new(SqliteMappings(db.clone())));
    let mut viva = mapping("Amareto", "", 101, Some(2), Some(1));
    viva.filter_regex = "-Viva$".into();
    viva.search_mode = Some(2);
    s.update_with(vec![("ServicesProvider", Ok(vec![viva])), ("XemService", Ok(vec![mapping("Fate Zero", "", 79151, None, Some(1))]))]);
    s.update_with(vec![("XemService", Ok(vec![mapping("Fate/Zero", "", 79151, None, Some(1))]))]);
    assert!(server::augment::scene::last_fetched(&db).is_some());
    let again = SceneMappingService::new(Box::new(SqliteMappings(db.clone())));
    let all = again.find_by_tvdb_id(101);
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].parse_term.as_str(), all[0].search_mode, all[0].season_number), ("amareto", Some(2), Some(2)));
    assert_eq!(all[0].kind, "ServicesProvider");
    let xem = again.find_by_tvdb_id(79151);
    assert_eq!(xem.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), vec!["Fate/Zero"]);
    assert_eq!(again.find_tvdb_id("Fate Zero", None, -1), Ok(Some(79151)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sonarr's services' JSON reads with its own names, with `searchMode` as a number or as names.
#[test]
fn services_json_reads() {
    let list: Vec<SceneMapping> = serde_json::from_value(json!([
        {"title": "Amareto", "searchTitle": "Amareto", "season": -1, "tvdbId": 100, "sceneSeasonNumber": null,
         "sceneOrigin": "unknown", "searchMode": "searchID, searchTitle", "comment": null, "filterRegex": "-Viva$"},
        {"title": "B", "searchTitle": "B", "tvdbId": 101, "searchMode": 2},
        {"title": "C", "searchTitle": "C", "tvdbId": 102}
    ]))
    .unwrap();
    assert_eq!(list[0].search_term, "Amareto");
    assert_eq!(list[0].season_number, Some(-1));
    assert_eq!(list[0].search_mode, Some(3));
    assert_eq!(list[0].filter_regex, "-Viva$");
    assert_eq!(list[1].search_mode, Some(2));
    assert_eq!(list[2].search_mode, None);
}

/// TheXEM's names read with their seasons; names without a whole-number season and Fate/Zero's later seasons
/// are left out.
#[test]
fn xem_names_read() {
    let names = server::augment::xem::names_from(&json!({
        "result": "success",
        "data": {
            "79151": [{"Fate/Zero": 1}, {"Fate/Zero 2nd Season": 2}],
            "100": [{"Amareto": "1"}, {"Amareto All": "all"}, {"Amareto Two": 2}]
        },
        "message": ""
    }));
    let got: Vec<(i64, &str, Option<i32>)> = names.iter().map(|m| (m.tvdb_id, m.title.as_str(), m.scene_season_number)).collect();
    assert!(got.contains(&(79151, "Fate/Zero", Some(1))));
    assert!(!got.iter().any(|g| g.1 == "Fate/Zero 2nd Season" || g.1 == "Amareto All"));
    assert!(got.contains(&(100, "Amareto", Some(1))) && got.contains(&(100, "Amareto Two", Some(2))));
    assert!(names.iter().all(|m| m.search_term == m.title));
}

/// The daily series list reads as TheTVDB ids, and a kept list answers without fetching.
#[tokio::test]
async fn daily_series_reads() {
    assert_eq!(server::augment::daily::ids_from(&json!([{"tvdbId": 71256}, {"tvdbId": 72231}])), vec![71256, 72231]);
    let d = server::augment::daily::DailySeries::new();
    d.set(vec![71256]);
    let http = reqwest::Client::new();
    assert!(d.is_daily_series(&http, 71256).await);
    assert!(!d.is_daily_series(&http, 1).await);
}

/// Sonarr's Levenshtein distance with its own costs.
#[test]
fn levenshtein_matches_sonarr() {
    assert_eq!(levenshtein_distance("same", "same", 1, 1, 1), 0);
    assert_eq!(levenshtein_distance("", "abc", 10, 1, 10), 30);
    assert_eq!(levenshtein_distance("abc", "", 10, 1, 10), 3);
    assert_eq!(levenshtein_distance("Working!!!!", "Working!!!", 10, 1, 10), 1);
    assert_eq!(levenshtein_distance("Working!!!!", "Working!!", 10, 1, 10), 2);
    assert_eq!(levenshtein_distance("kitten", "sitting", 1, 1, 1), 3);
}
