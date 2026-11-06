//! Scene mappings: the other names a series goes by in release names (from Sonarr's services and from TheXEM),
//! and which season those names mean. A port of Sonarr's `DataAugmentation/Scene` (`SceneMapping`,
//! `SceneMappingService`, `SceneMappingRepository`, `SearchMode`, `InvalidSceneMappingException`,
//! `ServicesProvider` and `SceneMappingProxy`) at the commit in `UPSTREAM.lock`.
//!
//! The mappings live in the `scene_mappings` table. [`SceneMappingService`] keeps them in memory by cleaned
//! title and by TheTVDB id, as Sonarr's caches do, and reads the table again only when those are empty or the
//! mappings were fetched again.

use std::collections::HashMap;
use std::sync::Mutex;

use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::parser::clr_regex::ClrRegex;
use crate::parser::episode::{clean_series_title, simplify_title};

/// Where Sonarr's services keep the scene mapping list.
pub const SERVICES_URL: &str = "https://services.sonarr.tv/v1/scenemapping";

/// The `type` of mappings from Sonarr's services (Sonarr names it after the provider class).
pub const SERVICES: &str = "ServicesProvider";

/// The `type` of mappings from TheXEM's name list.
pub const XEM: &str = "XemService";

/// Sonarr's `SearchMode` flags: how a mapping is used when searching.
pub mod search_mode {
    /// Search the way the indexer prefers.
    pub const DEFAULT: i32 = 0;
    /// Search by id.
    pub const SEARCH_ID: i32 = 1;
    /// Search by title.
    pub const SEARCH_TITLE: i32 = 2;
}

/// One other name of a series, as Sonarr's `SceneMapping` has it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SceneMapping {
    /// The row id in `scene_mappings` (0 before it is stored).
    #[serde(skip)]
    pub id: i64,
    /// The name as it appears in release names.
    #[serde(deserialize_with = "text")]
    pub title: String,
    /// The name cleaned the way parsed series titles are, used to look it up.
    #[serde(deserialize_with = "text")]
    pub parse_term: String,
    /// The name to search indexers with.
    #[serde(rename = "searchTitle", deserialize_with = "text")]
    pub search_term: String,
    /// TheTVDB's id of the series this name means.
    pub tvdb_id: i64,
    /// TheTVDB's season this name means (none or -1 for every season).
    #[serde(rename = "season")]
    pub season_number: Option<i32>,
    /// The scene's season this name means (none or -1 for every season).
    pub scene_season_number: Option<i32>,
    /// Where the season numbers come from (`tvdb`, `unknown`, ...).
    #[serde(deserialize_with = "text")]
    pub scene_origin: String,
    /// How the name is used when searching ([`search_mode`] flags), if the list says.
    #[serde(deserialize_with = "flags")]
    pub search_mode: Option<i32>,
    /// A note from the list's keepers.
    #[serde(deserialize_with = "text")]
    pub comment: String,
    /// A pattern the release name must match for this name to count (empty for none).
    #[serde(deserialize_with = "text")]
    pub filter_regex: String,
    /// Where the mapping came from: [`SERVICES`] or [`XEM`].
    #[serde(rename = "type", deserialize_with = "text")]
    pub kind: String,
}

/// Reads a JSON string that may be null as text (null is empty, as Sonarr's checks treat them alike).
fn text<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// Reads `searchMode` as Json.NET reads a flags enum: a number, or names such as `searchID, searchTitle`.
fn flags<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<i32>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().map(|n| n as i32),
        Value::String(s) => {
            let mut all = 0;
            for name in s.split(',').map(str::trim) {
                all |= match name.to_ascii_lowercase().as_str() {
                    "default" => search_mode::DEFAULT,
                    "searchid" => search_mode::SEARCH_ID,
                    "searchtitle" => search_mode::SEARCH_TITLE,
                    other => other.parse().map_err(|_| serde::de::Error::custom(format!("unknown search mode {other}")))?,
                };
            }
            Some(all)
        }
        _ => None,
    })
}

/// Sonarr's `InvalidSceneMappingException`: one name points at more than one series.
#[derive(Clone, Debug, PartialEq)]
pub struct InvalidSceneMapping {
    /// TheTVDB ids of every mapping that matched.
    pub tvdb_ids: Vec<i64>,
    /// The release name being read.
    pub release_title: String,
}

impl std::fmt::Display for InvalidSceneMapping {
    /// Sonarr's message for it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<String> = self.tvdb_ids.iter().map(i64::to_string).collect();
        write!(
            f,
            "Scene Mappings contains a conflict for tvdbids {}. Please notify Sonarr developers. ({})",
            ids.join(","),
            self.release_title
        )
    }
}

/// Where mappings are kept: Sonarr's `ISceneMappingRepository`. The app uses [`SqliteMappings`]; tests use
/// their own to count reads.
pub trait MappingStore: Send + Sync {
    /// Every mapping.
    fn all(&self) -> Result<Vec<SceneMapping>>;
    /// Removes the mappings of one `type`.
    fn clear(&self, kind: &str) -> Result<()>;
    /// Adds mappings.
    fn insert_many(&self, list: &[SceneMapping]) -> Result<()>;
}

/// The `scene_mappings` table.
pub struct SqliteMappings(pub SqliteDb);

/// Makes the `scene_mappings` table.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS scene_mappings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            parse_term TEXT NOT NULL,
            search_term TEXT NOT NULL,
            tvdb_id INTEGER NOT NULL,
            season_number INTEGER,
            scene_season_number INTEGER,
            scene_origin TEXT NOT NULL DEFAULT '',
            search_mode INTEGER,
            comment TEXT NOT NULL DEFAULT '',
            filter_regex TEXT NOT NULL DEFAULT '',
            type TEXT NOT NULL,
            fetched TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS scene_mappings_type ON scene_mappings(type);",
    )
    .map_err(|e| anyhow::anyhow!("The scene mapping table can't be made: {}", e.detail))
}

/// A table row as a mapping.
fn from_row(r: &Row) -> SceneMapping {
    let num = |k: &str| r.get(k).and_then(Value::as_i64).map(|n| n as i32);
    SceneMapping {
        id: r.i("id"),
        title: r.s("title"),
        parse_term: r.s("parse_term"),
        search_term: r.s("search_term"),
        tvdb_id: r.i("tvdb_id"),
        season_number: num("season_number"),
        scene_season_number: num("scene_season_number"),
        scene_origin: r.s("scene_origin"),
        search_mode: num("search_mode"),
        comment: r.s("comment"),
        filter_regex: r.s("filter_regex"),
        kind: r.s("type"),
    }
}

impl MappingStore for SqliteMappings {
    fn all(&self) -> Result<Vec<SceneMapping>> {
        Ok(self.0.query("SELECT * FROM scene_mappings ORDER BY id", p![])?.iter().map(from_row).collect())
    }

    fn clear(&self, kind: &str) -> Result<()> {
        self.0.execute("DELETE FROM scene_mappings WHERE type=?", p![kind])?;
        Ok(())
    }

    fn insert_many(&self, list: &[SceneMapping]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        for m in list {
            self.0.execute(
                "INSERT INTO scene_mappings (title, parse_term, search_term, tvdb_id, season_number, scene_season_number,
                    scene_origin, search_mode, comment, filter_regex, type, fetched) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                p![
                    m.title,
                    m.parse_term,
                    m.search_term,
                    m.tvdb_id,
                    m.season_number,
                    m.scene_season_number,
                    m.scene_origin,
                    m.search_mode,
                    m.comment,
                    m.filter_regex,
                    m.kind,
                    now
                ],
            )?;
        }
        Ok(())
    }
}

/// When the mappings were last fetched (RFC 3339), if ever.
pub fn last_fetched(db: &SqliteDb) -> Option<String> {
    let r = db.one("SELECT MAX(fetched) AS f FROM scene_mappings", p![]).ok()??;
    let f = r.s("f");
    (!f.is_empty()).then_some(f)
}

/// The in-memory lookups: Sonarr's `tvdb_id` (by cleaned name) and `find_tvdb_id` (by TheTVDB id) caches.
#[derive(Default)]
struct Caches {
    /// The mappings by their cleaned name (`parse_term`).
    by_parse_term: HashMap<String, Vec<SceneMapping>>,
    /// The mappings by TheTVDB id.
    by_tvdb_id: HashMap<i64, Vec<SceneMapping>>,
}

/// Sonarr's `SceneMappingService`: finds the series and season a scene name means.
pub struct SceneMappingService {
    /// Where the mappings are kept.
    store: Box<dyn MappingStore>,
    /// The in-memory lookups.
    caches: Mutex<Caches>,
    /// Whether the mappings were fetched since the server started (Sonarr's `_updatedAfterStartup`).
    updated_after_startup: Mutex<bool>,
}

impl SceneMappingService {
    /// A service over `store`, with empty lookups.
    pub fn new(store: Box<dyn MappingStore>) -> SceneMappingService {
        SceneMappingService { store, caches: Mutex::new(Caches::default()), updated_after_startup: Mutex::new(false) }
    }

    /// Whether the mappings were fetched since the server started.
    pub fn updated_after_startup(&self) -> bool {
        *self.updated_after_startup.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The names to search for a series with in the given TheTVDB and scene seasons: the mappings of those
    /// seasons and the ones for every season (unless their seasons come from TheTVDB), with Latin-1 letters
    /// only, each once whatever its case. Sonarr's `GetSceneNames`.
    pub fn get_scene_names(&self, tvdb_id: i64, seasons: &[i32], scene_seasons: &[i32]) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for n in self.find_by_tvdb_id(tvdb_id) {
            let season = n.season_number.unwrap_or(-1);
            let scene = n.scene_season_number.unwrap_or(-1);
            let wanted =
                seasons.contains(&season) || scene_seasons.contains(&scene) || (season == -1 && scene == -1 && n.scene_origin != "tvdb");
            if !wanted || !is_english(&n.search_term) {
                continue;
            }
            let lower = n.search_term.to_lowercase();
            if !names.iter().any(|have| have.to_lowercase() == lower) {
                names.push(n.search_term);
            }
        }
        names
    }

    /// TheTVDB id of the series a parsed name means. Sonarr's `FindTvdbId`.
    pub fn find_tvdb_id(
        &self,
        series_title: &str,
        release_title: Option<&str>,
        scene_season: i32,
    ) -> std::result::Result<Option<i64>, InvalidSceneMapping> {
        Ok(self.find_scene_mapping(series_title, release_title, scene_season)?.map(|m| m.tvdb_id))
    }

    /// Every mapping of a series. Sonarr's `FindByTvdbId`.
    pub fn find_by_tvdb_id(&self, tvdb_id: i64) -> Vec<SceneMapping> {
        let empty = self.caches.lock().unwrap_or_else(|e| e.into_inner()).by_tvdb_id.is_empty();
        if empty {
            self.refresh_cache();
        }
        self.caches.lock().unwrap_or_else(|e| e.into_inner()).by_tvdb_id.get(&tvdb_id).cloned().unwrap_or_default()
    }

    /// The mapping a parsed name means in a scene season (-1 for any), or an error when it means more than one
    /// series. Sonarr's `FindSceneMapping`.
    pub fn find_scene_mapping(
        &self,
        series_title: &str,
        release_title: Option<&str>,
        scene_season: i32,
    ) -> std::result::Result<Option<SceneMapping>, InvalidSceneMapping> {
        if series_title.trim().is_empty() {
            return Ok(None);
        }
        let Some(mappings) = self.find_mappings(series_title, release_title) else { return Ok(None) };
        let mappings = filter_by_season(mappings, scene_season);
        let mut distinct: Vec<&SceneMapping> = Vec::new();
        for m in &mappings {
            if !distinct.iter().any(|d| d.tvdb_id == m.tvdb_id) {
                distinct.push(m);
            }
        }
        match distinct.len() {
            0 => Ok(None),
            1 => {
                tracing::debug!("Found scene mapping for: {series_title}. TVDB ID for mapping: {}", distinct[0].tvdb_id);
                Ok(Some(distinct[0].clone()))
            }
            _ => Err(InvalidSceneMapping {
                tvdb_ids: mappings.iter().map(|m| m.tvdb_id).collect(),
                release_title: release_title.unwrap_or("").to_string(),
            }),
        }
    }

    /// The scene season a parsed name means, if its mapping says. Sonarr's `GetSceneSeasonNumber`.
    pub fn get_scene_season_number(
        &self,
        series_title: &str,
        release_title: Option<&str>,
    ) -> std::result::Result<Option<i32>, InvalidSceneMapping> {
        Ok(self.find_scene_mapping(series_title, release_title, -1)?.and_then(|m| m.scene_season_number))
    }

    /// Stores freshly fetched mappings: for each provider (its `type` and what it answered), a list that came
    /// back with entries replaces that provider's old ones, leaving out entries without a title or search name;
    /// an empty list or an error keeps the old ones. Then the lookups are read again. Sonarr's `UpdateMappings`.
    pub fn update_with(&self, answers: Vec<(&str, std::result::Result<Vec<SceneMapping>, String>)>) {
        tracing::info!("Updating Scene mappings");
        *self.updated_after_startup.lock().unwrap_or_else(|e| e.into_inner()) = true;
        for (kind, answer) in answers {
            let mut mappings = match answer {
                Ok(list) if !list.is_empty() => list,
                Ok(_) => {
                    tracing::warn!("Received empty list of mapping. will not update");
                    continue;
                }
                Err(e) => {
                    tracing::error!("Failed to Update Scene Mappings: {e}");
                    continue;
                }
            };
            if let Err(e) = self.store.clear(kind) {
                tracing::error!("Failed to Update Scene Mappings: {}", e.detail);
                continue;
            }
            mappings.retain(|m| {
                let bad = m.title.trim().is_empty() || m.search_term.trim().is_empty();
                if bad {
                    tracing::warn!("Invalid scene mapping found for: {}, skipping", m.tvdb_id);
                }
                !bad
            });
            for m in &mut mappings {
                m.parse_term = clean_series_title(&m.title);
                m.kind = kind.to_string();
            }
            if let Err(e) = self.store.insert_many(&mappings) {
                tracing::error!("Failed to Update Scene Mappings: {}", e.detail);
            }
        }
        self.refresh_cache();
    }

    /// Fetches the mappings from Sonarr's services and TheXEM and stores them ([`Self::update_with`]).
    pub async fn update(&self, http: &reqwest::Client) {
        let services = fetch_services(http).await.map_err(|e| e.detail);
        let xem = super::xem::fetch_names(http).await.map_err(|e| e.detail);
        self.update_with(vec![(SERVICES, services), (XEM, xem)]);
    }

    /// The candidates for a parsed name: the ones with its cleaned name, narrowed by the release name's filters,
    /// then the ones with exactly this name, else the ones with the closest name. Sonarr's `FindMappings`.
    fn find_mappings(&self, series_title: &str, release_title: Option<&str>) -> Option<Vec<SceneMapping>> {
        let empty = self.caches.lock().unwrap_or_else(|e| e.into_inner()).by_parse_term.is_empty();
        if empty {
            self.refresh_cache();
        }
        let candidates =
            self.caches.lock().unwrap_or_else(|e| e.into_inner()).by_parse_term.get(&clean_series_title(series_title)).cloned()?;
        let mut candidates = filter_by_release(candidates, release_title);
        if candidates.len() <= 1 {
            return Some(candidates);
        }
        let mut exact: Vec<SceneMapping> = candidates.iter().filter(|v| v.title == series_title).cloned().collect();
        if !exact.is_empty() {
            exact.sort_by_key(|v| std::cmp::Reverse(v.season_number));
            return Some(exact);
        }
        let mut ranked: Vec<(i32, &SceneMapping)> =
            candidates.iter().map(|v| (levenshtein_distance(series_title, &v.title, 10, 1, 10), v)).collect();
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.season_number.cmp(&a.1.season_number)));
        let closest = ranked[0].1.title.clone();
        candidates.retain(|v| v.title == closest);
        Some(candidates)
    }

    /// Reads every mapping from the store into the lookups. Sonarr's `RefreshCache`.
    fn refresh_cache(&self) {
        let all = match self.store.all() {
            Ok(all) => all,
            Err(e) => {
                tracing::warn!("The scene mappings can't be read: {}", e.detail);
                return;
            }
        };
        let mut caches = Caches::default();
        for m in all {
            caches.by_parse_term.entry(m.parse_term.clone()).or_default().push(m.clone());
            caches.by_tvdb_id.entry(m.tvdb_id).or_default().push(m);
        }
        *self.caches.lock().unwrap_or_else(|e| e.into_inner()) = caches;
    }
}

/// Keeps the candidates whose filter matches the release name, or, when none does (or there is no release
/// name), the ones without a filter. Sonarr's `FilterSceneMappings(candidates, releaseTitle)`.
fn filter_by_release(candidates: Vec<SceneMapping>, release_title: Option<&str>) -> Vec<SceneMapping> {
    let (filtered, normal): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|v| !v.filter_regex.trim().is_empty());
    let Some(release) = release_title.filter(|r| !r.trim().is_empty()) else { return normal };
    let simple = simplify_title(release);
    let matched: Vec<SceneMapping> = filtered
        .into_iter()
        .filter(|v| ClrRegex::try_new(&v.filter_regex, false).map(|re| re.is_match(&simple)).unwrap_or(false))
        .collect();
    if matched.is_empty() {
        normal
    } else {
        matched
    }
}

/// Keeps, for a scene season, the season-bound candidates that start at or before it (the latest one per name),
/// or the ones for every season when there are none or the season is unknown (-1). Sonarr's
/// `FilterSceneMappings(candidates, sceneSeasonNumber)`.
fn filter_by_season(candidates: Vec<SceneMapping>, scene_season: i32) -> Vec<SceneMapping> {
    let (filtered, normal): (Vec<_>, Vec<_>) =
        candidates.into_iter().partition(|v| v.scene_season_number.unwrap_or(-1) != -1 && v.season_number.unwrap_or(-1) != -1);
    if scene_season == -1 || filtered.is_empty() {
        return normal;
    }
    let mut groups: Vec<(String, Vec<SceneMapping>)> = Vec::new();
    for v in filtered.into_iter().filter(|v| v.scene_season_number.is_some_and(|s| s <= scene_season)) {
        match groups.iter_mut().find(|(t, _)| *t == v.title) {
            Some((_, g)) => g.push(v),
            None => groups.push((v.title.clone(), vec![v])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(_, mut g)| {
            g.sort_by(|a, b| b.scene_season_number.cmp(&a.scene_season_number).then(b.season_number.cmp(&a.season_number)));
            g.into_iter().next()
        })
        .collect()
}

/// Whether every letter is Latin-1 (code 255 or below). Sonarr's `IsEnglish`.
fn is_english(title: &str) -> bool {
    title.encode_utf16().all(|c| c <= 255)
}

/// The cost of turning `text` into `other` by inserting, deleting and swapping UTF-16 letters, each with its own
/// cost. Sonarr's `LevenshteinDistance`.
pub fn levenshtein_distance(text: &str, other: &str, cost_insert: i32, cost_delete: i32, cost_substitute: i32) -> i32 {
    if text == other {
        return 0;
    }
    let text: Vec<u16> = text.encode_utf16().collect();
    let other: Vec<u16> = other.encode_utf16().collect();
    if text.is_empty() {
        return other.len() as i32 * cost_insert;
    }
    if other.is_empty() {
        return text.len() as i32 * cost_delete;
    }
    let mut matrix: Vec<i32> = (0..=other.len() as i32).map(|i| i * cost_insert).collect();
    for &t in &text {
        let mut top_left = matrix[0];
        matrix[0] += cost_delete;
        for (j, &o) in other.iter().enumerate() {
            let top = matrix[j];
            let left = matrix[j + 1];
            let sum_ins = top + cost_insert;
            let sum_del = left + cost_delete;
            let sum_sub = top_left + if t == o { 0 } else { cost_substitute };
            top_left = matrix[j + 1];
            matrix[j + 1] = sum_ins.min(sum_del).min(sum_sub);
        }
    }
    matrix[other.len()]
}

/// Fetches Sonarr's services' scene mapping list. Sonarr's `SceneMappingProxy.Fetch`.
pub async fn fetch_services(http: &reqwest::Client) -> Result<Vec<SceneMapping>> {
    let r = http
        .get(SERVICES_URL)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| err(502, "Sonarr's scene name list can't be fetched right now"))?;
    if !r.status().is_success() {
        return Err(err(502, format!("Sonarr's scene name list answered {}", r.status().as_u16())));
    }
    r.json().await.map_err(|_| err(502, "Sonarr's scene name list can't be read"))
}
