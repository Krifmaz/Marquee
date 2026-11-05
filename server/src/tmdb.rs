//! TMDB, where movie (and later TV) information comes from: search, one movie's details, and finding a movie
//! by its IMDb id. The key is the `tmdb_api_key` setting: a v4 "API Read Access Token" (sent as a bearer
//! token) or a v3 key (sent as `api_key`). The `tmdb_url` setting (`MARQUEE_TMDB_URL`) points the client
//! somewhere else (the tests' stand-in server).

use kit_server_core::{err, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// TMDB's API address.
pub const DEFAULT_URL: &str = "https://api.themoviedb.org/3";

/// Where TMDB's pictures are, at full size.
pub const IMAGE_BASE: &str = "https://image.tmdb.org/t/p/original";

/// A TMDB client for one key.
#[derive(Clone)]
pub struct Tmdb {
    /// the API's address, without a trailing `/`
    base: String,
    /// the v4 token or v3 key
    key: String,
    /// the HTTP client (shared connection pool)
    http: reqwest::Client,
}

/// A movie as Marquee keeps it, from TMDB's details.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MovieInfo {
    /// TMDB's id
    pub tmdb_id: i64,
    /// IMDb's id (`tt0133093`), "" when unknown
    pub imdb_id: String,
    /// the title in English
    pub title: String,
    /// the title in its own language
    pub original_title: String,
    /// the original language, ISO 639-1 (`en`)
    pub original_language: String,
    /// the summary
    pub overview: String,
    /// the year of the first release, 0 when unknown
    pub year: i64,
    /// minutes
    pub runtime: i64,
    /// TMDB's status (`Released`, `In Production`, `Canceled`…)
    pub tmdb_status: String,
    /// the first cinema release, `YYYY-MM-DD`
    pub in_cinemas: Option<String>,
    /// the first disc release, `YYYY-MM-DD`
    pub physical_release: Option<String>,
    /// the first digital release, `YYYY-MM-DD`
    pub digital_release: Option<String>,
    /// genres in English
    pub genres: Vec<String>,
    /// the first production company
    pub studio: String,
    /// the movie's own website
    pub website: String,
    /// the US rating (`R`, `PG-13`), "" when none
    pub certification: String,
    /// a YouTube trailer's id
    pub youtube_trailer: String,
    /// the poster's TMDB path (`/abc.jpg`)
    pub poster: String,
    /// the backdrop's TMDB path
    pub fanart: String,
    /// TMDB's average vote, 0–10
    pub rating: f64,
    /// how many voted
    pub votes: i64,
    /// TMDB's popularity
    pub popularity: f64,
    /// the collection (`The Matrix Collection`), if any: (TMDB id, title)
    pub collection: Option<(i64, String)>,
}

impl Tmdb {
    /// A client for `key` at `base` ("": TMDB itself) on the shared `http` client.
    pub fn new(key: &str, base: &str, http: reqwest::Client) -> Tmdb {
        let base = if base.trim().is_empty() { DEFAULT_URL } else { base.trim() };
        Tmdb { base: base.trim_end_matches('/').to_string(), key: key.trim().to_string(), http }
    }

    /// GET `path` with `query`; the answer's JSON, or a plain error.
    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        if self.key.is_empty() {
            return Err(err(400, "Add a TMDB API key in Settings → Connections first"));
        }
        let mut req = self.http.get(format!("{}{path}", self.base)).query(query);
        // a v4 read token is a JWT (three dot-separated parts); a v3 key is 32 hex digits
        req = if self.key.matches('.').count() == 2 { req.bearer_auth(&self.key) } else { req.query(&[("api_key", &self.key)]) };
        let r = req.send().await.map_err(|e| err(502, format!("TMDB can't be reached: {e}")))?;
        match r.status().as_u16() {
            200 => r.json().await.map_err(|e| err(502, format!("TMDB sent an answer Marquee can't read: {e}"))),
            401 => Err(err(400, "TMDB refused the API key. Check it in Settings → Connections.")),
            404 => Err(err(404, "TMDB doesn't know this title")),
            429 => Err(err(503, "TMDB is busy (too many requests). Try again in a minute.")),
            s => Err(err(502, format!("TMDB answered with an error ({s})"))),
        }
    }

    /// Movies matching `term` (and `year`, when given), best match first; brief information only.
    pub async fn search_movies(&self, term: &str, year: Option<i64>) -> Result<Vec<MovieInfo>> {
        let mut q = vec![("query", term.to_string()), ("include_adult", "false".into())];
        if let Some(y) = year {
            q.push(("year", y.to_string()));
        }
        let v = self.get("/search/movie", &q).await?;
        Ok(v["results"].as_array().map(|a| a.iter().map(brief).collect()).unwrap_or_default())
    }

    /// One movie's full details.
    pub async fn movie(&self, tmdb_id: i64) -> Result<MovieInfo> {
        let q = [("append_to_response", "release_dates,videos,external_ids".to_string())];
        let v = self.get(&format!("/movie/{tmdb_id}"), &q).await?;
        Ok(details(&v))
    }

    /// The TMDB id of the movie with IMDb id `imdb_id`, `None` when TMDB knows none.
    pub async fn find_imdb(&self, imdb_id: &str) -> Result<Option<i64>> {
        let v = self.get(&format!("/find/{imdb_id}"), &[("external_source", "imdb_id".into())]).await?;
        Ok(v["movie_results"].as_array().and_then(|a| a.first()).and_then(|m| m["id"].as_i64()))
    }
}

/// A string field, "" when missing or null.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The year of a `YYYY-MM-DD` date, 0 when there is none.
fn year_of(date: &str) -> i64 {
    date.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
}

/// A search result: what a list shows.
pub fn brief(v: &Value) -> MovieInfo {
    let date = s(v, "release_date");
    MovieInfo {
        tmdb_id: v["id"].as_i64().unwrap_or(0),
        title: s(v, "title"),
        original_title: s(v, "original_title"),
        original_language: s(v, "original_language"),
        overview: s(v, "overview"),
        year: year_of(&date),
        in_cinemas: (!date.is_empty()).then_some(date),
        poster: s(v, "poster_path"),
        fanart: s(v, "backdrop_path"),
        rating: v["vote_average"].as_f64().unwrap_or(0.0),
        votes: v["vote_count"].as_i64().unwrap_or(0),
        popularity: v["popularity"].as_f64().unwrap_or(0.0),
        ..MovieInfo::default()
    }
}

/// A movie's details (`/movie/{id}` with release dates, videos and external ids).
pub fn details(v: &Value) -> MovieInfo {
    let mut m = brief(v);
    m.imdb_id = s(v, "imdb_id");
    if m.imdb_id.is_empty() {
        m.imdb_id = s(&v["external_ids"], "imdb_id");
    }
    m.runtime = v["runtime"].as_i64().unwrap_or(0);
    m.tmdb_status = s(v, "status");
    m.genres = v["genres"].as_array().map(|a| a.iter().map(|g| s(g, "name")).collect()).unwrap_or_default();
    m.studio = v["production_companies"].as_array().and_then(|a| a.first()).map(|c| s(c, "name")).unwrap_or_default();
    m.website = s(v, "homepage");
    m.collection = v["belongs_to_collection"]
        .as_object()
        .map(|c| (c.get("id").and_then(Value::as_i64).unwrap_or(0), c.get("name").and_then(Value::as_str).unwrap_or("").to_string()));
    m.youtube_trailer = v["videos"]["results"]
        .as_array()
        .and_then(|a| a.iter().find(|x| s(x, "site") == "YouTube" && s(x, "type") == "Trailer"))
        .map(|x| s(x, "key"))
        .unwrap_or_default();
    // release dates of every country, by type: 1 premiere, 2 limited, 3 cinemas, 4 digital, 5 disc, 6 TV
    let mut earliest: [Option<String>; 7] = Default::default();
    for country in v["release_dates"]["results"].as_array().into_iter().flatten() {
        let us = s(country, "iso_3166_1") == "US";
        for r in country["release_dates"].as_array().into_iter().flatten() {
            let t = r["type"].as_u64().unwrap_or(0) as usize;
            let date: String = s(r, "release_date").chars().take(10).collect();
            if t < 7 && date.len() == 10 && earliest[t].as_ref().is_none_or(|e| date < *e) {
                earliest[t] = Some(date);
            }
            if us && m.certification.is_empty() {
                m.certification = s(r, "certification");
            }
        }
    }
    let cinemas = [&earliest[2], &earliest[3]].into_iter().flatten().min().cloned();
    if cinemas.is_some() {
        m.in_cinemas = cinemas;
    }
    m.digital_release = earliest[4].clone();
    m.physical_release = earliest[5].clone();
    m
}

#[cfg(test)]
mod tests {
    use super::details;
    use serde_json::json;

    #[test]
    fn details_pick_the_earliest_dates_and_the_us_rating() {
        let v = json!({
            "id": 603, "imdb_id": "tt0133093", "title": "The Matrix", "original_title": "The Matrix", "original_language": "en",
            "release_date": "1999-03-30", "runtime": 136, "status": "Released", "genres": [{"name": "Action"}],
            "production_companies": [{"name": "Village Roadshow Pictures"}], "belongs_to_collection": {"id": 2344, "name": "The Matrix Collection"},
            "videos": {"results": [{"site": "YouTube", "type": "Teaser", "key": "x"}, {"site": "YouTube", "type": "Trailer", "key": "vKQi3bBA1y8"}]},
            "release_dates": {"results": [
                {"iso_3166_1": "GB", "release_dates": [{"type": 3, "release_date": "1999-06-11T00:00:00.000Z", "certification": "15"}]},
                {"iso_3166_1": "US", "release_dates": [
                    {"type": 3, "release_date": "1999-03-31T00:00:00.000Z", "certification": "R"},
                    {"type": 5, "release_date": "1999-09-21T00:00:00.000Z", "certification": ""},
                    {"type": 4, "release_date": "2001-11-20T00:00:00.000Z", "certification": ""}
                ]}
            ]}
        });
        let m = details(&v);
        assert_eq!((m.year, m.runtime, m.certification.as_str()), (1999, 136, "R"));
        assert_eq!(m.in_cinemas.as_deref(), Some("1999-03-31"));
        assert_eq!(m.physical_release.as_deref(), Some("1999-09-21"));
        assert_eq!(m.digital_release.as_deref(), Some("2001-11-20"));
        assert_eq!(m.youtube_trailer, "vKQi3bBA1y8");
        assert_eq!(m.collection, Some((2344, "The Matrix Collection".into())));
    }
}
