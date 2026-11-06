//! Requests (the requests track; docs/TRACKS.md says who owns this folder): people ask for movies and seasons,
//! an admin approves or declines (or the account is auto-approved), an approved request is added to the library
//! and searched for, and it is done once the files are there. The status goes request → approved → downloading →
//! available.
//!
//! The rules live here; the Seerr-compatible API that the web app, LunaSea, nzb360, Homepage and Prowlarr-era
//! tools use is in [`seerr`], discovery (trending, popular, search, details) in [`discover`], the news people get
//! in [`notify`] and AniList's anime data in [`anilist`].
//!
//! Tables:
//!
//! - `request_media`: one row per title anybody asked for (Seerr's "media"), keyed by kind and TMDB id. Whether it
//!   is available is worked out from the library each time, never stored, unless an admin set it by hand.
//! - `requests`: one row per request, with the seasons it asks for, its status, who asked and who decided.
//! - `request_prefs`: each account's default language choice for anime requests.

pub mod anilist;
pub mod blocklist;
pub mod discover;
pub mod issues;
pub mod moving_in;
pub mod notify;
pub mod seerr;
pub mod sliders;

use crate::roles::Role;
use crate::series::Season;
use crate::AppState;
use axum::Router;
use chrono::{DateTime, Duration, Utc};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A request's status, as Seerr numbers them.
pub mod status {
    /// waiting for an admin
    pub const PENDING: i64 = 1;
    /// approved: in the library and being looked for
    pub const APPROVED: i64 = 2;
    /// an admin said no
    pub const DECLINED: i64 = 3;
    /// approving it didn't work (the reason is in `message`); Retry tries again
    pub const FAILED: i64 = 4;
    /// everything it asked for is there
    pub const COMPLETED: i64 = 5;
}

/// A title's availability, as Seerr numbers it.
pub mod media_status {
    /// nobody asked for it and it isn't in the library
    pub const UNKNOWN: i64 = 1;
    /// asked for, waiting for an admin
    pub const PENDING: i64 = 2;
    /// in the library, being looked for or downloading
    pub const PROCESSING: i64 = 3;
    /// some of its seasons are there
    pub const PARTIALLY_AVAILABLE: i64 = 4;
    /// all of it is there
    pub const AVAILABLE: i64 = 5;
    /// kept out of requests (Seerr's blocklist)
    pub const BLOCKLISTED: i64 = 6;
    /// it was there and was removed
    pub const DELETED: i64 = 7;
}

/// The language choices of an anime request: the audio wanted.
pub const ANIME_AUDIO: [&str; 3] = ["dub", "dual", "sub"];

/// The track's tables.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_media (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            tmdb_id INTEGER NOT NULL,
            tvdb_id INTEGER,
            imdb_id TEXT,
            title TEXT NOT NULL DEFAULT '',
            year INTEGER NOT NULL DEFAULT 0,
            poster TEXT NOT NULL DEFAULT '',
            anime INTEGER NOT NULL DEFAULT 0,
            status_override INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(kind, tmdb_id)
        );
        CREATE TABLE IF NOT EXISTS requests (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            media_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            status INTEGER NOT NULL,
            is4k INTEGER NOT NULL DEFAULT 0,
            seasons TEXT NOT NULL DEFAULT '[]',
            requested_by INTEGER NOT NULL,
            modified_by INTEGER,
            profile_id INTEGER,
            root_folder TEXT,
            language_profile_id INTEGER,
            tags TEXT NOT NULL DEFAULT '[]',
            is_auto INTEGER NOT NULL DEFAULT 0,
            ignore_quota INTEGER NOT NULL DEFAULT 0,
            anime_audio TEXT NOT NULL DEFAULT '',
            anime_subtitles TEXT NOT NULL DEFAULT '',
            message TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS requests_media ON requests(media_id);
        CREATE INDEX IF NOT EXISTS requests_user ON requests(requested_by);
        CREATE TABLE IF NOT EXISTS request_prefs (
            user_id INTEGER PRIMARY KEY,
            anime_audio TEXT NOT NULL DEFAULT 'dual',
            anime_subtitles TEXT NOT NULL DEFAULT 'en'
        );",
    )
    .map_err(|e| anyhow::anyhow!("The request tables can't be made: {}", e.detail))?;
    sliders::migrate(db)?;
    blocklist::migrate(db)?;
    issues::migrate(db)
}

/// The track's routes: the Seerr API (also the web app's) at `/api/v1`.
pub fn router() -> Router<AppState> {
    seerr::router()
        .merge(blocklist::router())
        .merge(issues::router())
        .merge(discover::router())
        .merge(sliders::router())
        .merge(moving_in::router())
}

/// Every minute: approved requests whose files are all there become completed, and their people hear about it.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if let Err(e) = complete_available(&st).await {
                tracing::warn!("Checking which requests are done didn't work: {}", e.detail);
            }
        }
    });
}

/// Now as Seerr writes times: `2026-11-05T01:50:50.000Z`.
pub fn stamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// A title somebody asked for (Seerr's "media").
#[derive(Clone, Debug, Default, Serialize)]
pub struct Media {
    /// Marquee's id for it (Seerr's media id)
    pub id: i64,
    /// `movie` or `tv`
    pub kind: String,
    /// TMDB's id
    pub tmdb_id: i64,
    /// TheTVDB's id (TV only)
    pub tvdb_id: Option<i64>,
    /// IMDb's id
    pub imdb_id: Option<String>,
    /// the title
    pub title: String,
    /// the year it came out
    pub year: i64,
    /// the poster's TMDB path
    pub poster: String,
    /// whether it is anime (Japanese animation): requests get the language choice
    pub anime: bool,
    /// an availability an admin set by hand, which wins over the library's
    pub status_override: Option<i64>,
    /// when somebody first asked for it
    pub created_at: String,
    /// when it last changed
    pub updated_at: String,
}

impl Media {
    /// The title of a table row.
    pub fn from_row(r: &Row) -> Media {
        Media {
            id: r.i("id"),
            kind: r.s("kind"),
            tmdb_id: r.i("tmdb_id"),
            tvdb_id: r.get("tvdb_id").and_then(Value::as_i64).filter(|v| *v > 0),
            imdb_id: Some(r.s("imdb_id")).filter(|s| !s.is_empty()),
            title: r.s("title"),
            year: r.i("year"),
            poster: r.s("poster"),
            anime: r.b("anime"),
            status_override: r.get("status_override").and_then(Value::as_i64),
            created_at: r.s("created_at"),
            updated_at: r.s("updated_at"),
        }
    }
}

/// One request.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Request {
    /// Marquee's id
    pub id: i64,
    /// the title asked for
    pub media_id: i64,
    /// `movie` or `tv`
    pub kind: String,
    /// [`status`]
    pub status: i64,
    /// a 4K request (kept for Seerr clients; Marquee has one library)
    pub is4k: bool,
    /// the seasons asked for (TV)
    pub seasons: Vec<i64>,
    /// who asked
    pub requested_by: i64,
    /// who approved or declined it
    pub modified_by: Option<i64>,
    /// the quality profile an admin chose, else the default
    pub profile_id: Option<i64>,
    /// the root folder an admin chose, else the default
    pub root_folder: Option<String>,
    /// Sonarr's language profile (kept for Seerr clients)
    pub language_profile_id: Option<i64>,
    /// tag ids for the library
    pub tags: Vec<i64>,
    /// made automatically (watchlists), not by a person
    pub is_auto: bool,
    /// doesn't count toward the account's limit (an admin said so)
    pub ignore_quota: bool,
    /// anime: `dub`, `dual` or `sub`
    pub anime_audio: String,
    /// anime: the subtitle language, ISO 639-1
    pub anime_subtitles: String,
    /// why approving it didn't work
    pub message: String,
    /// when it was made
    pub created_at: String,
    /// when it last changed
    pub updated_at: String,
}

impl Request {
    /// The request of a table row.
    pub fn from_row(r: &Row) -> Request {
        let list = |k: &str| serde_json::from_str::<Vec<i64>>(&r.s(k)).unwrap_or_default();
        Request {
            id: r.i("id"),
            media_id: r.i("media_id"),
            kind: r.s("kind"),
            status: r.i("status"),
            is4k: r.b("is4k"),
            seasons: list("seasons"),
            requested_by: r.i("requested_by"),
            modified_by: r.get("modified_by").and_then(Value::as_i64),
            profile_id: r.get("profile_id").and_then(Value::as_i64),
            root_folder: Some(r.s("root_folder")).filter(|s| !s.is_empty()),
            language_profile_id: r.get("language_profile_id").and_then(Value::as_i64),
            tags: list("tags"),
            is_auto: r.b("is_auto"),
            ignore_quota: r.b("ignore_quota"),
            anime_audio: r.s("anime_audio"),
            anime_subtitles: r.s("anime_subtitles"),
            message: r.s("message"),
            created_at: r.s("created_at"),
            updated_at: r.s("updated_at"),
        }
    }
}

/// Every request, newest first.
pub fn all(db: &SqliteDb) -> Result<Vec<Request>> {
    Ok(db.query("SELECT * FROM requests ORDER BY id DESC", p![])?.iter().map(Request::from_row).collect())
}

/// One request.
pub fn find(db: &SqliteDb, id: i64) -> Result<Option<Request>> {
    Ok(db.one("SELECT * FROM requests WHERE id=?", p![id])?.map(|r| Request::from_row(&r)))
}

/// The requests for one title.
pub fn for_media(db: &SqliteDb, media_id: i64) -> Result<Vec<Request>> {
    Ok(db.query("SELECT * FROM requests WHERE media_id=? ORDER BY id DESC", p![media_id])?.iter().map(Request::from_row).collect())
}

/// One title.
pub fn media(db: &SqliteDb, id: i64) -> Result<Option<Media>> {
    Ok(db.one("SELECT * FROM request_media WHERE id=?", p![id])?.map(|r| Media::from_row(&r)))
}

/// The title of `kind` with TMDB id `tmdb_id`, if anybody asked for it.
pub fn media_by_tmdb(db: &SqliteDb, kind: &str, tmdb_id: i64) -> Result<Option<Media>> {
    Ok(db.one("SELECT * FROM request_media WHERE kind=? AND tmdb_id=?", p![kind, tmdb_id])?.map(|r| Media::from_row(&r)))
}

/// Every title, newest first.
pub fn all_media(db: &SqliteDb) -> Result<Vec<Media>> {
    Ok(db.query("SELECT * FROM request_media ORDER BY id DESC", p![])?.iter().map(Media::from_row).collect())
}

/// Saves a request's changeable fields.
pub fn save(db: &SqliteDb, r: &Request) -> Result<()> {
    db.execute(
        "UPDATE requests SET status=?, seasons=?, modified_by=?, profile_id=?, root_folder=?, language_profile_id=?, tags=?,
            anime_audio=?, anime_subtitles=?, message=?, requested_by=?, updated_at=? WHERE id=?",
        p![
            r.status,
            serde_json::to_string(&r.seasons).unwrap_or_else(|_| "[]".into()),
            r.modified_by,
            r.profile_id,
            r.root_folder,
            r.language_profile_id,
            serde_json::to_string(&r.tags).unwrap_or_else(|_| "[]".into()),
            r.anime_audio,
            r.anime_subtitles,
            r.message,
            r.requested_by,
            r.updated_at,
            r.id
        ],
    )?;
    Ok(())
}

/// Forgets a request.
pub fn remove(db: &SqliteDb, id: i64) -> Result<()> {
    db.execute("DELETE FROM requests WHERE id=?", p![id])?;
    Ok(())
}

/// Forgets a title and every request for it (the library keeps the movie or series).
pub fn remove_media(db: &SqliteDb, id: i64) -> Result<()> {
    db.execute("DELETE FROM requests WHERE media_id=?", p![id])?;
    db.execute("DELETE FROM request_media WHERE id=?", p![id])?;
    Ok(())
}

/// A title's availability and, for TV, each regular season's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Availability {
    /// [`media_status`] of the whole title
    pub status: i64,
    /// (season, [`media_status`]) for TV, specials left out
    pub seasons: Vec<(i64, i64)>,
    /// Marquee's movie or series id, when it is in the library
    pub library_id: Option<i64>,
}

/// Where `m` stands now: from the library (files there or not), its requests, or an admin's choice.
pub fn availability(db: &SqliteDb, m: &Media) -> Result<Availability> {
    let reqs = for_media(db, m.id)?;
    let pending = |season: Option<i64>| {
        reqs.iter().any(|r| r.status == status::PENDING && season.is_none_or(|s| r.seasons.contains(&s) || r.seasons.is_empty()))
    };
    let mut a = Availability::default();
    if m.kind == "movie" {
        let movie = crate::movies::by_tmdb(db, m.tmdb_id)?;
        a.library_id = movie.as_ref().map(|x| x.id);
        a.status = match movie {
            Some(x) if x.has_file() => media_status::AVAILABLE,
            Some(_) => media_status::PROCESSING,
            None if pending(None) => media_status::PENDING,
            None => media_status::UNKNOWN,
        };
    } else {
        let series = crate::series::by_tmdb(db, m.tmdb_id)?;
        a.library_id = series.as_ref().map(|s| s.id);
        let now = Utc::now();
        match series {
            Some(s) => {
                let eps = crate::series::episodes(db, s.id)?;
                for season in s.seasons.iter().filter(|x| x.season > 0) {
                    let aired: Vec<_> = eps.iter().filter(|e| e.season == season.season && e.aired(now)).collect();
                    let have = aired.iter().filter(|e| e.has_file()).count();
                    let st = if !aired.is_empty() && have == aired.len() {
                        media_status::AVAILABLE
                    } else if have > 0 {
                        media_status::PARTIALLY_AVAILABLE
                    } else if season.monitored && s.monitored {
                        media_status::PROCESSING
                    } else if pending(Some(season.season)) {
                        media_status::PENDING
                    } else {
                        media_status::UNKNOWN
                    };
                    a.seasons.push((season.season, st));
                }
                a.status = overall(&a.seasons);
            }
            None => {
                let mut asked: Vec<i64> = reqs.iter().filter(|r| r.status == status::PENDING).flat_map(|r| r.seasons.clone()).collect();
                asked.sort_unstable();
                asked.dedup();
                a.seasons = asked.iter().map(|s| (*s, media_status::PENDING)).collect();
                a.status = if pending(None) { media_status::PENDING } else { media_status::UNKNOWN };
            }
        }
    }
    if let Some(o) = m.status_override {
        a.status = o;
    }
    Ok(a)
}

/// A series' status from its seasons': all there is available, some is partly, any being looked for is processing.
pub fn overall(seasons: &[(i64, i64)]) -> i64 {
    let has = |st: i64| seasons.iter().any(|(_, s)| *s == st);
    // seasons with nothing aired yet don't count against "all there"
    let counted: Vec<i64> = seasons.iter().map(|(_, s)| *s).filter(|s| *s != media_status::UNKNOWN).collect();
    if !counted.is_empty() && counted.iter().all(|s| *s == media_status::AVAILABLE) {
        media_status::AVAILABLE
    } else if has(media_status::AVAILABLE) || has(media_status::PARTIALLY_AVAILABLE) {
        media_status::PARTIALLY_AVAILABLE
    } else if has(media_status::PROCESSING) {
        media_status::PROCESSING
    } else if has(media_status::PENDING) {
        media_status::PENDING
    } else {
        media_status::UNKNOWN
    }
}

/// Whether everything request `r` asked for is there.
pub fn fulfilled(r: &Request, a: &Availability) -> bool {
    if r.kind == "movie" || r.seasons.is_empty() {
        return a.status == media_status::AVAILABLE;
    }
    r.seasons.iter().all(|n| a.seasons.iter().any(|(s, st)| s == n && *st == media_status::AVAILABLE))
}

/// The words for where a request stands, for people: Waiting for approval, Approved, Downloading, Available,
/// Declined or Didn't work.
pub fn words(db: &SqliteDb, r: &Request, a: &Availability) -> &'static str {
    match r.status {
        status::PENDING => "Waiting for approval",
        status::DECLINED => "Declined",
        status::FAILED => "Didn't work",
        status::COMPLETED => "Available",
        _ if fulfilled(r, a) => "Available",
        _ if a.library_id.is_some_and(|id| downloading(db, &r.kind, id)) => "Downloading",
        _ => "Approved",
    }
}

/// Whether the movie or series `id` has a download on its way.
pub fn downloading(db: &SqliteDb, kind: &str, id: i64) -> bool {
    crate::downloads::active(db).unwrap_or_default().iter().any(|d| {
        if kind == "movie" {
            d.movie_id == Some(id)
        } else {
            d.series_id == Some(id)
        }
    })
}

/// What an account may ask for in a period, as Seerr's `User.getQuota` works it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Quota {
    /// how many in the period, 0 for no limit
    pub limit: i64,
    /// the period in days, 0 for all time
    pub days: i64,
    /// how many were asked for in the period (0 without a limit, like Seerr)
    pub used: i64,
}

impl Quota {
    /// How many are left, `None` without a limit.
    pub fn remaining(&self) -> Option<i64> {
        (self.limit > 0).then(|| (self.limit - self.used).max(0))
    }

    /// Whether nothing is left (Seerr's `restricted`).
    pub fn restricted(&self) -> bool {
        self.limit > 0 && self.limit - self.used <= 0
    }
}

/// The account `user`'s movie and TV quotas, ported from Seerr's `User.getQuota`: accounts that may manage
/// accounts (admins) have no limit; every request not declined and not marked "skip the limit" made in the last
/// `days` days counts (all time when `days` is 0), TV by its seasons.
pub fn quotas(db: &SqliteDb, user: &Row) -> Result<(Quota, Quota)> {
    let bypass = seerr::has_any(user, &[seerr::perm::MANAGE_USERS]);
    let mine: Vec<Request> =
        db.query("SELECT * FROM requests WHERE requested_by=?", p![user.i("id")])?.iter().map(Request::from_row).collect();
    let count = |kind: &str, limit: i64, days: i64| -> i64 {
        if limit <= 0 {
            return 0;
        }
        let from = (days > 0).then(|| (Utc::now() - Duration::days(days)).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
        mine.iter()
            .filter(|r| r.kind == kind && r.status != status::DECLINED && !r.ignore_quota)
            .filter(|r| from.as_ref().is_none_or(|f| r.created_at > *f))
            .map(|r| if kind == "tv" { r.seasons.len() as i64 } else { 1 })
            .sum()
    };
    let limit = |k: &str| if bypass { 0 } else { user.i(k).max(0) };
    let (ml, tl) = (limit("movie_quota_limit"), limit("tv_quota_limit"));
    let (md, td) = (user.i("movie_quota_days").max(0), user.i("tv_quota_days").max(0));
    Ok((Quota { limit: ml, days: md, used: count("movie", ml, md) }, Quota { limit: tl, days: td, used: count("tv", tl, td) }))
}

/// An account's row (the `users` table), 404 when there is none.
pub fn user(db: &SqliteDb, id: i64) -> Result<Row> {
    db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| err(404, "There is no such account"))
}

/// An account's default language choice for anime: (audio, subtitles).
pub fn anime_prefs(db: &SqliteDb, user_id: i64) -> Result<(String, String)> {
    Ok(db
        .one("SELECT * FROM request_prefs WHERE user_id=?", p![user_id])?
        .map(|r| (r.s("anime_audio"), r.s("anime_subtitles")))
        .unwrap_or_else(|| ("dual".into(), "en".into())))
}

/// Sets an account's default language choice for anime; 400 for an unknown audio choice or language.
pub fn set_anime_prefs(db: &SqliteDb, user_id: i64, audio: &str, subtitles: &str) -> Result<()> {
    check_anime_choice(audio, subtitles)?;
    db.execute(
        "INSERT INTO request_prefs (user_id, anime_audio, anime_subtitles) VALUES (?,?,?)
            ON CONFLICT(user_id) DO UPDATE SET anime_audio=excluded.anime_audio, anime_subtitles=excluded.anime_subtitles",
        p![user_id, audio, subtitles.to_lowercase()],
    )?;
    Ok(())
}

/// 400 unless `audio` is dub, dual or sub and `subtitles` a two-letter language code.
pub fn check_anime_choice(audio: &str, subtitles: &str) -> Result<()> {
    if !ANIME_AUDIO.contains(&audio) {
        return Err(err(400, "Pick the audio: Dub only, Dual audio or Sub only"));
    }
    if subtitles.len() != 2 || !subtitles.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(err(400, "The subtitle language is a two-letter code, like en"));
    }
    Ok(())
}

/// What somebody asks for.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Ask {
    /// `movie` or `tv`
    pub kind: String,
    /// TMDB's id
    pub tmdb_id: i64,
    /// TheTVDB's id, when the caller knows it
    pub tvdb_id: Option<i64>,
    /// the seasons (TV); empty means every regular season
    pub seasons: Vec<i64>,
    /// a 4K request
    pub is4k: bool,
    /// a quality profile (admins)
    pub profile_id: Option<i64>,
    /// a root folder (admins)
    pub root_folder: Option<String>,
    /// Sonarr's language profile (kept)
    pub language_profile_id: Option<i64>,
    /// tags (admins)
    pub tags: Vec<i64>,
    /// ask in another account's name (admins)
    pub user_id: Option<i64>,
    /// don't count toward the account's limit (admins)
    pub ignore_quota: bool,
    /// anime: `dub`, `dual` or `sub`; the account's default when not given
    pub anime_audio: Option<String>,
    /// anime: the subtitle language; the account's default when not given
    pub anime_subtitles: Option<String>,
}

/// The facts about a title that a request keeps, from TMDB or the library.
#[derive(Clone, Debug, Default)]
pub struct TitleFacts {
    /// the title
    pub title: String,
    /// the year
    pub year: i64,
    /// the poster's TMDB path
    pub poster: String,
    /// TheTVDB's id (TV)
    pub tvdb_id: Option<i64>,
    /// IMDb's id
    pub imdb_id: String,
    /// whether it is anime
    pub anime: bool,
    /// the regular seasons (TV)
    pub seasons: Vec<i64>,
}

/// Whether a title is anime: Japanese and animated.
pub fn is_anime(genres: &[String], language: &str) -> bool {
    language == "ja" && genres.iter().any(|g| g == "Animation")
}

/// The facts about the title `kind`/`tmdb_id`: the library's when it has the title, else TMDB's.
pub async fn facts(st: &AppState, kind: &str, tmdb_id: i64) -> Result<TitleFacts> {
    if kind == "movie" {
        let info = match crate::movies::by_tmdb(&st.db, tmdb_id)? {
            Some(m) => m.info,
            None => crate::movies::tmdb(st).movie(tmdb_id).await?,
        };
        return Ok(TitleFacts {
            title: info.title,
            year: info.year,
            poster: info.poster,
            tvdb_id: None,
            imdb_id: info.imdb_id,
            anime: is_anime(&info.genres, &info.original_language),
            seasons: vec![],
        });
    }
    let (info, anime_kind) = match crate::series::by_tmdb(&st.db, tmdb_id)? {
        Some(s) => (s.info, s.kind == "anime"),
        None => (crate::movies::tmdb(st).series(tmdb_id).await?, false),
    };
    Ok(TitleFacts {
        anime: anime_kind || is_anime(&info.genres, &info.original_language),
        title: info.title,
        year: info.year,
        poster: info.poster,
        tvdb_id: Some(info.tvdb_id).filter(|v| *v > 0),
        imdb_id: info.imdb_id,
        seasons: info.seasons.into_iter().filter(|s| *s > 0).collect(),
    })
}

/// The title row for `kind`/`tmdb_id`, made from `f` when nobody asked for it before (its facts refreshed when
/// somebody did).
pub fn upsert_media(db: &SqliteDb, kind: &str, tmdb_id: i64, f: &TitleFacts) -> Result<Media> {
    let now = stamp();
    match media_by_tmdb(db, kind, tmdb_id)? {
        Some(m) => {
            db.execute(
                "UPDATE request_media SET title=?, year=?, poster=?, tvdb_id=COALESCE(?, tvdb_id), imdb_id=?, anime=? WHERE id=?",
                p![f.title, f.year, f.poster, f.tvdb_id, f.imdb_id, f.anime as i64, m.id],
            )?;
            media(db, m.id)?.ok_or_else(|| err(500, "The title can't be read back"))
        }
        None => {
            let id = db.execute(
                "INSERT INTO request_media (kind, tmdb_id, tvdb_id, imdb_id, title, year, poster, anime, created_at, updated_at)
                    VALUES (?,?,?,?,?,?,?,?,?,?)",
                p![kind, tmdb_id, f.tvdb_id, f.imdb_id, f.title, f.year, f.poster, f.anime as i64, now, now],
            )?;
            media(db, id)?.ok_or_else(|| err(500, "The title can't be read back"))
        }
    }
}

/// The account that asks, as the request rules need it.
#[derive(Clone, Debug)]
pub struct Asker {
    /// the account's row
    pub row: Row,
    /// its role
    pub role: Role,
}

impl Asker {
    /// The account's id.
    pub fn id(&self) -> i64 {
        self.row.i("id")
    }

    /// Whether it may approve, decline and change any request.
    pub fn admin(&self) -> bool {
        self.role.is_admin()
    }

    /// Seerr's `hasPermission(bits, {type: 'or'})`: any one of `bits` (admins have them all).
    pub fn has(&self, bits: &[i64]) -> bool {
        seerr::has_any(&self.row, bits)
    }

    /// Seerr's `hasPermission(bits)`: every one of `bits` (admins have them all).
    pub fn has_all(&self, bits: &[i64]) -> bool {
        seerr::has_all(&self.row, bits)
    }
}

/// Makes a request, ported from Seerr's `MediaRequest.request`: the account it is for (another one only for
/// accounts that may manage requests and accounts), its permission to ask, its limits, what is already asked for
/// or there, then the new request, approved at once when `by` may auto-approve or manage requests. Errors carry
/// Seerr's status and words: 403 permission or limit, 409 already asked for, 202 no seasons left to ask for.
pub async fn ask(st: &AppState, by: &Asker, a: Ask) -> Result<Request> {
    use seerr::perm::*;
    if a.kind != "movie" && a.kind != "tv" {
        return Err(err(400, "Say what you're asking for: a movie or TV"));
    }
    if a.tmdb_id <= 0 {
        return Err(err(400, "Say which title you're asking for"));
    }
    let who = match a.user_id {
        Some(_) if !by.has_all(&[MANAGE_USERS, MANAGE_REQUESTS]) => {
            return Err(err(403, "You do not have permission to modify the request user."));
        }
        Some(u) if u != by.id() => {
            let row = user(&st.db, u)?;
            Asker { role: Role::parse(&row.s("role")), row }
        }
        _ => by.clone(),
    };
    let movie = a.kind == "movie";
    let may_ask = match (movie, a.is4k) {
        (true, false) => [REQUEST, REQUEST_MOVIE],
        (true, true) => [REQUEST_4K, REQUEST_4K_MOVIE],
        (false, false) => [REQUEST, REQUEST_TV],
        (false, true) => [REQUEST_4K, REQUEST_4K_TV],
    };
    if !who.has(&may_ask) {
        let what = if movie { "movie" } else { "series" };
        return Err(err(403, format!("You do not have permission to make {}{what} requests.", if a.is4k { "4K " } else { "" })));
    }
    let (mq, tq) = quotas(&st.db, &who.row)?;
    let can_bypass = by.has(&[MANAGE_REQUESTS]);
    let ignore_quota = a.ignore_quota && can_bypass && (if movie { mq.limit } else { tq.limit }) > 0;
    if !ignore_quota {
        if a.ignore_quota && !can_bypass {
            return Err(err(403, "You do not have permission to bypass user quota limits."));
        } else if movie && mq.restricted() {
            return Err(err(403, "Movie Quota exceeded."));
        } else if !movie && tq.restricted() {
            return Err(err(403, "Series Quota exceeded."));
        }
    }
    if media_by_tmdb(&st.db, &a.kind, a.tmdb_id)?.and_then(|m| m.status_override) == Some(media_status::BLOCKLISTED) {
        return Err(err(403, "This media is blocklisted."));
    }
    let f = facts(st, &a.kind, a.tmdb_id).await?;
    let m = upsert_media(&st.db, &a.kind, a.tmdb_id, &f)?;
    if m.status_override == Some(media_status::DELETED) {
        // like Seerr, a deleted title asked for again starts over
        st.db.execute("UPDATE request_media SET status_override=NULL WHERE id=?", p![m.id])?;
    }
    if let Some(t) = a.tvdb_id.filter(|t| *t > 0 && m.tvdb_id.is_none()) {
        st.db.execute("UPDATE request_media SET tvdb_id=? WHERE id=?", p![t, m.id])?;
    }
    // oldest first, like Seerr's query without an order
    let mut existing: Vec<Request> = for_media(&st.db, m.id)?.into_iter().filter(|r| r.is4k == a.is4k).collect();
    existing.reverse();
    if movie && existing.first().is_some_and(|r| !matches!(r.status, status::DECLINED | status::COMPLETED)) {
        return Err(err(409, "Request for this media already exists."));
    }
    if existing.iter().any(|r| r.requested_by == who.id() && r.is_auto) {
        return Err(err(409, "Auto-request for this media and user already exists."));
    }
    let mut seasons = Vec::new();
    if !movie {
        // "all" (or nothing) is every regular season; specials only when asked for by number
        let wanted: Vec<i64> = if a.seasons.is_empty() { f.seasons.clone() } else { a.seasons.clone() };
        let avail = availability(&st.db, &m)?;
        let mut taken: Vec<i64> =
            existing.iter().filter(|r| !matches!(r.status, status::DECLINED | status::COMPLETED)).flat_map(|r| r.seasons.clone()).collect();
        taken.extend(avail.seasons.iter().filter(|(_, s)| *s != media_status::UNKNOWN).map(|(n, _)| *n));
        seasons = wanted.into_iter().filter(|n| *n > 0 && !taken.contains(n)).collect();
        seasons.dedup();
        if seasons.is_empty() {
            return Err(err(202, "No seasons available to request"));
        }
        if !ignore_quota && tq.limit > 0 && seasons.len() as i64 > tq.remaining().unwrap_or(0) {
            return Err(err(403, "Series Quota exceeded."));
        }
    }
    let (audio, subs) = if f.anime {
        let (da, ds) = anime_prefs(&st.db, who.id())?;
        let (audio, subs) = (a.anime_audio.clone().unwrap_or(da), a.anime_subtitles.clone().unwrap_or(ds).to_lowercase());
        check_anime_choice(&audio, &subs)?;
        (audio, subs)
    } else {
        (String::new(), String::new())
    };
    let auto_bits = match (movie, a.is4k) {
        (true, false) => [AUTO_APPROVE, AUTO_APPROVE_MOVIE, MANAGE_REQUESTS],
        (true, true) => [AUTO_APPROVE_4K, AUTO_APPROVE_4K_MOVIE, MANAGE_REQUESTS],
        (false, false) => [AUTO_APPROVE, AUTO_APPROVE_TV, MANAGE_REQUESTS],
        (false, true) => [AUTO_APPROVE_4K, AUTO_APPROVE_4K_TV, MANAGE_REQUESTS],
    };
    let auto = by.has(&auto_bits);
    let now = stamp();
    let id = st.db.execute(
        "INSERT INTO requests (media_id, kind, status, is4k, seasons, requested_by, profile_id, root_folder, language_profile_id,
            tags, ignore_quota, anime_audio, anime_subtitles, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        p![
            m.id,
            a.kind,
            status::PENDING,
            a.is4k as i64,
            serde_json::to_string(&seasons).unwrap_or_else(|_| "[]".into()),
            who.id(),
            a.profile_id,
            a.root_folder.filter(|r| !r.trim().is_empty()),
            // Sonarr v4 has one language profile; Seerr clients expect its id on TV requests
            if movie { a.language_profile_id } else { a.language_profile_id.or(Some(1)) },
            serde_json::to_string(&a.tags).unwrap_or_else(|_| "[]".into()),
            ignore_quota as i64,
            audio,
            subs,
            now,
            now
        ],
    )?;
    st.db.execute("UPDATE request_media SET updated_at=? WHERE id=?", p![now, m.id])?;
    let r = find(&st.db, id)?.ok_or_else(|| err(500, "The new request can't be read back"))?;
    tracing::info!("{} asked for {} ({})", who.row.s("username"), f.title, a.kind);
    if auto {
        return decide(st, r, by, true).await;
    }
    notify::spawn(st, notify::Event::Pending, &r, &m, &who.row.s("username"));
    Ok(r)
}

/// Approves (`yes`) or declines a request as `by`. Approving adds the title to the library (or turns on the
/// asked-for seasons) and starts a search; when that can't be done the request is marked as failed with the
/// reason, and Retry tries again.
pub async fn decide(st: &AppState, mut r: Request, by: &Asker, yes: bool) -> Result<Request> {
    let m = media(&st.db, r.media_id)?.ok_or_else(|| err(404, "The title of this request is gone"))?;
    r.modified_by = Some(by.id());
    r.updated_at = stamp();
    let asker = user(&st.db, r.requested_by).map(|u| u.s("username")).unwrap_or_default();
    if !yes {
        r.status = status::DECLINED;
        save(&st.db, &r)?;
        notify::spawn(st, notify::Event::Declined, &r, &m, &asker);
        return Ok(r);
    }
    match to_library(st, &r, &m).await {
        Ok((profile, root)) => {
            r.status = status::APPROVED;
            r.profile_id = Some(profile);
            r.root_folder = Some(root);
            if r.kind == "tv" && r.language_profile_id.is_none() {
                // Sonarr v4 has one language profile; Seerr clients expect its id
                r.language_profile_id = Some(1);
            }
            r.message.clear();
            save(&st.db, &r)?;
            notify::spawn(st, notify::Event::Approved, &r, &m, &asker);
        }
        Err(e) => {
            r.status = status::FAILED;
            r.message = e.detail.clone();
            save(&st.db, &r)?;
            tracing::warn!("Approving the request for {} didn't work: {}", m.title, e.detail);
            notify::spawn(st, notify::Event::Failed, &r, &m, &asker);
        }
    }
    Ok(r)
}

/// Puts what `r` asks for into the library and starts looking for it; the quality profile and root folder the
/// movie or series has.
pub async fn to_library(st: &AppState, r: &Request, m: &Media) -> Result<(i64, String)> {
    if r.kind == "movie" {
        let movie = match crate::movies::by_tmdb(&st.db, m.tmdb_id)? {
            Some(mut movie) => {
                if !movie.monitored {
                    movie.monitored = true;
                    crate::movies::save(&st.db, &movie)?;
                }
                movie
            }
            None => {
                let info = crate::movies::tmdb(st).movie(m.tmdb_id).await?;
                let o = crate::movies::AddOptions {
                    quality_profile_id: match r.profile_id {
                        Some(p) => p,
                        None => crate::quality::default_profile(&st.db, "movie")?,
                    },
                    // anime films go in the anime folder (the Anime tab's library), else with the movies
                    root_folder: match &r.root_folder {
                        Some(f) => f.clone(),
                        None if m.anime => crate::series::default_root(&st.db, "anime").or_else(|_| crate::movies::default_root(&st.db))?,
                        None => crate::movies::default_root(&st.db)?,
                    },
                    path: None,
                    monitored: true,
                    minimum_availability: "released".into(),
                    tags: r.tags.clone(),
                };
                crate::movies::add(&st.db, &info, &o)?
            }
        };
        crate::downloads::spawn_search(st, crate::downloads::Search::Movies(vec![movie.id]));
        return Ok((movie.quality_profile_id, movie.root_folder));
    }
    let s = match crate::series::by_tmdb(&st.db, m.tmdb_id)? {
        Some(mut s) => {
            s.monitored = true;
            let wanted: Vec<Season> = r.seasons.iter().map(|n| Season { season: *n, monitored: true }).collect();
            crate::series::set_seasons(&st.db, &mut s, &wanted)?;
            s
        }
        None => {
            let t = crate::movies::tmdb(st);
            let info = t.series(m.tmdb_id).await?;
            let eps = crate::series::fetch_episodes(&t, &info).await?;
            let kind = if m.anime { "anime" } else { "tv" };
            let seasons = info.seasons.iter().map(|n| Season { season: *n, monitored: r.seasons.contains(n) }).collect();
            let o = crate::series::AddOptions {
                quality_profile_id: match r.profile_id {
                    Some(p) => p,
                    None => crate::quality::default_profile(&st.db, "tv")?,
                },
                root_folder: match &r.root_folder {
                    Some(f) => f.clone(),
                    None => crate::series::default_root(&st.db, kind).or_else(|e| {
                        if kind == "anime" {
                            crate::series::default_root(&st.db, "tv")
                        } else {
                            Err(e)
                        }
                    })?,
                },
                path: None,
                monitored: true,
                monitor: "all".into(),
                series_type: if m.anime { "anime".into() } else { "standard".into() },
                season_folder: true,
                tags: r.tags.clone(),
                seasons: Some(seasons),
            };
            crate::series::add(&st.db, &info, eps, &o)?
        }
    };
    crate::downloads::spawn_search(st, crate::downloads::Search::Series(s.id, None));
    Ok((s.quality_profile_id, s.root_folder))
}

/// Marks approved requests whose files are all there as completed and tells their people.
pub async fn complete_available(st: &AppState) -> Result<()> {
    for mut r in all(&st.db)?.into_iter().filter(|r| r.status == status::APPROVED) {
        let Some(m) = media(&st.db, r.media_id)? else { continue };
        if fulfilled(&r, &availability(&st.db, &m)?) {
            r.status = status::COMPLETED;
            r.updated_at = stamp();
            save(&st.db, &r)?;
            let asker = user(&st.db, r.requested_by).map(|u| u.s("username")).unwrap_or_default();
            notify::spawn(st, notify::Event::Available, &r, &m, &asker);
        }
    }
    Ok(())
}

/// Seconds since `t` (RFC 3339), for sorting; 0 when it can't be read.
pub fn when(t: &str) -> i64 {
    DateTime::parse_from_rfc3339(t).map(|d| d.timestamp_millis()).unwrap_or(0)
}

/// A request for the web app's own lists: the Seerr shape plus the words and the language choice.
pub fn extra(db: &SqliteDb, r: &Request, m: &Media) -> Result<Value> {
    let a = availability(db, m)?;
    Ok(json!({
        "statusWords": words(db, r, &a),
        "title": m.title,
        "year": m.year,
        "posterUrl": crate::movies::poster_url(&m.poster),
        "anime": m.anime,
        "animeAudio": r.anime_audio,
        "animeSubtitles": r.anime_subtitles,
        "message": r.message,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_series_is_available_when_every_aired_season_is() {
        use media_status::*;
        assert_eq!(overall(&[(1, AVAILABLE), (2, AVAILABLE)]), AVAILABLE);
        assert_eq!(overall(&[(1, AVAILABLE), (2, UNKNOWN)]), AVAILABLE, "a season nobody wants doesn't count");
        assert_eq!(overall(&[(1, AVAILABLE), (2, PROCESSING)]), PARTIALLY_AVAILABLE);
        assert_eq!(overall(&[(1, PROCESSING), (2, UNKNOWN)]), PROCESSING);
        assert_eq!(overall(&[(1, PENDING)]), PENDING);
        assert_eq!(overall(&[]), UNKNOWN);
    }

    #[test]
    fn quotas_count_what_is_left() {
        let q = Quota { limit: 3, days: 7, used: 2 };
        assert_eq!(q.remaining(), Some(1));
        assert!(!q.restricted());
        assert!(Quota { limit: 3, days: 7, used: 3 }.restricted(), "nothing left is restricted, like Seerr");
        let none = Quota { limit: 0, days: 7, used: 99 };
        assert_eq!(none.remaining(), None);
        assert!(!none.restricted());
    }

    #[test]
    fn anime_is_japanese_animation_and_its_choice_is_checked() {
        assert!(is_anime(&["Animation".into(), "Action".into()], "ja"));
        assert!(!is_anime(&["Animation".into()], "en"));
        assert!(check_anime_choice("dub", "en").is_ok());
        assert!(check_anime_choice("loud", "en").is_err());
        assert!(check_anime_choice("sub", "eng").is_err());
    }

    #[test]
    fn a_request_is_fulfilled_by_its_own_seasons() {
        let r = Request { kind: "tv".into(), seasons: vec![1], status: status::APPROVED, ..Request::default() };
        let a = Availability { status: media_status::PARTIALLY_AVAILABLE, seasons: vec![(1, 5), (2, 3)], library_id: Some(1) };
        assert!(fulfilled(&r, &a));
        let r2 = Request { seasons: vec![1, 2], ..r };
        assert!(!fulfilled(&r2, &a));
    }
}
