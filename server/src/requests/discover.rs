//! Finding something to ask for: trending, popular and upcoming movies and TV, search, and one title's details,
//! all from TMDB (`movies::tmdb`), in Seerr's shape (TMDB's names in camelCase) with `mediaInfo` added when the
//! title was asked for or is in the library, so every client shows Requested, Processing or Available.

use super::seerr::media_json;
use super::{media_by_tmdb, upsert_media, TitleFacts};
use crate::AppState;
use kit_server_core::{Result, SqliteDb};
use serde_json::{json, Map, Value};

/// `poster_path` → `posterPath`.
pub fn camel_key(k: &str) -> String {
    let mut out = String::with_capacity(k.len());
    let mut up = false;
    for c in k.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// TMDB's answer with every key in camelCase, as Seerr sends it (`watch/providers` becomes `watchProviders`).
pub fn camel(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut m = Map::new();
            for (k, x) in o {
                let key = if k == "watch/providers" { "watchProviders".to_string() } else { camel_key(k) };
                m.insert(key, camel(x));
            }
            Value::Object(m)
        }
        Value::Array(a) => Value::Array(a.iter().map(camel).collect()),
        x => x.clone(),
    }
}

/// The `mediaInfo` of the title `kind`/`tmdb_id`: Seerr's media object when somebody asked for it or the library
/// has it (a title row is made for a library title the first time it is shown), else `None`.
pub fn media_info(db: &SqliteDb, kind: &str, tmdb_id: i64) -> Result<Option<Value>> {
    if let Some(m) = media_by_tmdb(db, kind, tmdb_id)? {
        return Ok(Some(media_json(db, &m, true)?));
    }
    let facts = if kind == "movie" {
        crate::movies::by_tmdb(db, tmdb_id)?.map(|m| TitleFacts {
            title: m.info.title.clone(),
            year: m.info.year,
            poster: m.info.poster.clone(),
            imdb_id: m.info.imdb_id.clone(),
            anime: super::is_anime(&m.info.genres, &m.info.original_language),
            ..TitleFacts::default()
        })
    } else {
        crate::series::by_tmdb(db, tmdb_id)?.map(|s| TitleFacts {
            anime: s.kind == "anime",
            title: s.info.title.clone(),
            year: s.info.year,
            poster: s.info.poster.clone(),
            imdb_id: s.info.imdb_id.clone(),
            tvdb_id: Some(s.info.tvdb_id).filter(|v| *v > 0),
            seasons: vec![],
        })
    };
    match facts {
        Some(f) => Ok(Some(media_json(db, &upsert_media(db, kind, tmdb_id, &f)?, true)?)),
        None => Ok(None),
    }
}

/// One TMDB result (movie, TV or person) in Seerr's shape; `kind` is its kind when TMDB doesn't say (discover
/// answers).
pub fn result(db: &SqliteDb, v: &Value, kind: Option<&str>) -> Result<Value> {
    let mut r = camel(v);
    let k = v["media_type"].as_str().or(kind).unwrap_or("movie").to_string();
    r["mediaType"] = json!(k);
    if k == "movie" || k == "tv" {
        if let Some(id) = v["id"].as_i64() {
            if let Some(info) = media_info(db, &k, id)? {
                r["mediaInfo"] = info;
            }
        }
    }
    Ok(r)
}

/// A TMDB page (`{page, total_pages, total_results, results}`) in Seerr's shape.
pub fn page(db: &SqliteDb, v: &Value, kind: Option<&str>) -> Result<Value> {
    let mut results = Vec::new();
    for x in v["results"].as_array().into_iter().flatten() {
        results.push(result(db, x, kind)?);
    }
    Ok(json!({
        "page": v["page"].as_i64().unwrap_or(1),
        "totalPages": v["total_pages"].as_i64().unwrap_or(1),
        "totalResults": v["total_results"].as_i64().unwrap_or(results.len() as i64),
        "results": results,
    }))
}

/// The query parameters Seerr passes on to TMDB: the page (1 when missing) and the language, when given.
pub fn base_query(page: Option<i64>, language: Option<&str>) -> Vec<(&'static str, String)> {
    let mut q = vec![("page", page.unwrap_or(1).clamp(1, 500).to_string()), ("include_adult", "false".to_string())];
    if let Some(l) = language.filter(|l| !l.is_empty()) {
        q.push(("language", l.to_string()));
    }
    q
}

/// TMDB's `path` with `query` as a Seerr page of `kind`.
pub async fn tmdb_page(st: &AppState, path: &str, query: &[(&str, String)], kind: Option<&str>) -> Result<Value> {
    let v = crate::movies::tmdb(st).get(path, query).await?;
    page(&st.db, &v, kind)
}

/// Today, `YYYY-MM-DD`, for "upcoming".
pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// One movie's details in Seerr's shape.
pub async fn movie(st: &AppState, tmdb_id: i64, language: Option<&str>) -> Result<Value> {
    let mut q = vec![("append_to_response", "credits,external_ids,videos,release_dates,watch/providers".to_string())];
    q.extend(base_query(None, language).into_iter().filter(|(k, _)| *k == "language"));
    let v = crate::movies::tmdb(st).get(&format!("/movie/{tmdb_id}"), &q).await?;
    let mut d = camel(&v);
    if let Some(o) = d.as_object_mut() {
        o.remove("belongsToCollection");
        o.remove("releaseDates");
        o.remove("videos");
    }
    d["mediaType"] = json!("movie");
    d["collection"] = camel(&v["belongs_to_collection"]);
    d["releases"] = camel(&v["release_dates"]);
    d["relatedVideos"] = videos(&v);
    d["mediaInfo"] = media_info(&st.db, "movie", tmdb_id)?.unwrap_or(Value::Null);
    if d["mediaInfo"].is_null() {
        d.as_object_mut().map(|o| o.remove("mediaInfo"));
    }
    Ok(d)
}

/// One series' details in Seerr's shape.
pub async fn tv(st: &AppState, tmdb_id: i64, language: Option<&str>) -> Result<Value> {
    let mut q = vec![("append_to_response", "credits,external_ids,content_ratings,keywords,videos,watch/providers".to_string())];
    q.extend(base_query(None, language).into_iter().filter(|(k, _)| *k == "language"));
    let v = crate::movies::tmdb(st).get(&format!("/tv/{tmdb_id}"), &q).await?;
    let mut d = camel(&v);
    if let Some(o) = d.as_object_mut() {
        o.remove("videos");
        o.remove("numberOfSeasons");
    }
    d["mediaType"] = json!("tv");
    d["numberOfSeasons"] = v["number_of_seasons"].clone();
    d["keywords"] = camel(&v["keywords"]["results"]);
    d["relatedVideos"] = videos(&v);
    d["mediaInfo"] = media_info(&st.db, "tv", tmdb_id)?.unwrap_or(Value::Null);
    if d["mediaInfo"].is_null() {
        d.as_object_mut().map(|o| o.remove("mediaInfo"));
    }
    Ok(d)
}

/// TMDB's videos as Seerr's `relatedVideos`: `{url, key, name, size, type, site}`.
pub fn videos(v: &Value) -> Value {
    let list: Vec<Value> = v["videos"]["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let key = x["key"].as_str().unwrap_or("");
            let url = if x["site"] == "YouTube" { format!("https://www.youtube.com/watch?v={key}") } else { String::new() };
            json!({ "url": url, "key": key, "name": x["name"], "size": x["size"], "type": x["type"], "site": x["site"] })
        })
        .collect();
    json!(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmdb_names_become_seerrs() {
        assert_eq!(camel_key("poster_path"), "posterPath");
        assert_eq!(camel_key("genre_ids"), "genreIds");
        assert_eq!(camel_key("id"), "id");
        let v = camel(&json!({ "first_air_date": "2022-02-17", "origin_country": ["US"], "seasons": [{ "season_number": 1 }],
            "watch/providers": {} }));
        assert_eq!(
            v,
            json!({ "firstAirDate": "2022-02-17", "originCountry": ["US"], "seasons": [{ "seasonNumber": 1 }],
            "watchProviders": {} })
        );
        assert_eq!(base_query(None, Some("en")), vec![("page", "1".into()), ("include_adult", "false".into()), ("language", "en".into())]);
        let vids = videos(&json!({ "videos": { "results": [{ "site": "YouTube", "key": "abc", "type": "Trailer" }] } }));
        assert_eq!(vids[0]["url"], "https://www.youtube.com/watch?v=abc");
    }
}
