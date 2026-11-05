//! TRaSH's files turned into what Marquee keeps: quality profiles in the Radarr/Sonarr API shape, quality sizes
//! and naming. Nothing here touches the database; [`super::apply`] writes the results.

use super::snapshot::Snapshot;
use serde_json::{json, Map, Value};

/// The TRaSH folder (`radarr` or `sonarr`) a media kind's files come from.
pub fn app_of(kind: &str) -> &'static str {
    if kind == "movie" {
        "radarr"
    } else {
        "sonarr"
    }
}

/// True for `true` and `"true"` (TRaSH writes both).
fn truthy(v: &Value) -> bool {
    v.as_bool().unwrap_or(false) || v.as_str() == Some("true")
}

/// The custom formats (trash ids) a TRaSH profile uses: its own `formatItems`, plus the formats of every
/// custom format group that is on by default and made for this profile (all its formats when they are required,
/// else the ones marked default).
pub fn profile_formats(snap: &Snapshot, app: &str, profile: &Value) -> Vec<String> {
    let mut out: Vec<String> =
        profile["formatItems"].as_object().map(|o| o.values().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let pid = profile["trash_id"].as_str().unwrap_or("");
    for (_, group) in snap.folder(app, "cf-groups") {
        let for_us = group["quality_profiles"]["include"].as_object().is_some_and(|o| o.values().any(|v| v.as_str() == Some(pid)));
        if !for_us || !truthy(&group["default"]) {
            continue;
        }
        for cf in group["custom_formats"].as_array().into_iter().flatten() {
            if truthy(&cf["required"]) || truthy(&cf["default"]) {
                if let Some(id) = cf["trash_id"].as_str() {
                    out.push(id.to_string());
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|id| seen.insert(id.clone()));
    out
}

/// A custom format's score in a profile: its score for the profile's score set, else its default.
pub fn score(cf: &Value, score_set: Option<&str>) -> i64 {
    let scores = &cf["trash_scores"];
    score_set.and_then(|s| scores[s].as_i64()).or_else(|| scores["default"].as_i64()).unwrap_or(0)
}

/// Radarr's language object for TRaSH's language name (`Original`, `Any`, `English`…).
fn language(name: &str, languages: &[Value]) -> Value {
    languages
        .iter()
        .find(|l| l["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .map(|l| json!({ "id": l["id"], "name": l["name"] }))
        .unwrap_or_else(|| json!({ "id": -1, "name": "Any" }))
}

/// A TRaSH quality profile in the APIs' shape (no id).
///
/// - `qualities`: the kind's quality list (`quality_definitions` bodies), to find each quality's object;
/// - `formats`: (custom format id in Marquee, name, score) for every custom format of the kind; the profile gets
///   all of them, scored or 0;
/// - `languages`: Radarr's language list for movies, `None` for TV (Sonarr profiles have no language).
pub fn profile(trash: &Value, qualities: &[Value], formats: &[(i64, String, i64)], languages: Option<&[Value]>) -> Value {
    let quality = |name: &str| qualities.iter().map(|d| &d["quality"]).find(|q| q["name"].as_str() == Some(name)).cloned();
    let mut items: Vec<Value> = Vec::new();
    let mut listed: Vec<String> = Vec::new();
    let mut cutoff = Value::Null;
    let cutoff_name = trash["cutoff"].as_str().unwrap_or("");
    let mut group_id = 1000;
    // TRaSH lists the best first; the APIs list the lowest first
    for item in trash["items"].as_array().cloned().unwrap_or_default().iter().rev() {
        let name = item["name"].as_str().unwrap_or("");
        let allowed = item.get("allowed").or_else(|| item.get("hallowed")).and_then(Value::as_bool).unwrap_or(false);
        match item["items"].as_array() {
            Some(subs) if !subs.is_empty() => {
                let inner: Vec<Value> = subs
                    .iter()
                    .filter_map(|s| s.as_str())
                    .filter_map(|s| {
                        listed.push(s.to_string());
                        quality(s).map(|q| json!({ "quality": q, "items": [], "allowed": allowed }))
                    })
                    .collect();
                if inner.is_empty() {
                    continue;
                }
                group_id += 1;
                if name == cutoff_name {
                    cutoff = json!(group_id);
                }
                items.push(json!({ "name": name, "items": inner, "allowed": allowed, "id": group_id }));
            }
            _ => {
                listed.push(name.to_string());
                if let Some(q) = quality(name) {
                    if name == cutoff_name {
                        cutoff = q["id"].clone();
                    }
                    items.push(json!({ "quality": q, "items": [], "allowed": allowed }));
                }
            }
        }
    }
    // qualities TRaSH doesn't list stay in, not allowed, at the bottom
    let missing: Vec<Value> = qualities
        .iter()
        .map(|d| &d["quality"])
        .filter(|q| !listed.iter().any(|n| q["name"].as_str() == Some(n)))
        .map(|q| json!({ "quality": q, "items": [], "allowed": false }))
        .collect();
    items.splice(0..0, missing);
    let format_items: Vec<Value> = formats.iter().map(|(id, name, s)| json!({ "format": id, "name": name, "score": s })).collect();
    let mut out = Map::new();
    out.insert("name".into(), trash["name"].clone());
    out.insert("upgradeAllowed".into(), json!(trash["upgradeAllowed"].as_bool().unwrap_or(true)));
    out.insert("cutoff".into(), cutoff);
    out.insert("minFormatScore".into(), json!(trash["minFormatScore"].as_i64().unwrap_or(0)));
    out.insert("cutoffFormatScore".into(), json!(trash["cutoffFormatScore"].as_i64().unwrap_or(0)));
    out.insert("minUpgradeFormatScore".into(), json!(trash["minUpgradeFormatScore"].as_i64().unwrap_or(1)));
    out.insert("items".into(), json!(items));
    out.insert("formatItems".into(), json!(format_items));
    if let Some(langs) = languages {
        out.insert("language".into(), language(trash["language"].as_str().unwrap_or("Original"), langs));
    }
    Value::Object(out)
}

/// The TRaSH quality size file for a media kind.
pub fn size_file(kind: &str) -> &'static str {
    match kind {
        "movie" => "radarr/quality-size/movie.json",
        "anime" => "sonarr/quality-size/anime.json",
        _ => "sonarr/quality-size/series.json",
    }
}

/// TRaSH's sizes for a kind's qualities: `[{quality, min, preferred, max}]` in TRaSH's order, only the qualities
/// the kind's list has (`names`).
pub fn sizes(file: &Value, names: &[String]) -> Value {
    let rows: Vec<Value> = file["qualities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|q| q["quality"].as_str().is_some_and(|n| names.iter().any(|x| x == n)))
        .map(|q| json!({ "quality": q["quality"], "min": q["min"], "preferred": q["preferred"], "max": q["max"] }))
        .collect();
    json!(rows)
}

/// Which naming TRaSH suggests for the media server: (folder key, file key) in TRaSH's naming files.
fn naming_keys(server: &str, movie: bool) -> (String, String) {
    let id = if movie { "tmdb" } else { "tvdb" };
    match server {
        "plex" | "emby" | "jellyfin" => (format!("{server}-{id}"), format!("{server}-{id}")),
        _ => ("default".into(), if movie { "standard".into() } else { "default".into() }),
    }
}

/// The naming for a media kind (`movie` or `tv`) in the APIs' shape (no id), from TRaSH's naming file, for the
/// media server (`plex`, `jellyfin`, `emby`, or anything else for TRaSH's default).
pub fn naming(kind: &str, file: &Value, server: &str) -> Value {
    if kind == "movie" {
        let (folder, name) = naming_keys(server, true);
        let pick = |section: &str, key: &str, fallback: &str| {
            file[section][key].as_str().or_else(|| file[section][fallback].as_str()).unwrap_or("").to_string()
        };
        json!({
            "renameMovies": true,
            "replaceIllegalCharacters": true,
            "colonReplacementFormat": "smart",
            "standardMovieFormat": pick("file", &name, "standard"),
            "movieFolderFormat": pick("folder", &folder, "default"),
        })
    } else {
        // Sonarr's files name the episode by series id only on Plex and Emby; Jellyfin and the default use the
        // standard episode name (the folder carries the id)
        let (folder, _) = naming_keys(server, false);
        let s = |path: &[&str]| path.iter().fold(file, |v, k| &v[*k]).as_str().unwrap_or("").to_string();
        let series = if file["series"][folder.as_str()].is_string() { s(&["series", &folder]) } else { s(&["series", "default"]) };
        json!({
            "renameEpisodes": true,
            "replaceIllegalCharacters": true,
            "colonReplacementFormat": 4,
            "customColonReplacementFormat": "",
            "multiEpisodeStyle": 5,
            "standardEpisodeFormat": s(&["episodes", "standard", "default"]),
            "dailyEpisodeFormat": s(&["episodes", "daily", "default"]),
            "animeEpisodeFormat": s(&["episodes", "anime", "default"]),
            "seriesFolderFormat": series,
            "seasonFolderFormat": s(&["season", "default"]),
            "specialsFolderFormat": "Specials",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qualities() -> Vec<Value> {
        serde_json::from_str(include_str!("../../data/radarr/qualitydefinition.json")).unwrap()
    }

    #[test]
    fn a_trash_profile_becomes_a_radarr_profile() {
        let trash = json!({
            "name": "HD Bluray + WEB", "upgradeAllowed": true, "cutoff": "Bluray-1080p", "language": "Original",
            "minFormatScore": 0, "cutoffFormatScore": 10000, "minUpgradeFormatScore": 1,
            "items": [
                {"name": "Bluray-1080p", "allowed": true},
                {"name": "WEB 1080p", "allowed": true, "items": ["WEBDL-1080p", "WEBRip-1080p"]},
                {"name": "Bluray-720p", "allowed": true},
                {"name": "Unknown", "allowed": false}
            ]
        });
        let langs: Vec<Value> = serde_json::from_str(include_str!("../../data/radarr/language.json")).unwrap();
        let p = profile(&trash, &qualities(), &[(3, "WEB Tier 01".into(), 1700)], Some(&langs));
        let items = p["items"].as_array().unwrap();
        let last = items.last().unwrap();
        assert_eq!(last["quality"]["name"], "Bluray-1080p", "the best is last");
        assert_eq!(p["cutoff"], last["quality"]["id"]);
        let web = items.iter().find(|i| i["name"] == "WEB 1080p").unwrap();
        assert_eq!(web["items"].as_array().unwrap().len(), 2);
        assert!(web["id"].as_i64().unwrap() > 1000);
        assert_eq!(items.len(), qualities().len() - 1, "every quality is listed once (the two WEB ones in a group)");
        assert!(items.iter().filter(|i| i["quality"]["name"] == "CAM").all(|i| i["allowed"] == false));
        assert_eq!(p["formatItems"][0], json!({"format": 3, "name": "WEB Tier 01", "score": 1700}));
        assert_eq!(p["language"]["name"], "Original");
    }

    #[test]
    fn scores_use_the_profiles_score_set() {
        let cf = json!({"trash_scores": {"default": 1700, "anime-sonarr": 50}});
        assert_eq!(score(&cf, Some("anime-sonarr")), 50);
        assert_eq!(score(&cf, Some("sqp-1-1080p")), 1700);
        assert_eq!(score(&cf, None), 1700);
        assert_eq!(score(&json!({}), None), 0);
    }

    #[test]
    fn naming_follows_the_media_server() {
        let movie = json!({"folder": {"default": "{Movie CleanTitle} ({Release Year})", "jellyfin-tmdb": "J"}, "file": {"standard": "S", "jellyfin-tmdb": "JF"}});
        assert_eq!(naming("movie", &movie, "jellyfin")["movieFolderFormat"], "J");
        assert_eq!(naming("movie", &movie, "none")["standardMovieFormat"], "S");
        let tv = json!({"season": {"default": "Season {season:00}"}, "series": {"default": "D", "jellyfin-tvdb": "JT"},
            "episodes": {"standard": {"default": "E"}, "daily": {"default": "DA"}, "anime": {"default": "AN"}}});
        let n = naming("tv", &tv, "jellyfin");
        assert_eq!((n["seriesFolderFormat"].as_str(), n["animeEpisodeFormat"].as_str()), (Some("JT"), Some("AN")));
        assert_eq!(naming("tv", &tv, "emby")["seriesFolderFormat"], "D", "no Emby entry: the default");
    }
}
