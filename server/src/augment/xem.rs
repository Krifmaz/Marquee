//! TheXEM's name list: the other names of series, each with the scene season it means. A port of the
//! `GetSceneTvdbNames` half of Sonarr's `XemProxy` and of `XemService.GetSceneMappings` at the commit in
//! `UPSTREAM.lock`. (XEM's episode numbering is a separate step.)

use kit_server_core::{err, Result};
use serde_json::Value;

use super::scene::SceneMapping;

/// TheXEM's name list, numbered by TheTVDB, with each name's season.
pub const NAMES_URL: &str = "https://thexem.info/map/allNames?seasonNumbers=True&origin=tvdb";

/// TheTVDB's id of Fate/Zero: Sonarr keeps only its first season's names.
const FATE_ZERO: i64 = 79151;

/// Reads TheXEM's `allNames` answer (`{"data": {"<tvdb id>": [{"<name>": <season>}, ...]}}`) as scene mappings:
/// one per name whose season is a whole number. Sonarr's `XemProxy.GetSceneTvdbNames`.
pub fn names_from(answer: &Value) -> Vec<SceneMapping> {
    let mut result = Vec::new();
    let Some(data) = answer["data"].as_object() else { return result };
    for (key, names) in data {
        let Ok(tvdb_id) = key.trim().parse::<i64>() else { continue };
        for name in names.as_array().map(Vec::as_slice).unwrap_or_default() {
            let Some(name) = name.as_object() else { continue };
            for (title, season) in name {
                let season = match season {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let Ok(season) = season.trim().parse::<i32>() else { continue };
                // Sonarr's hack to deal with Fate/Zero
                if tvdb_id == FATE_ZERO && season > 1 {
                    continue;
                }
                result.push(SceneMapping {
                    title: title.clone(),
                    search_term: title.clone(),
                    scene_season_number: Some(season),
                    tvdb_id,
                    ..SceneMapping::default()
                });
            }
        }
    }
    result
}

/// Fetches TheXEM's name list as scene mappings.
pub async fn fetch_names(http: &reqwest::Client) -> Result<Vec<SceneMapping>> {
    let r = http
        .get(NAMES_URL)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| err(502, "TheXEM's name list can't be fetched right now"))?;
    if !r.status().is_success() {
        return Err(err(502, format!("TheXEM's name list answered {}", r.status().as_u16())));
    }
    let v: Value = r.json().await.map_err(|_| err(502, "TheXEM's name list can't be read"))?;
    Ok(names_from(&v))
}
