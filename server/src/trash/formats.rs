//! Custom formats: kept in the shape the Radarr and Sonarr APIs use (`name`, `includeCustomFormatWhenRenaming`,
//! `specifications` with their `fields`), and read into [`Format`]s that say whether a release matches.
//!
//! A format matches like it does in Radarr and Sonarr: its conditions are grouped by type; in every group each
//! required condition must hold and at least one condition must hold ("negate" turns a condition around).
//! Release names are matched with TRaSH's own regular expressions (.NET style, so look-arounds work), ignoring case.

use super::dotnet_regex::Regex;
use crate::release::Parsed;
use kit_server_core::{p, Db, Result, RowExt, SqliteDb};
use serde_json::{json, Value};
use std::sync::Arc;

/// The custom format kind for a media kind: anime shares the TV (Sonarr) formats.
pub fn format_kind(kind: &str) -> &'static str {
    if kind == "movie" {
        "movie"
    } else {
        "tv"
    }
}

/// What a release is, as custom format conditions ask it.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// the release's name
    pub title: String,
    /// its release group, "" when unknown
    pub group: String,
    /// its quality name (Sonarr's spelling, as the name parser gives it)
    pub quality: String,
    /// its resolution in lines, 0 when unknown
    pub resolution: i64,
    /// its size in bytes
    pub size: i64,
    /// its year, 0 when the name has none
    pub year: i64,
    /// for TV: 1 one episode, 2 several episodes, 3 a whole season; 0 for a movie
    pub release_type: i64,
    /// the audio languages its name says (the original language when it says none), as language ids
    pub languages: Vec<i64>,
    /// the original language's id (Japanese for anime, else English: Marquee doesn't know each title's)
    pub original: i64,
}

/// Words in release names that say which languages it has: (word, language id).
const LANGUAGE_WORDS: [(&str, i64); 22] = [
    ("english", 1),
    ("eng", 1),
    ("french", 2),
    ("truefrench", 2),
    ("vff", 2),
    ("vfq", 2),
    ("vf2", 2),
    ("spanish", 3),
    ("castellano", 3),
    ("latino", 3),
    ("german", 4),
    ("deutsch", 4),
    ("ger", 4),
    ("ita", 5),
    ("italian", 5),
    ("japanese", 8),
    ("jpn", 8),
    ("chinese", 10),
    ("mandarin", 10),
    ("cantonese", 10),
    ("korean", 21),
    ("kor", 21),
];

impl Facts {
    /// The facts of a release of `kind` named `title`, `size` bytes, read as `parsed`.
    pub fn new(kind: &str, title: &str, parsed: &Parsed, size: i64) -> Facts {
        let original = if kind == "anime" { 8 } else { 1 };
        let words: Vec<String> =
            title.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
        let mut languages: Vec<i64> = Vec::new();
        for (w, id) in LANGUAGE_WORDS {
            if words.iter().any(|x| x == w) && !languages.contains(&id) {
                languages.push(id);
            }
        }
        let lower = title.to_lowercase();
        let multi = words.iter().any(|w| w == "multi") || lower.contains("dual audio") || lower.contains("dual-audio");
        if multi || languages.is_empty() {
            for id in [original, if multi { 1 } else { original }] {
                if !languages.contains(&id) {
                    languages.push(id);
                }
            }
        }
        let release_type = match (parsed.season, parsed.full_season, parsed.episodes.len()) {
            (Some(_), true, _) => 3,
            (Some(_), false, n) if n > 1 => 2,
            (Some(_), false, _) => 1,
            (None, _, _) if parsed.absolutes.len() > 1 => 2,
            (None, _, _) if parsed.absolute.is_some() => 1,
            _ => 0,
        };
        Facts {
            title: title.to_string(),
            group: parsed.group.clone(),
            quality: if kind == "movie" { parsed.movie_quality_name } else { parsed.quality_name }.to_string(),
            resolution: parsed.resolution,
            size,
            year: parsed.year,
            release_type,
            languages,
            original,
        }
    }

    /// Radarr's source number for the quality: 5 DVD, 6 TV, 7 WEB-DL, 8 WEBRip, 9 Blu-ray, 0 unknown.
    fn radarr_source(&self) -> i64 {
        let q = self.quality.as_str();
        if q.starts_with("WEBDL") {
            7
        } else if q.starts_with("WEBRip") {
            8
        } else if q.starts_with("Bluray") || q.starts_with("Remux") {
            9
        } else if q.starts_with("HDTV") || q == "SDTV" || q == "Raw-HD" {
            6
        } else if q.starts_with("DVD") {
            5
        } else {
            0
        }
    }

    /// Sonarr's source number: 1 TV, 2 raw TV, 3 WEB-DL, 4 WEBRip, 5 DVD, 6 Blu-ray, 7 Blu-ray remux, 0 unknown.
    fn sonarr_source(&self) -> i64 {
        let q = self.quality.as_str();
        if q.starts_with("WEBDL") {
            3
        } else if q.starts_with("WEBRip") {
            4
        } else if q.contains("Remux") {
            7
        } else if q.starts_with("Bluray") {
            6
        } else if q == "Raw-HD" {
            2
        } else if q.starts_with("HDTV") || q == "SDTV" {
            1
        } else if q.starts_with("DVD") {
            5
        } else {
            0
        }
    }

    /// Radarr's quality modifier: 3 raw HD, 5 remux, 0 none.
    fn modifier(&self) -> i64 {
        if self.quality.contains("Remux") {
            5
        } else if self.quality == "Raw-HD" {
            3
        } else {
            0
        }
    }
}

/// One condition of a custom format.
#[derive(Clone, Debug)]
enum Test {
    /// the release name matches
    Title(Regex),
    /// the release group matches
    Group(Regex),
    /// the source is this (Radarr's numbers when `radarr`, else Sonarr's)
    Source(i64, bool),
    /// the resolution is this
    Resolution(i64),
    /// Radarr's quality modifier is this
    Modifier(i64),
    /// Sonarr's release type is this
    ReleaseType(i64),
    /// the size is in this range (GB)
    Size(f64, f64),
    /// the year is in this range
    Year(i64, i64),
    /// the release has this language (-1 any, -2 the original); the bool: every language but this one
    Language(i64, bool),
    /// a condition Marquee can't check (indexer flags, a broken expression): never holds
    Never,
}

/// One condition: its type (conditions are grouped by it), whether it is turned around, whether it must hold.
#[derive(Clone, Debug)]
struct Spec {
    /// the implementation name (`ReleaseTitleSpecification`…)
    kind: String,
    /// true: the condition holds when the test fails
    negate: bool,
    /// true: the format can't match without it
    required: bool,
    /// the test
    test: Test,
}

/// A custom format, ready to match releases.
#[derive(Clone, Debug)]
pub struct Format {
    /// its name, how profiles refer to it
    pub name: String,
    /// its conditions
    specs: Vec<Spec>,
}

/// A field of a specification: `fields` is a list of `{name, value}` (the APIs) or an object (TRaSH's files).
fn field<'a>(spec: &'a Value, name: &str) -> &'a Value {
    match &spec["fields"] {
        Value::Array(list) => list.iter().find(|f| f["name"].as_str() == Some(name)).map(|f| &f["value"]).unwrap_or(&Value::Null),
        Value::Object(o) => o.get(name).unwrap_or(&Value::Null),
        _ => &Value::Null,
    }
}

/// A number field (numbers may come as text).
fn num(v: &Value) -> f64 {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0)
}

/// A case-insensitive expression, or `None` when it can't be read.
fn regex(v: &Value) -> Option<Regex> {
    Regex::new(v.as_str()?)
}

impl Format {
    /// Reads a custom format in the APIs' (or TRaSH's) shape.
    pub fn read(body: &Value) -> Format {
        let specs = body["specifications"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|s| {
                        let kind = s["implementation"].as_str().unwrap_or("").to_string();
                        let value = field(s, "value");
                        let test = match kind.as_str() {
                            "ReleaseTitleSpecification" | "EditionSpecification" => regex(value).map(Test::Title),
                            "ReleaseGroupSpecification" => regex(value).map(Test::Group),
                            "SourceSpecification" => Some(Test::Source(num(value) as i64, true)),
                            "ResolutionSpecification" => Some(Test::Resolution(num(value) as i64)),
                            "QualityModifierSpecification" => Some(Test::Modifier(num(value) as i64)),
                            "ReleaseTypeSpecification" => Some(Test::ReleaseType(num(value) as i64)),
                            "SizeSpecification" => Some(Test::Size(num(field(s, "min")), num(field(s, "max")))),
                            "YearSpecification" => Some(Test::Year(num(field(s, "min")) as i64, num(field(s, "max")) as i64)),
                            "LanguageSpecification" => {
                                Some(Test::Language(num(value) as i64, field(s, "exceptLanguage").as_bool().unwrap_or(false)))
                            }
                            _ => None,
                        };
                        Spec {
                            kind,
                            negate: s["negate"].as_bool().unwrap_or(false),
                            required: s["required"].as_bool().unwrap_or(false),
                            test: test.unwrap_or(Test::Never),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Format { name: body["name"].as_str().unwrap_or("").to_string(), specs }
    }

    /// Marks the source conditions as Sonarr's numbering (TV and anime formats).
    pub fn sonarr(mut self) -> Format {
        for s in &mut self.specs {
            if let Test::Source(v, _) = s.test {
                s.test = Test::Source(v, false);
            }
        }
        self
    }

    /// Whether a release with `facts` matches.
    pub fn matches(&self, facts: &Facts) -> bool {
        if self.specs.is_empty() {
            return false;
        }
        let mut kinds: Vec<&str> = self.specs.iter().map(|s| s.kind.as_str()).collect();
        kinds.sort_unstable();
        kinds.dedup();
        kinds.into_iter().all(|k| {
            let results: Vec<(bool, bool)> =
                self.specs.iter().filter(|s| s.kind == k).map(|s| (s.required, test(&s.test, facts) != s.negate)).collect();
            !results.iter().any(|(req, ok)| *req && !ok) && results.iter().any(|(_, ok)| *ok)
        })
    }
}

/// Whether a test holds for `f` (before "negate").
fn test(t: &Test, f: &Facts) -> bool {
    match t {
        Test::Title(r) => r.is_match(&f.title),
        Test::Group(r) => !f.group.is_empty() && r.is_match(&f.group),
        Test::Source(v, true) => f.radarr_source() == *v,
        Test::Source(v, false) => f.sonarr_source() == *v,
        Test::Resolution(v) => f.resolution == *v,
        Test::Modifier(v) => f.modifier() == *v,
        Test::ReleaseType(v) => f.release_type == *v,
        Test::Size(min, max) => {
            let gb = f.size as f64 / 1_073_741_824.0;
            gb > *min && gb <= *max
        }
        Test::Year(min, max) => f.year >= *min && f.year <= *max,
        Test::Language(v, except) => {
            let id = if *v == -2 { f.original } else { *v };
            if *v == -1 {
                true
            } else if *except {
                f.languages.iter().any(|l| *l != id)
            } else {
                f.languages.contains(&id)
            }
        }
        Test::Never => false,
    }
}

/// A TRaSH custom format file as the APIs' shape (no id): `fields` becomes a list of `{name, value}`.
pub fn from_trash(cf: &Value) -> Value {
    let specs: Vec<Value> = cf["specifications"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|s| {
                    let fields: Vec<Value> = s["fields"]
                        .as_object()
                        .map(|o| o.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect())
                        .unwrap_or_default();
                    json!({
                        "name": s["name"],
                        "implementation": s["implementation"],
                        "negate": s["negate"].as_bool().unwrap_or(false),
                        "required": s["required"].as_bool().unwrap_or(false),
                        "fields": fields,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "name": cf["name"],
        "includeCustomFormatWhenRenaming": cf["includeCustomFormatWhenRenaming"].as_bool().unwrap_or(false),
        "specifications": specs,
    })
}

/// A stored body with `id` set.
fn with_id(body: &str, id: i64) -> Value {
    let mut v: Value = serde_json::from_str(body).unwrap_or_else(|_| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("id".into(), json!(id));
    }
    v
}

/// Every custom format of a media kind, in the APIs' shape with ids.
pub fn all(db: &SqliteDb, kind: &str) -> Result<Vec<Value>> {
    Ok(db
        .query("SELECT id, body FROM custom_formats WHERE kind=? ORDER BY id", p![format_kind(kind)])?
        .iter()
        .map(|r| with_id(&r.s("body"), r.i("id")))
        .collect())
}

/// The custom format `id`, with its id.
pub fn one(db: &SqliteDb, id: i64) -> Result<Option<Value>> {
    Ok(db.one("SELECT id, body FROM custom_formats WHERE id=?", p![id])?.map(|r| with_id(&r.s("body"), r.i("id"))))
}

/// The id of the custom format of `kind` named `name`.
pub fn id_by_name(db: &SqliteDb, kind: &str, name: &str) -> Result<Option<i64>> {
    Ok(db.one("SELECT id FROM custom_formats WHERE kind=? AND name=?", p![format_kind(kind), name])?.map(|r| r.i("id")))
}

/// Saves a custom format body (no id) of `kind`: replaces `id` when given, else adds it; its id.
pub fn save(db: &SqliteDb, kind: &str, id: Option<i64>, body: &Value) -> Result<i64> {
    let name = body["name"].as_str().unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(kit_server_core::err(400, "A custom format needs a name"));
    }
    let mut body = body.clone();
    if let Some(o) = body.as_object_mut() {
        o.remove("id");
    }
    match id {
        Some(id) => {
            db.execute("UPDATE custom_formats SET name=?, body=? WHERE id=?", p![name, body.to_string(), id])?;
            Ok(id)
        }
        None => db.execute("INSERT INTO custom_formats (kind, name, body) VALUES (?,?,?)", p![format_kind(kind), name, body.to_string()]),
    }
}

/// Formats compiled before, by a fingerprint of the bodies they were read from (reading TRaSH's hundreds of
/// expressions takes a moment, and searches ask for them often).
static COMPILED: std::sync::Mutex<Option<std::collections::HashMap<String, Arc<Vec<Format>>>>> = std::sync::Mutex::new(None);

/// Every custom format of a media kind, ready to match releases.
pub fn compiled(db: &SqliteDb, kind: &str) -> Arc<Vec<Format>> {
    let sonarr = format_kind(kind) == "tv";
    let bodies = all(db, kind).unwrap_or_default();
    let key = format!("{}:{}", format_kind(kind), super::apply::fingerprint(&json!(bodies)));
    let mut cache = COMPILED.lock().unwrap_or_else(|p| p.into_inner());
    let cache = cache.get_or_insert_with(Default::default);
    if let Some(hit) = cache.get(&key) {
        return hit.clone();
    }
    let list: Vec<Format> = bodies
        .iter()
        .map(|b| {
            let f = Format::read(b);
            if sonarr {
                f.sonarr()
            } else {
                f
            }
        })
        .collect();
    let list = Arc::new(list);
    if cache.len() > 16 {
        cache.clear();
    }
    cache.insert(key, list.clone());
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::parse_for;

    fn facts(kind: &str, title: &str) -> Facts {
        Facts::new(kind, title, &parse_for(kind, title), 5_000_000_000)
    }

    #[test]
    fn trash_formats_match_like_radarr() {
        let tier = from_trash(&json!({
            "name": "WEB Tier 01",
            "specifications": [
                {"name": "FLUX", "implementation": "ReleaseGroupSpecification", "negate": false, "required": false, "fields": {"value": "^(FLUX)$"}},
                {"name": "NTb", "implementation": "ReleaseGroupSpecification", "negate": false, "required": false, "fields": {"value": "^(NTb)$"}},
                {"name": "WEBDL", "implementation": "SourceSpecification", "negate": false, "required": false, "fields": {"value": 7}},
                {"name": "WEBRIP", "implementation": "SourceSpecification", "negate": false, "required": false, "fields": {"value": 8}}
            ]
        }));
        let f = Format::read(&tier);
        assert!(f.matches(&facts("movie", "Movie.2020.1080p.AMZN.WEB-DL.DDP5.1.H.264-FLUX")));
        assert!(!f.matches(&facts("movie", "Movie.2020.1080p.BluRay.x264-FLUX")), "the source group must match too");
        assert!(!f.matches(&facts("movie", "Movie.2020.1080p.WEB-DL.H.264-OTHER")));
    }

    #[test]
    fn negated_required_conditions_and_look_arounds_work() {
        let f = Format::read(&from_trash(&json!({
            "name": "x265 (HD)",
            "specifications": [
                {"name": "x265", "implementation": "ReleaseTitleSpecification", "negate": false, "required": true, "fields": {"value": "[xh][ ._-]?265|\\bHEVC(\\b|\\d)"}},
                {"name": "Not 2160p", "implementation": "ResolutionSpecification", "negate": true, "required": true, "fields": {"value": 2160}}
            ]
        })));
        assert!(f.matches(&facts("movie", "Movie.2020.1080p.BluRay.x265-GRP")));
        assert!(!f.matches(&facts("movie", "Movie.2020.2160p.BluRay.x265-GRP")));
        let la = Format::read(&from_trash(&json!({
            "name": "Not a sample",
            "specifications": [{"name": "t", "implementation": "ReleaseTitleSpecification", "required": true, "fields": {"value": "\\bmovie(?!\\.sample)"}}]
        })));
        assert!(la.matches(&facts("movie", "Movie.2020.1080p")));
        assert!(!la.matches(&facts("movie", "Movie.sample.1080p")));
    }

    #[test]
    fn every_trash_expression_can_be_read() {
        let snap = crate::trash::snapshot::Snapshot::bundled();
        let mut broken = Vec::new();
        for app in ["radarr", "sonarr"] {
            for (key, cf) in snap.folder(app, "cf") {
                for s in cf["specifications"].as_array().into_iter().flatten() {
                    let regex_kind = matches!(
                        s["implementation"].as_str(),
                        Some("ReleaseTitleSpecification" | "ReleaseGroupSpecification" | "EditionSpecification")
                    );
                    if regex_kind && regex(&s["fields"]["value"]).is_none() {
                        broken.push(format!("{key}: {}", s["fields"]["value"]));
                    }
                }
            }
        }
        assert!(broken.is_empty(), "{broken:#?}");
    }

    #[test]
    fn languages_come_from_the_name() {
        assert_eq!(facts("movie", "Movie.2020.1080p.WEB-DL-GRP").languages, vec![1]);
        assert_eq!(facts("movie", "Movie.2020.German.1080p.WEB-DL-GRP").languages, vec![4]);
        assert_eq!(facts("anime", "[SubsPlease] Show - 01 (1080p)").languages, vec![8]);
        let dual = facts("anime", "[Group] Show - 01 (1080p) [Dual Audio]").languages;
        assert!(dual.contains(&8) && dual.contains(&1));
        let not_english = Format::read(&from_trash(&json!({
            "name": "Language: Not English",
            "specifications": [{"name": "Not English", "implementation": "LanguageSpecification", "negate": true, "required": false, "fields": {"value": 1}}]
        })));
        assert!(not_english.matches(&facts("movie", "Movie.2020.German.1080p.WEB-DL-GRP")));
        assert!(!not_english.matches(&facts("movie", "Movie.2020.1080p.WEB-DL-GRP")));
    }

    #[test]
    fn sonarr_sources_and_release_types() {
        let f = Format::read(&from_trash(&json!({
            "name": "Season Pack",
            "specifications": [{"name": "Season Pack", "implementation": "ReleaseTypeSpecification", "required": false, "fields": {"value": 3}}]
        })))
        .sonarr();
        assert!(f.matches(&facts("tv", "Show.S01.1080p.WEB-DL-GRP")));
        assert!(!f.matches(&facts("tv", "Show.S01E01.1080p.WEB-DL-GRP")));
        let web = Format::read(&from_trash(&json!({
            "name": "WEB", "specifications": [{"name": "WEB", "implementation": "SourceSpecification", "required": true, "fields": {"value": 3}}]
        })))
        .sonarr();
        assert!(web.matches(&facts("tv", "Show.S01E01.1080p.WEB-DL-GRP")));
    }
}
