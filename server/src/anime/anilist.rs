//! AniList (GraphQL): titles, formats, airing status and episodes, and the seasons of a show as separate entries
//! (the chain of sequels), for anime request pages.

use crate::AppState;
use kit_server_core::{err, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// AniList's address (setting `request_anilist_url`, shared with the request pages, can point elsewhere, for tests).
pub const URL: &str = "https://graphql.anilist.co";

/// The fields Marquee asks for, for every entry.
const FIELDS: &str = "id idMal title { romaji english native } format status episodes season seasonYear
    startDate { year month day } coverImage { large } nextAiringEpisode { episode airingAt } description(asHtml: false)
    isAdult genres averageScore studios(isMain: true) { nodes { name } }";

/// One AniList entry: a season, a movie or an OVA.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
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
    /// AniList's genres (`Action`, `Fantasy`, …)
    pub genres: Vec<String>,
    /// the main studios
    pub studios: Vec<String>,
    /// the average score out of 100, when it has one
    pub score: Option<i64>,
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
        genres: v["genres"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect(),
        studios: v["studios"]["nodes"].as_array().into_iter().flatten().filter_map(|n| n["name"].as_str()).map(String::from).collect(),
        score: v["averageScore"].as_i64(),
    }
}

/// A list of anime to browse in the Anime tab's Discover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    /// what people watch most right now
    Trending,
    /// what airs this season
    ThisSeason,
    /// what hasn't aired yet, soonest and most awaited first
    Upcoming,
    /// the best scored of all time
    TopRated,
}

impl List {
    /// The list a word names (`trending`, `season`, `upcoming`, `top`).
    pub fn parse(word: &str) -> Option<List> {
        match word {
            "trending" | "" => Some(List::Trending),
            "season" => Some(List::ThisSeason),
            "upcoming" => Some(List::Upcoming),
            "top" => Some(List::TopRated),
            _ => None,
        }
    }

    /// AniList's sort for the list.
    fn sort(self) -> &'static str {
        match self {
            List::Trending => "TRENDING_DESC",
            List::ThisSeason | List::Upcoming => "POPULARITY_DESC",
            List::TopRated => "SCORE_DESC",
        }
    }
}

/// The anime season (`WINTER`, `SPRING`, `SUMMER`, `FALL`) and year of a day: January to March is winter.
pub fn season_of(month: u32, year: i32) -> (&'static str, i32) {
    let q = match month {
        1..=3 => "WINTER",
        4..=6 => "SPRING",
        7..=9 => "SUMMER",
        _ => "FALL",
    };
    (q, year)
}

/// Whether an entry passes the Discover filters: one of its genres is `genre` and one of its studios is
/// `studio` (case doesn't matter; an empty filter lets everything through).
pub fn passes(m: &Media, genre: &str, studio: &str) -> bool {
    let has = |list: &[String], want: &str| want.trim().is_empty() || list.iter().any(|x| x.eq_ignore_ascii_case(want.trim()));
    has(&m.genres, genre) && has(&m.studios, studio)
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

    /// The entries with these AniList ids, in any order (at most 50 per call; more are asked for in turns).
    pub async fn by_ids(&self, ids: &[i64]) -> Result<Vec<Media>> {
        let q = format!("query ($ids: [Int]) {{ Page(perPage: 50) {{ media(id_in: $ids, type: ANIME) {{ {FIELDS} }} }} }}");
        let mut out = Vec::new();
        for chunk in ids.chunks(50) {
            let d = self.ask(&q, json!({ "ids": chunk })).await?;
            out.extend(d["Page"]["media"].as_array().into_iter().flatten().map(media));
        }
        Ok(out)
    }

    /// One page (from 1) of a Discover list, narrowed to a genre; with a studio, the studio's own anime in the
    /// list's order (AniList can't filter lists by studio), narrowed the same way. Whether there is a next page.
    pub async fn browse(&self, list: List, genre: &str, studio: &str, page: i64, today: (u32, i32)) -> Result<(Vec<Media>, bool)> {
        let page = page.max(1);
        if !studio.trim().is_empty() {
            let q = format!(
                "query ($q: String, $sort: [MediaSort]) {{ Studio(search: $q) {{ name media(sort: $sort, isMain: true, perPage: 50) {{ nodes {{ {FIELDS} }} }} }} }}"
            );
            let d = self.ask(&q, json!({ "q": studio.trim(), "sort": [list.sort()] })).await?;
            let (quarter, year) = season_of(today.0, today.1);
            let all: Vec<Media> = d["Studio"]["media"]["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|n| n["isAdult"] != true)
                .filter(|n| match list {
                    List::ThisSeason => n["season"] == quarter && n["seasonYear"] == year,
                    List::Upcoming => n["status"] == "NOT_YET_RELEASED",
                    _ => true,
                })
                .map(media)
                .filter(|m| passes(m, genre, ""))
                .collect();
            let start = ((page - 1) * 20) as usize;
            let more = all.len() > start + 20;
            return Ok((all.into_iter().skip(start).take(20).collect(), more));
        }
        let mut vars = json!({ "page": page, "sort": [list.sort()] });
        let mut args = vec!["sort: $sort", "type: ANIME", "isAdult: false"];
        let mut decl = vec!["$page: Int", "$sort: [MediaSort]"];
        if !genre.trim().is_empty() {
            vars["genre"] = json!(genre.trim());
            args.push("genre: $genre");
            decl.push("$genre: String");
        }
        match list {
            List::ThisSeason => {
                let (quarter, year) = season_of(today.0, today.1);
                vars["season"] = json!(quarter);
                vars["year"] = json!(year);
                args.extend(["season: $season", "seasonYear: $year"]);
                decl.extend(["$season: MediaSeason", "$year: Int"]);
            }
            List::Upcoming => args.push("status: NOT_YET_RELEASED"),
            List::Trending | List::TopRated => {}
        }
        let q = format!(
            "query ({}) {{ Page(page: $page, perPage: 20) {{ pageInfo {{ hasNextPage }} media({}) {{ {FIELDS} }} }} }}",
            decl.join(", "),
            args.join(", ")
        );
        let d = self.ask(&q, vars).await?;
        let found = d["Page"]["media"].as_array().into_iter().flatten().map(media).collect();
        Ok((found, d["Page"]["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false)))
    }

    /// AniList's genres, for the Discover filter.
    pub async fn genres(&self) -> Result<Vec<String>> {
        let d = self.ask("query { GenreCollection }", json!({})).await?;
        Ok(d["GenreCollection"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|g| *g != "Hentai")
            .map(String::from)
            .collect())
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

    #[test]
    fn genres_studios_and_seasons_filter() {
        let m =
            media(&json!({ "id": 1, "genres": ["Action", "Fantasy"], "studios": {"nodes": [{"name": "MADHOUSE"}]}, "averageScore": 90 }));
        assert_eq!((m.studios.as_slice(), m.score), (&["MADHOUSE".to_string()][..], Some(90)));
        assert!(super::passes(&m, "fantasy", "Madhouse"));
        assert!(super::passes(&m, "", ""));
        assert!(!super::passes(&m, "Romance", ""));
        assert!(!super::passes(&m, "", "Bones"));
        assert_eq!(super::season_of(11, 2026), ("FALL", 2026));
        assert_eq!(super::season_of(1, 2027), ("WINTER", 2027));
        assert_eq!(super::List::parse("top"), Some(super::List::TopRated));
        assert_eq!(super::List::parse("nope"), None);
    }
}
