//! AniList (GraphQL): titles, formats, airing status and episodes, and the seasons of a show as separate entries
//! (the chain of sequels), for anime request pages.

use crate::AppState;
use kit_server_core::{err, Result};
use serde::Serialize;
use serde_json::{json, Value};

/// AniList's address (setting `request_anilist_url`, shared with the request pages, can point elsewhere, for tests).
pub const URL: &str = "https://graphql.anilist.co";

/// The fields Marquee asks for, for every entry.
const FIELDS: &str = "id idMal title { romaji english native } format status episodes season seasonYear
    startDate { year month day } coverImage { large } nextAiringEpisode { episode airingAt } description(asHtml: false)";

/// One AniList entry: a season, a movie or an OVA.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Media {
    /// AniList's id
    pub id: i64,
    /// MyAnimeList's id
    pub mal: Option<i64>,
    /// the English title, else the romaji one
    pub title: String,
    /// the romaji title
    pub romaji: String,
    /// `TV`, `TV_SHORT`, `MOVIE`, `OVA`, `ONA`, `SPECIAL`
    pub format: String,
    /// airing status in words: `Airing`, `Finished`, `Not yet aired`, `Cancelled`, `On hiatus`
    pub status: String,
    /// episodes, when known
    pub episodes: Option<i64>,
    /// `Winter 2024`
    pub season: String,
    /// the year it started
    pub year: Option<i64>,
    /// the poster
    pub cover: String,
    /// the next episode and when it airs (Unix seconds), while airing
    pub next_episode: Option<(i64, i64)>,
    /// the summary
    pub overview: String,
}

/// An AniList entry from its JSON.
pub fn media(v: &Value) -> Media {
    let s = |x: &Value| x.as_str().unwrap_or("").to_string();
    let status = match v["status"].as_str().unwrap_or("") {
        "RELEASING" => "Airing",
        "FINISHED" => "Finished",
        "NOT_YET_RELEASED" => "Not yet aired",
        "CANCELLED" => "Cancelled",
        "HIATUS" => "On hiatus",
        _ => "",
    };
    let english = s(&v["title"]["english"]);
    let romaji = s(&v["title"]["romaji"]);
    let season = match (v["season"].as_str(), v["seasonYear"].as_i64()) {
        (Some(q), Some(y)) => {
            let mut c = q.to_lowercase();
            if let Some(f) = c.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            format!("{c} {y}")
        }
        _ => String::new(),
    };
    Media {
        id: v["id"].as_i64().unwrap_or(0),
        mal: v["idMal"].as_i64(),
        title: if english.is_empty() { romaji.clone() } else { english },
        romaji,
        format: s(&v["format"]),
        status: status.into(),
        episodes: v["episodes"].as_i64(),
        season,
        year: v["startDate"]["year"].as_i64().or(v["seasonYear"].as_i64()),
        cover: s(&v["coverImage"]["large"]),
        next_episode: v["nextAiringEpisode"]["episode"].as_i64().zip(v["nextAiringEpisode"]["airingAt"].as_i64()),
        overview: s(&v["description"]),
    }
}

/// AniList as the settings say to reach it.
pub struct AniList {
    /// the GraphQL address
    url: String,
    /// the shared HTTP client
    http: reqwest::Client,
}

impl AniList {
    /// AniList for this server.
    pub fn from(st: &AppState) -> AniList {
        let url = st.settings.str("request_anilist_url");
        AniList { url: if url.trim().is_empty() { URL.into() } else { url.trim().to_string() }, http: st.http.clone() }
    }

    /// One GraphQL query; its `data`.
    async fn ask(&self, query: &str, vars: Value) -> Result<Value> {
        let r = self
            .http
            .post(&self.url)
            .json(&json!({ "query": query, "variables": vars }))
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| err(502, "AniList can't be reached right now. Try again in a minute."))?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let why = v["errors"][0]["message"].as_str().unwrap_or("no reason given");
            return Err(err(502, format!("AniList answered {}: {why}", status.as_u16())));
        }
        Ok(v["data"].clone())
    }

    /// Anime whose titles match `term`.
    pub async fn search(&self, term: &str) -> Result<Vec<Media>> {
        let q = format!("query ($q: String) {{ Page(perPage: 20) {{ media(search: $q, type: ANIME, isAdult: false) {{ {FIELDS} }} }} }}");
        let d = self.ask(&q, json!({ "q": term })).await?;
        Ok(d["Page"]["media"].as_array().map(|a| a.iter().map(media).collect()).unwrap_or_default())
    }

    /// One entry and the seasons of its show: the entry, its prequels and its sequels (TV formats only), in order.
    pub async fn seasons(&self, id: i64) -> Result<Vec<Media>> {
        let q = format!(
            "query ($id: Int) {{ Media(id: $id, type: ANIME) {{ {FIELDS} relations {{ edges {{ relationType node {{ {FIELDS} relations {{ edges {{ relationType node {{ id format }} }} }} }} }} }} }} }}"
        );
        let d = self.ask(&q, json!({ "id": id })).await?;
        let root = &d["Media"];
        if root.is_null() {
            return Err(err(404, "AniList doesn't know this anime"));
        }
        let mut out = vec![media(root)];
        for edge in root["relations"]["edges"].as_array().cloned().unwrap_or_default() {
            let kind = edge["relationType"].as_str().unwrap_or("");
            let node = &edge["node"];
            if matches!(kind, "PREQUEL" | "SEQUEL") && matches!(node["format"].as_str(), Some("TV") | Some("TV_SHORT") | Some("ONA")) {
                out.push(media(node));
            }
        }
        out.sort_by_key(|m| (m.year.unwrap_or(9999), m.id));
        out.dedup_by_key(|m| m.id);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::media;
    use serde_json::json;

    #[test]
    fn entries_read_in_words() {
        let m = media(
            &json!({ "id": 154587, "idMal": 52991, "title": {"romaji": "Sousou no Frieren", "english": "Frieren: Beyond Journey's End"},
            "format": "TV", "status": "FINISHED", "episodes": 28, "season": "FALL", "seasonYear": 2023, "startDate": {"year": 2023},
            "nextAiringEpisode": null }),
        );
        assert_eq!(
            (m.title.as_str(), m.status.as_str(), m.season.as_str(), m.episodes),
            ("Frieren: Beyond Journey's End", "Finished", "Fall 2023", Some(28))
        );
        assert_eq!(media(&json!({"title": {"romaji": "X"}, "status": "RELEASING"})).title, "X");
    }
}
