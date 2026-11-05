//! The community anime mapping list (Fribb/anime-lists, built from Anime-Lists and others): which AniDB, AniList
//! and MyAnimeList entry is which TheTVDB/TMDB series and season, with the episode offset. Kept in `anime_map`,
//! refreshed once a day.

use kit_server_core::{p, Db, Result, Row, RowExt, SqliteDb};
use serde::Serialize;
use serde_json::Value;

/// Where the list comes from (settings `anime_list_url` can point elsewhere).
pub const LIST_URL: &str = "https://raw.githubusercontent.com/Fribb/anime-lists/master/anime-list-full.json";

/// One entry of the list.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Entry {
    /// AniDB's id
    pub anidb: Option<i64>,
    /// AniList's id
    pub anilist: Option<i64>,
    /// MyAnimeList's id
    pub mal: Option<i64>,
    /// TheTVDB's series id
    pub tvdb: Option<i64>,
    /// TMDB's TV id
    pub tmdb_tv: Option<i64>,
    /// TMDB's movie id
    pub tmdb_movie: Option<i64>,
    /// the TheTVDB season it is (0 for specials and OVAs)
    pub season: Option<i64>,
    /// episodes to add to this entry's numbers to get TheTVDB's
    pub offset: i64,
    /// `TV`, `MOVIE`, `OVA`, `ONA`, `SPECIAL`
    pub kind: String,
}

/// Reads one list item.
pub fn entry(v: &Value) -> Entry {
    let id = |k: &str| v[k].as_i64();
    Entry {
        anidb: id("anidb_id"),
        anilist: id("anilist_id"),
        mal: id("mal_id"),
        tvdb: id("tvdb_id").or(id("thetvdb_id")),
        tmdb_tv: v["themoviedb_id"]["tv"].as_i64(),
        tmdb_movie: v["themoviedb_id"]["movie"].as_i64(),
        season: v["season"]["tvdb"].as_i64(),
        offset: v["episode_offset"].as_i64().unwrap_or(0),
        kind: v["type"].as_str().unwrap_or("").to_string(),
    }
}

/// The mapping table.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS anime_map (
            anidb INTEGER, anilist INTEGER, mal INTEGER, tvdb INTEGER, tmdb_tv INTEGER, tmdb_movie INTEGER,
            season INTEGER, offset_eps INTEGER NOT NULL DEFAULT 0, kind TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS anime_map_anilist ON anime_map(anilist);
        CREATE INDEX IF NOT EXISTS anime_map_tvdb ON anime_map(tvdb);",
    )
    .map_err(|e| anyhow::anyhow!("The anime mapping table can't be made: {}", e.detail))
}

/// Replaces the table with a list (the JSON array); how many entries it holds.
pub fn load(db: &SqliteDb, list: &Value) -> Result<usize> {
    let items = list.as_array().cloned().unwrap_or_default();
    db.execute("DELETE FROM anime_map", p![])?;
    let mut n = 0;
    for v in &items {
        let e = entry(v);
        if e.anilist.is_none() && e.anidb.is_none() {
            continue;
        }
        db.execute(
            "INSERT INTO anime_map (anidb, anilist, mal, tvdb, tmdb_tv, tmdb_movie, season, offset_eps, kind) VALUES (?,?,?,?,?,?,?,?,?)",
            p![e.anidb, e.anilist, e.mal, e.tvdb, e.tmdb_tv, e.tmdb_movie, e.season, e.offset, e.kind],
        )?;
        n += 1;
    }
    Ok(n)
}

/// A table row as an entry.
fn from_row(r: &Row) -> Entry {
    let id = |k: &str| r.get(k).and_then(Value::as_i64);
    Entry {
        anidb: id("anidb"),
        anilist: id("anilist"),
        mal: id("mal"),
        tvdb: id("tvdb"),
        tmdb_tv: id("tmdb_tv"),
        tmdb_movie: id("tmdb_movie"),
        season: id("season"),
        offset: r.i("offset_eps"),
        kind: r.s("kind"),
    }
}

/// The entry of an AniList id.
pub fn by_anilist(db: &SqliteDb, id: i64) -> Result<Option<Entry>> {
    Ok(db.one("SELECT * FROM anime_map WHERE anilist=?", p![id])?.map(|r| from_row(&r)))
}

/// Every entry of a TheTVDB series (its seasons, specials and OVAs), by season.
pub fn by_tvdb(db: &SqliteDb, tvdb: i64) -> Result<Vec<Entry>> {
    Ok(db.query("SELECT * FROM anime_map WHERE tvdb=? ORDER BY season, offset_eps", p![tvdb])?.iter().map(from_row).collect())
}

/// How many entries the table holds.
pub fn count(db: &SqliteDb) -> i64 {
    db.one("SELECT COUNT(*) AS n FROM anime_map", p![]).ok().flatten().map(|r| r.i("n")).unwrap_or(0)
}

/// TheTVDB's season and episode for episode `n` of an entry (AniList and AniDB count from 1 in each entry).
pub fn to_tvdb(e: &Entry, n: i64) -> Option<(i64, i64)> {
    Some((e.season?, n + e.offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entries_are_read_and_numbers_mapped() {
        let e = entry(&json!({"type": "TV", "anidb_id": 17617, "anilist_id": 154587, "mal_id": 52991,
            "themoviedb_id": {"tv": 209867}, "tvdb_id": 424536, "season": {"tvdb": 1, "tmdb": 1}}));
        assert_eq!((e.anilist, e.tvdb, e.tmdb_tv, e.season), (Some(154587), Some(424536), Some(209867), Some(1)));
        let part2 = entry(&json!({"anilist_id": 2, "tvdb_id": 5, "season": {"tvdb": 1}, "episode_offset": 12}));
        assert_eq!(to_tvdb(&part2, 3), Some((1, 15)), "a second cour continues the season's numbers");
    }
}
