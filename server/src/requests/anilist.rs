//! AniList, for anime requests: AniList lists each season (cour) of an anime as its own entry with its own
//! airing status, which TMDB doesn't. The request page shows them so people see what they are asking for.
//!
//! The address is the `request_anilist_url` setting (AniList's GraphQL API by default; the tests' stand-in
//! otherwise). AniList needs no key.

use kit_server_core::{err, Result};
use serde::Serialize;
use serde_json::{json, Value};

/// AniList's GraphQL API.
pub const DEFAULT_URL: &str = "https://graphql.anilist.co";

/// The query: anime matching a title, with their dates, episodes and status.
const QUERY: &str = "query ($search: String) { Page(perPage: 15) { media(search: $search, type: ANIME, sort: SEARCH_MATCH) {
  id idMal title { romaji english native } format status episodes season seasonYear
  startDate { year month day } coverImage { large } siteUrl } } }";

/// One AniList entry: a season, a film or an OVA of an anime.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Entry {
    /// AniList's id
    pub id: i64,
    /// MyAnimeList's id, when AniList knows it
    pub mal_id: Option<i64>,
    /// the English title, else the romaji one
    pub title: String,
    /// `TV`, `MOVIE`, `OVA`, `ONA`, `SPECIAL`, `TV_SHORT`
    pub format: String,
    /// how it is airing, in words: Finished, Airing, Not yet aired, Cancelled or On break
    pub airing: String,
    /// how many episodes, when known
    pub episodes: Option<i64>,
    /// when it started, `YYYY-MM-DD` (day and month may be `01` when unknown)
    pub start: String,
    /// the cover picture's address
    pub cover: String,
    /// the page on AniList
    pub url: String,
}

/// AniList's status in words.
pub fn airing_words(status: &str) -> &'static str {
    match status {
        "FINISHED" => "Finished",
        "RELEASING" => "Airing",
        "NOT_YET_RELEASED" => "Not yet aired",
        "CANCELLED" => "Cancelled",
        "HIATUS" => "On break",
        _ => "Unknown",
    }
}

/// The entries of one AniList answer, oldest first (so seasons come in order).
pub fn entries(v: &Value) -> Vec<Entry> {
    let mut out: Vec<Entry> = v["data"]["Page"]["media"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| {
                    let t = &m["title"];
                    let title = t["english"].as_str().filter(|s| !s.is_empty()).or_else(|| t["romaji"].as_str()).unwrap_or("").to_string();
                    let d = &m["startDate"];
                    let start = match d["year"].as_i64() {
                        Some(y) => format!("{y:04}-{:02}-{:02}", d["month"].as_i64().unwrap_or(1), d["day"].as_i64().unwrap_or(1)),
                        None => String::new(),
                    };
                    Entry {
                        id: m["id"].as_i64().unwrap_or(0),
                        mal_id: m["idMal"].as_i64(),
                        title,
                        format: m["format"].as_str().unwrap_or("").to_string(),
                        airing: airing_words(m["status"].as_str().unwrap_or("")).to_string(),
                        episodes: m["episodes"].as_i64(),
                        start,
                        cover: m["coverImage"]["large"].as_str().unwrap_or("").to_string(),
                        url: m["siteUrl"].as_str().unwrap_or("").to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // entries without a date go last
    out.sort_by(|a, b| (a.start.is_empty(), &a.start).cmp(&(b.start.is_empty(), &b.start)));
    out
}

/// AniList's entries for the anime called `title`.
pub async fn search(http: &reqwest::Client, base: &str, title: &str) -> Result<Vec<Entry>> {
    let url = if base.trim().is_empty() { DEFAULT_URL } else { base.trim() };
    let r = http
        .post(url)
        .json(&json!({ "query": QUERY, "variables": { "search": title } }))
        .send()
        .await
        .map_err(|e| err(502, format!("AniList can't be reached: {e}")))?;
    match r.status().as_u16() {
        200 => Ok(entries(&r.json().await.map_err(|e| err(502, format!("AniList sent an answer Marquee can't read: {e}")))?)),
        429 => Err(err(503, "AniList is busy (too many requests). Try again in a minute.")),
        s => Err(err(502, format!("AniList answered with an error ({s})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_come_in_order_with_their_status_in_words() {
        let v = json!({ "data": { "Page": { "media": [
            { "id": 2, "title": { "romaji": "Shingeki no Kyojin Season 2", "english": "Attack on Titan Season 2" }, "format": "TV",
              "status": "FINISHED", "episodes": 12, "startDate": { "year": 2017, "month": 4, "day": 1 } },
            { "id": 1, "idMal": 16498, "title": { "romaji": "Shingeki no Kyojin", "english": null }, "format": "TV",
              "status": "FINISHED", "episodes": 25, "startDate": { "year": 2013, "month": 4, "day": 7 } },
            { "id": 3, "title": { "romaji": "Next" }, "format": "TV", "status": "NOT_YET_RELEASED", "startDate": { "year": null } }
        ] } } });
        let e = entries(&v);
        assert_eq!(e.iter().map(|x| x.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(e[0].title, "Shingeki no Kyojin");
        assert_eq!(e[1].title, "Attack on Titan Season 2");
        assert_eq!((e[0].start.as_str(), e[0].mal_id, e[0].episodes), ("2013-04-07", Some(16498), Some(25)));
        assert_eq!(e[2].airing, "Not yet aired");
    }
}
