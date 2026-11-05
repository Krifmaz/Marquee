//! The Seerr API at `/api/v1`, so the tools that talk to Seerr (Overseerr, Jellyseerr) keep working against
//! Marquee unchanged: LunaSea, nzb360, Homepage's widget, Requestrr-style bots. The web app uses it too. Answers
//! have Seerr's shape, checked against real Jellyseerr 2.7.3 answers in `tests/contracts/seerr/`.
//!
//! Sign-in: the server's API key in `X-Api-Key` (as the owner; with `X-Api-User: <id>` as that account, like
//! Seerr), an app token, or the web session. Errors are `{"message", "detail"}`: Seerr's name and the kit's.
//!
//! ```text
//! GET    /status, /status/appdata, /settings/public       (settings/public needs no sign-in)
//! GET    /auth/me
//! GET    /request?take&skip&filter&sort&sortDirection&requestedBy&mediaType   POST /request (201)
//! GET    /request/count
//! GET    /request/{id}   PUT /request/{id} (admins)   DELETE /request/{id} (admins, or your own waiting one)
//! POST   /request/{id}/approve | /decline | /retry (admins)
//! GET    /media?take&skip&filter&sort    DELETE /media/{id}    POST /media/{id}/{available|partial|processing|pending|unknown}
//! GET    /user  /user/{id}  /user/{id}/requests  /user/{id}/quota
//! GET    /user/{id}/settings/main (POST: limits, admins; anime language, the account itself)
//! GET    /user/{id}/settings/permissions (POST: admins)
//! GET    /discover/movies, /discover/movies/upcoming, /discover/tv, /discover/tv/upcoming, /discover/trending
//! GET    /search?query&page, /movie/{id}, /movie/{id}/recommendations|similar, /tv/{id},
//!        /tv/{id}/season/{n}, /tv/{id}/recommendations|similar
//! GET    /tv/{id}/anilist                    Marquee's own: AniList's seasons of an anime
//! POST   /settings/notifications/test        Marquee's own: a test notification to every place set up (admins)
//! ```

use super::{
    all, all_media, ask, availability, decide, discover, extra, find, media, quotas, remove, remove_media, save, stamp, status, user, when,
    Ask, Asker, Media, Request,
};
use crate::roles::Role;
use crate::AppState;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The Seerr version Marquee answers as, so clients' version checks pass (Marquee's own is in `commitTag`).
pub const SEERR_VERSION: &str = "2.7.3";

/// Seerr's permission bits that Marquee's roles map to.
pub mod perm {
    /// everything
    pub const ADMIN: i64 = 2;
    /// approve and change requests
    pub const MANAGE_REQUESTS: i64 = 16;
    /// ask for movies and TV
    pub const REQUEST: i64 = 32;
    /// movie and TV requests skip approval
    pub const AUTO_APPROVE: i64 = 128;
    /// movie requests skip approval
    pub const AUTO_APPROVE_MOVIE: i64 = 256;
    /// TV requests skip approval
    pub const AUTO_APPROVE_TV: i64 = 512;
    /// see everybody's requests
    pub const REQUEST_VIEW: i64 = 16384;
}

/// An error in Seerr's shape and the kit's: `{"message": …, "detail": …}`.
pub struct Fail(pub kit_server_core::Error);

impl From<kit_server_core::Error> for Fail {
    fn from(e: kit_server_core::Error) -> Fail {
        Fail(e)
    }
}

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        (self.0.status, Json(json!({ "message": self.0.detail, "detail": self.0.detail }))).into_response()
    }
}

/// What a Seerr handler returns.
pub type Answer = std::result::Result<Response, Fail>;

/// JSON with status 200.
fn ok(v: Value) -> Answer {
    Ok(Json(v).into_response())
}

/// The signed-in account of a Seerr call (with `X-Api-User`, the account the API key acts for).
pub struct Seerr(pub Asker);

impl FromRequestParts<AppState> for Seerr {
    type Rejection = Fail;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> std::result::Result<Seerr, Fail> {
        let (account, _) = crate::auth::signed_in_with_role(st, parts)?;
        let key = parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()).unwrap_or("");
        let want = st.settings.str("api_key");
        let as_user = parts.headers.get("x-api-user").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<i64>().ok());
        let id = match as_user {
            Some(u) if !want.is_empty() && key == want => u,
            _ => account.id,
        };
        let row =
            st.db.one("SELECT * FROM users WHERE id=? AND disabled=0", p![id])?.ok_or_else(|| err(403, "There is no such account"))?;
        let mut row = row;
        row.remove("password_hash");
        Ok(Seerr(Asker { role: Role::parse(&row.s("role")), row }))
    }
}

impl Seerr {
    /// `Ok` for admins, else 403.
    fn admin(&self) -> std::result::Result<(), Fail> {
        if self.0.admin() {
            Ok(())
        } else {
            Err(Fail(err(403, "Only an admin can do this")))
        }
    }
}

/// The Seerr routes.
pub fn router() -> Router<AppState> {
    let v1 = Router::new()
        .route("/status", get(status_info))
        .route("/status/appdata", get(appdata))
        .route("/settings/public", get(public_settings))
        .route("/auth/me", get(me))
        .route("/request", get(list_requests).post(create_request))
        .route("/request/count", get(count))
        .route("/request/{id}", get(one_request).put(change_request).delete(delete_request))
        .route("/request/{id}/retry", post(retry))
        .route("/request/{id}/{action}", post(approve_or_decline))
        .route("/media", get(list_media))
        .route("/media/{id}", axum::routing::delete(delete_media))
        .route("/media/{id}/{status}", post(set_media_status))
        .route("/user", get(list_users))
        .route("/user/{id}", get(one_user))
        .route("/user/{id}/requests", get(user_requests))
        .route("/user/{id}/quota", get(user_quota))
        .route("/user/{id}/settings/main", get(main_settings).post(set_main_settings))
        .route("/user/{id}/settings/permissions", get(permissions_of).post(set_permissions))
        .route("/discover/movies", get(discover_movies))
        .route("/discover/movies/upcoming", get(upcoming_movies))
        .route("/discover/tv", get(discover_tv))
        .route("/discover/tv/upcoming", get(upcoming_tv))
        .route("/discover/trending", get(trending))
        .route("/search", get(search))
        .route("/movie/{id}", get(movie))
        .route("/movie/{id}/{list}", get(movie_list))
        .route("/tv/{id}", get(tv))
        .route("/tv/{id}/anilist", get(anilist))
        .route("/tv/{id}/season/{n}", get(tv_season))
        .route("/tv/{id}/{list}", get(tv_list))
        .route("/settings/notifications/test", post(test_notifications));
    Router::new().nest("/api/v1", v1)
}

/// Seerr's Seerr-style permission bits for an account.
pub fn permissions(u: &Row) -> i64 {
    match Role::parse(&u.s("role")) {
        Role::Owner | Role::Admin => perm::ADMIN,
        Role::Viewer => 0,
        Role::Requester => {
            let mut p = perm::REQUEST;
            if u.b("auto_approve_movies") {
                p |= perm::AUTO_APPROVE_MOVIE;
            }
            if u.b("auto_approve_tv") {
                p |= perm::AUTO_APPROVE_TV;
            }
            p
        }
    }
}

/// A kit time (seconds since 1970) as Seerr writes times.
pub fn seconds_iso(t: f64) -> String {
    chrono::DateTime::from_timestamp(t as i64, ((t.fract()) * 1e9) as u32).unwrap_or_default().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// A limit as Seerr sends it: `null` for none (0).
fn limit(v: i64) -> Value {
    if v > 0 {
        json!(v)
    } else {
        Value::Null
    }
}

/// An account in Seerr's shape (`settings` is null: Seerr's per-account settings live elsewhere in Marquee).
pub fn user_json(db: &SqliteDb, u: &Row) -> kit_server_core::Result<Value> {
    let count = db.one("SELECT COUNT(*) AS n FROM requests WHERE requested_by=?", p![u.i("id")])?.map(|r| r.i("n")).unwrap_or(0);
    let created = seconds_iso(u.f("created_at"));
    let limited = |l: &str, d: &str| if u.i(l) > 0 { (limit(u.i(l)), json!(u.i(d))) } else { (Value::Null, Value::Null) };
    let (ml, md) = limited("movie_quota_limit", "movie_quota_days");
    let (tl, td) = limited("tv_quota_limit", "tv_quota_days");
    let mut v = json!({
        "permissions": permissions(u),
        "warnings": [],
        "id": u.i("id"),
        "email": u.s("email"),
        "plexUsername": null,
        "jellyfinUsername": null,
        "username": u.s("username"),
        "recoveryLinkExpirationDate": null,
        "userType": 2,
        "plexId": null,
        "jellyfinUserId": null,
        "avatar": "",
        "avatarETag": null,
        "avatarVersion": null,
        "movieQuotaLimit": ml,
        "movieQuotaDays": md,
        "tvQuotaLimit": tl,
        "tvQuotaDays": td,
        "createdAt": created,
        "updatedAt": created,
        "requestCount": count,
        "displayName": u.s("username"),
        "role": Role::parse(&u.s("role")).as_str(),
    });
    v["settings"] = Value::Null;
    Ok(v)
}

/// The account `id` in Seerr's shape, or `null` when it is gone.
fn user_or_null(db: &SqliteDb, id: Option<i64>) -> kit_server_core::Result<Value> {
    match id.and_then(|i| user(db, i).ok()) {
        Some(u) => user_json(db, &u),
        None => Ok(Value::Null),
    }
}

/// A word from a title for Seerr's `externalServiceSlug` (`game-of-thrones`).
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// A title in Seerr's media shape: its availability (and each season's), the library's id as
/// `externalServiceId`, and its downloads as `downloadStatus`.
pub fn media_json(db: &SqliteDb, m: &Media, seasons: bool) -> kit_server_core::Result<Value> {
    let a = availability(db, m)?;
    let downloads: Vec<Value> = match a.library_id {
        Some(id) => crate::downloads::active(db)?
            .into_iter()
            .filter(|d| if m.kind == "movie" { d.movie_id == Some(id) } else { d.series_id == Some(id) })
            .map(|d| {
                let eta = d.eta.map(|s| (chrono::Utc::now() + chrono::Duration::seconds(s)).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
                json!({
                    "mediaType": m.kind, "externalId": id, "size": d.size, "sizeLeft": d.left, "status": d.status,
                    "timeLeft": d.eta.map(|s| format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)).unwrap_or_default(),
                    "estimatedCompletionTime": eta, "title": d.title, "downloadId": d.lantern_id,
                })
            })
            .collect(),
        None => vec![],
    };
    let mut v = json!({
        "downloadStatus": downloads,
        "downloadStatus4k": [],
        "id": m.id,
        "mediaType": m.kind,
        "tmdbId": m.tmdb_id,
        "tvdbId": m.tvdb_id,
        "imdbId": m.imdb_id,
        "status": a.status,
        "status4k": 1,
        "createdAt": m.created_at,
        "updatedAt": m.updated_at,
        "lastSeasonChange": m.updated_at,
        "mediaAddedAt": m.created_at,
        "serviceId": a.library_id.map(|_| 0),
        "serviceId4k": null,
        "externalServiceId": a.library_id,
        "externalServiceId4k": null,
        "externalServiceSlug": a.library_id.map(|_| if m.kind == "movie" { m.tmdb_id.to_string() } else { slug(&m.title) }),
        "externalServiceSlug4k": null,
        "ratingKey": null,
        "ratingKey4k": null,
        "jellyfinMediaId": null,
        "jellyfinMediaId4k": null,
    });
    if seasons {
        v["seasons"] = json!(a
            .seasons
            .iter()
            .map(|(n, st)| json!({
                "id": m.id * 1000 + n, "seasonNumber": n, "status": st, "status4k": 1,
                "createdAt": m.created_at, "updatedAt": m.updated_at,
            }))
            .collect::<Vec<_>>());
    }
    Ok(v)
}

/// A request in Seerr's shape, with Marquee's words (`statusWords`, title, poster, anime choice) added. `viewer`
/// decides `canRemove`.
pub fn request_json(db: &SqliteDb, r: &Request, viewer: Option<&Asker>) -> kit_server_core::Result<Value> {
    let m = media(db, r.media_id)?.unwrap_or_else(|| Media { id: r.media_id, kind: r.kind.clone(), ..Media::default() });
    let seasons: Vec<Value> = r
        .seasons
        .iter()
        .map(|n| json!({ "id": r.id * 1000 + n, "seasonNumber": n, "status": r.status, "createdAt": r.created_at, "updatedAt": r.updated_at }))
        .collect();
    let can_remove = viewer.is_some_and(|v| v.admin() || (v.id() == r.requested_by && r.status == status::PENDING));
    let mut v = json!({
        "id": r.id,
        "status": r.status,
        "createdAt": r.created_at,
        "updatedAt": r.updated_at,
        "type": r.kind,
        "is4k": r.is4k,
        "serverId": 0,
        "profileId": r.profile_id,
        "rootFolder": r.root_folder,
        "languageProfileId": r.language_profile_id,
        "tags": if r.tags.is_empty() { Value::Null } else { json!(r.tags) },
        "isAutoRequest": r.is_auto,
        "media": media_json(db, &m, true)?,
        "seasons": seasons,
        "modifiedBy": user_or_null(db, r.modified_by)?,
        "requestedBy": user_or_null(db, Some(r.requested_by))?,
        "seasonCount": r.seasons.len(),
        "canRemove": can_remove,
    });
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra(db, r, &m)?) {
        o.extend(e);
    }
    Ok(v)
}

/// The paging of a list answer.
#[derive(Deserialize, Default)]
pub struct Paging {
    /// how many per page
    pub take: Option<usize>,
    /// how many to skip
    pub skip: Option<usize>,
}

/// One page of `items` in Seerr's shape: `{pageInfo: {pages, pageSize, results, page}, results}`.
pub fn paged(items: Vec<Value>, q: &Paging, default_take: usize) -> Value {
    let take = q.take.filter(|t| *t > 0).unwrap_or(default_take).min(1000);
    let skip = q.skip.unwrap_or(0);
    let total = items.len();
    let results: Vec<Value> = items.into_iter().skip(skip).take(take).collect();
    json!({
        "pageInfo": { "pages": total.div_ceil(take), "pageSize": take, "results": total, "page": skip / take + 1 },
        "results": results,
    })
}

/// `GET /status`.
async fn status_info() -> Json<Value> {
    Json(json!({
        "version": SEERR_VERSION,
        "commitTag": format!("marquee-{}", crate::VERSION),
        "updateAvailable": false,
        "commitsBehind": 0,
        "restartRequired": false,
    }))
}

/// `GET /status/appdata`.
async fn appdata(State(st): State<AppState>) -> Json<Value> {
    Json(json!({ "appData": true, "appDataPath": st.data_dir.display().to_string(), "appDataPermissions": true }))
}

/// `GET /settings/public`: what the sign-in page and clients read before signing in.
async fn public_settings(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "initialized": true,
        "applicationTitle": st.app_name(),
        "applicationUrl": "",
        "hideAvailable": false,
        "hideBlacklisted": false,
        "localLogin": true,
        "mediaServerLogin": false,
        "jellyfinExternalHost": "",
        "jellyfinForgotPasswordUrl": "",
        "movie4kEnabled": false,
        "series4kEnabled": false,
        "discoverRegion": "",
        "streamingRegion": "",
        "originalLanguage": "",
        "mediaServerType": 4,
        "partialRequestsEnabled": true,
        "enableSpecialEpisodes": false,
        "cacheImages": false,
        "vapidPublic": "",
        "enablePushRegistration": false,
        "locale": "en",
        "emailEnabled": false,
        "userEmailRequired": false,
        "newPlexLogin": true,
        "youtubeUrl": "",
    }))
}

/// `GET /auth/me`.
async fn me(State(st): State<AppState>, s: Seerr) -> Answer {
    let mut v = user_json(&st.db, &s.0.row)?;
    let (audio, subs) = super::anime_prefs(&st.db, s.0.id())?;
    v["animeAudio"] = json!(audio);
    v["animeSubtitles"] = json!(subs);
    ok(v)
}

/// `GET /request` query.
#[derive(Deserialize, Default)]
pub struct RequestQuery {
    /// how many per page (10)
    pub take: Option<usize>,
    /// how many to skip
    pub skip: Option<usize>,
    /// `all`, `pending`, `approved`, `processing`, `available`, `unavailable`, `failed`, `completed`, `declined`
    pub filter: Option<String>,
    /// `added` (default) or `modified`
    pub sort: Option<String>,
    /// `desc` (default) or `asc`
    #[serde(rename = "sortDirection")]
    pub sort_direction: Option<String>,
    /// one account's requests
    #[serde(rename = "requestedBy")]
    pub requested_by: Option<i64>,
    /// `movie`, `tv` or `all`
    #[serde(rename = "mediaType")]
    pub media_type: Option<String>,
}

/// Whether request `r` matches Seerr's `filter`.
pub fn matches(db: &SqliteDb, r: &Request, filter: &str) -> bool {
    let available =
        || media(db, r.media_id).ok().flatten().and_then(|m| availability(db, &m).ok()).is_some_and(|a| super::fulfilled(r, &a));
    match filter {
        "pending" => r.status == status::PENDING,
        "approved" => r.status == status::APPROVED,
        "declined" => r.status == status::DECLINED,
        "failed" => r.status == status::FAILED,
        "completed" => r.status == status::COMPLETED,
        "processing" => r.status == status::APPROVED && !available(),
        "available" => r.status == status::COMPLETED || (r.status == status::APPROVED && available()),
        "unavailable" => matches!(r.status, status::PENDING | status::APPROVED) && !available(),
        "deleted" => false,
        _ => true,
    }
}

/// The requests `s` may see, filtered and sorted as `q` asks.
fn requests_for(st: &AppState, s: &Seerr, q: &RequestQuery, only: Option<i64>) -> kit_server_core::Result<Vec<Value>> {
    let filter = q.filter.clone().unwrap_or_else(|| "all".into());
    let kind = q.media_type.clone().unwrap_or_else(|| "all".into());
    let mut list: Vec<Request> = all(&st.db)?
        .into_iter()
        .filter(|r| s.0.admin() || r.requested_by == s.0.id())
        .filter(|r| only.or(q.requested_by).is_none_or(|u| r.requested_by == u))
        .filter(|r| kind == "all" || r.kind == kind)
        .filter(|r| matches(&st.db, r, &filter))
        .collect();
    let modified = q.sort.as_deref() == Some("modified");
    list.sort_by_key(|r| (when(if modified { &r.updated_at } else { &r.created_at }), r.id));
    if q.sort_direction.as_deref() != Some("asc") {
        list.reverse();
    }
    list.iter().map(|r| request_json(&st.db, r, Some(&s.0))).collect()
}

/// `GET /request`.
async fn list_requests(State(st): State<AppState>, s: Seerr, Query(q): Query<RequestQuery>) -> Answer {
    let items = requests_for(&st, &s, &q, None)?;
    ok(paged(items, &Paging { take: q.take, skip: q.skip }, 10))
}

/// `POST /request` body (Seerr's names).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRequest {
    /// `movie` or `tv`
    pub media_type: String,
    /// TMDB's id
    pub media_id: i64,
    /// TheTVDB's id
    pub tvdb_id: Option<i64>,
    /// season numbers, or `"all"`
    pub seasons: Option<Value>,
    /// a 4K request
    #[serde(default)]
    pub is4k: bool,
    /// Seerr's Radarr/Sonarr server (ignored: Marquee is both)
    pub server_id: Option<i64>,
    /// a quality profile (admins)
    pub profile_id: Option<i64>,
    /// a root folder (admins)
    pub root_folder: Option<String>,
    /// Sonarr's language profile
    pub language_profile_id: Option<i64>,
    /// tags (admins)
    pub tags: Option<Vec<i64>>,
    /// ask in another account's name (admins)
    pub user_id: Option<i64>,
    /// don't count toward the limit (admins)
    #[serde(default)]
    pub ignore_quota: bool,
    /// anime: `dub`, `dual` or `sub` (Marquee's own)
    pub anime_audio: Option<String>,
    /// anime: the subtitle language (Marquee's own)
    pub anime_subtitles: Option<String>,
}

/// Seerr's `seasons` (`[1, 2]` or `"all"`) as season numbers; empty means all.
pub fn season_list(v: &Option<Value>) -> Vec<i64> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_i64).collect(),
        _ => vec![],
    }
}

/// `POST /request`: 201 with the new request.
async fn create_request(State(st): State<AppState>, s: Seerr, Json(b): Json<NewRequest>) -> Answer {
    let a = Ask {
        kind: b.media_type.clone(),
        tmdb_id: b.media_id,
        tvdb_id: b.tvdb_id,
        seasons: season_list(&b.seasons),
        is4k: b.is4k,
        profile_id: b.profile_id,
        root_folder: b.root_folder,
        language_profile_id: b.language_profile_id,
        tags: b.tags.unwrap_or_default(),
        user_id: b.user_id,
        ignore_quota: b.ignore_quota,
        anime_audio: b.anime_audio,
        anime_subtitles: b.anime_subtitles,
    };
    let _ = b.server_id;
    let r = ask(&st, &s.0, a).await?;
    Ok((StatusCode::CREATED, Json(request_json(&st.db, &r, Some(&s.0))?)).into_response())
}

/// `GET /request/count`.
async fn count(State(st): State<AppState>, s: Seerr) -> Answer {
    let list: Vec<Request> = all(&st.db)?.into_iter().filter(|r| s.0.admin() || r.requested_by == s.0.id()).collect();
    let n = |f: &dyn Fn(&Request) -> bool| list.iter().filter(|r| f(r)).count();
    ok(json!({
        "total": list.len(),
        "movie": n(&|r| r.kind == "movie"),
        "tv": n(&|r| r.kind == "tv"),
        "pending": n(&|r| r.status == status::PENDING),
        "approved": n(&|r| r.status == status::APPROVED),
        "declined": n(&|r| r.status == status::DECLINED),
        "processing": n(&|r| matches(&st.db, r, "processing")),
        "available": n(&|r| matches(&st.db, r, "available")),
    }))
}

/// The request `id` if `s` may see it: 404 "Request not found." (Seerr's words), 403 for somebody else's.
fn visible(st: &AppState, s: &Seerr, id: i64) -> std::result::Result<Request, Fail> {
    let r = find(&st.db, id)?.ok_or_else(|| err(404, "Request not found."))?;
    if !s.0.admin() && r.requested_by != s.0.id() {
        return Err(Fail(err(403, "You can only see your own requests")));
    }
    Ok(r)
}

/// `GET /request/{id}`.
async fn one_request(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let r = visible(&st, &s, id)?;
    ok(request_json(&st.db, &r, Some(&s.0))?)
}

/// `PUT /request/{id}` body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestChange {
    /// `movie` or `tv` (must stay the same)
    pub media_type: Option<String>,
    /// the seasons (TV)
    pub seasons: Option<Value>,
    /// the quality profile
    pub profile_id: Option<i64>,
    /// the root folder
    pub root_folder: Option<String>,
    /// Sonarr's language profile
    pub language_profile_id: Option<i64>,
    /// tags
    pub tags: Option<Vec<i64>>,
    /// move it to another account
    pub user_id: Option<i64>,
    /// anime: `dub`, `dual` or `sub`
    pub anime_audio: Option<String>,
    /// anime: the subtitle language
    pub anime_subtitles: Option<String>,
}

/// `PUT /request/{id}`: change a request (admins).
async fn change_request(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<RequestChange>) -> Answer {
    s.admin()?;
    let mut r = visible(&st, &s, id)?;
    if b.media_type.as_deref().is_some_and(|t| t != r.kind) {
        return Err(Fail(err(400, "A request can't change between movie and TV")));
    }
    if r.kind == "tv" {
        let seasons = season_list(&b.seasons);
        if !seasons.is_empty() {
            r.seasons = seasons;
        }
    }
    if b.profile_id.is_some() {
        r.profile_id = b.profile_id;
    }
    if let Some(f) = b.root_folder {
        r.root_folder = Some(f).filter(|f| !f.trim().is_empty());
    }
    if b.language_profile_id.is_some() {
        r.language_profile_id = b.language_profile_id;
    }
    if let Some(t) = b.tags {
        r.tags = t;
    }
    if let Some(u) = b.user_id {
        r.requested_by = user(&st.db, u)?.i("id");
    }
    if b.anime_audio.is_some() || b.anime_subtitles.is_some() {
        let audio = b.anime_audio.unwrap_or(r.anime_audio.clone());
        let subs = b.anime_subtitles.unwrap_or(r.anime_subtitles.clone()).to_lowercase();
        super::check_anime_choice(&audio, &subs)?;
        (r.anime_audio, r.anime_subtitles) = (audio, subs);
    }
    r.modified_by = Some(s.0.id());
    r.updated_at = stamp();
    save(&st.db, &r)?;
    ok(request_json(&st.db, &r, Some(&s.0))?)
}

/// `DELETE /request/{id}`: 204. Admins any; an account its own while it waits for approval.
async fn delete_request(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let r = visible(&st, &s, id)?;
    if !s.0.admin() && r.status != status::PENDING {
        return Err(Fail(err(403, "Only an admin can remove a request once it was approved or declined")));
    }
    remove(&st.db, id)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /request/{id}/approve` or `/decline` (admins).
async fn approve_or_decline(State(st): State<AppState>, s: Seerr, Path((id, action)): Path<(i64, String)>) -> Answer {
    s.admin()?;
    let yes = match action.as_str() {
        "approve" => true,
        "decline" => false,
        _ => return Err(Fail(err(404, "Unknown action: approve, decline or retry"))),
    };
    let r = visible(&st, &s, id)?;
    let r = decide(&st, r, &s.0, yes).await?;
    ok(request_json(&st.db, &r, Some(&s.0))?)
}

/// `POST /request/{id}/retry`: approve again after it didn't work (admins).
async fn retry(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.admin()?;
    let r = visible(&st, &s, id)?;
    let r = decide(&st, r, &s.0, true).await?;
    ok(request_json(&st.db, &r, Some(&s.0))?)
}

/// `GET /media` query.
#[derive(Deserialize, Default)]
pub struct MediaQuery {
    /// how many per page (20)
    pub take: Option<usize>,
    /// how many to skip
    pub skip: Option<usize>,
    /// `available`, `partial`, `allavailable`, `processing`, `pending`
    pub filter: Option<String>,
    /// `added` (default), `modified` or `mediaAdded`
    pub sort: Option<String>,
}

/// `GET /media`.
async fn list_media(State(st): State<AppState>, _s: Seerr, Query(q): Query<MediaQuery>) -> Answer {
    let mut list = Vec::new();
    for m in all_media(&st.db)? {
        let a = availability(&st.db, &m)?;
        let keep = match q.filter.as_deref().unwrap_or("all") {
            "available" => a.status == super::media_status::AVAILABLE,
            "partial" => a.status == super::media_status::PARTIALLY_AVAILABLE,
            "allavailable" => a.status >= super::media_status::PARTIALLY_AVAILABLE,
            "processing" => a.status == super::media_status::PROCESSING,
            "pending" => a.status == super::media_status::PENDING,
            _ => true,
        };
        if keep {
            list.push(m);
        }
    }
    let modified = q.sort.as_deref() == Some("modified");
    list.sort_by_key(|m| std::cmp::Reverse((when(if modified { &m.updated_at } else { &m.created_at }), m.id)));
    let items = list.iter().map(|m| media_json(&st.db, m, true)).collect::<kit_server_core::Result<Vec<_>>>()?;
    ok(paged(items, &Paging { take: q.take, skip: q.skip }, 20))
}

/// `DELETE /media/{id}`: forget a title and its requests (admins; the library keeps the movie or series).
async fn delete_media(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.admin()?;
    media(&st.db, id)?.ok_or_else(|| err(404, "Media not found."))?;
    remove_media(&st.db, id)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /media/{id}/{status}`: set a title's availability by hand (admins); `unknown` goes back to the library's.
async fn set_media_status(State(st): State<AppState>, s: Seerr, Path((id, word)): Path<(i64, String)>) -> Answer {
    s.admin()?;
    let m = media(&st.db, id)?.ok_or_else(|| err(404, "Media not found."))?;
    let value: Option<i64> = match word.as_str() {
        "available" => Some(super::media_status::AVAILABLE),
        "partial" => Some(super::media_status::PARTIALLY_AVAILABLE),
        "processing" => Some(super::media_status::PROCESSING),
        "pending" => Some(super::media_status::PENDING),
        "unknown" => None,
        _ => return Err(Fail(err(400, "The status must be available, partial, processing, pending or unknown"))),
    };
    st.db.execute("UPDATE request_media SET status_override=?, updated_at=? WHERE id=?", p![value, stamp(), m.id])?;
    let m = media(&st.db, id)?.ok_or_else(|| err(404, "Media not found."))?;
    ok(media_json(&st.db, &m, true)?)
}

/// `GET /user` (admins).
async fn list_users(State(st): State<AppState>, s: Seerr, Query(q): Query<Paging>) -> Answer {
    s.admin()?;
    let rows = st.db.query("SELECT * FROM users ORDER BY id", p![])?;
    let items = rows.iter().map(|u| user_json(&st.db, u)).collect::<kit_server_core::Result<Vec<_>>>()?;
    ok(paged(items, &q, 10))
}

/// The account `id` when `s` is an admin or that account; 403 otherwise.
fn account(st: &AppState, s: &Seerr, id: i64) -> std::result::Result<Row, Fail> {
    if s.0.id() != id {
        s.admin()?;
    }
    Ok(user(&st.db, id)?)
}

/// `GET /user/{id}`.
async fn one_user(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let u = account(&st, &s, id)?;
    ok(user_json(&st.db, &u)?)
}

/// `GET /user/{id}/requests`.
async fn user_requests(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Query(q): Query<RequestQuery>) -> Answer {
    account(&st, &s, id)?;
    let items = requests_for(&st, &s, &q, Some(id))?;
    ok(paged(items, &Paging { take: q.take, skip: q.skip }, 20))
}

/// One quota in Seerr's shape.
fn quota_json(q: &super::Quota) -> Value {
    json!({
        "days": q.days, "limit": q.limit, "used": q.used, "remaining": q.remaining(),
        "restricted": q.remaining() == Some(0),
    })
}

/// `GET /user/{id}/quota`.
async fn user_quota(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let u = account(&st, &s, id)?;
    let (m, t) = quotas(&st.db, &u)?;
    ok(json!({ "movie": quota_json(&m), "tv": quota_json(&t) }))
}

/// `GET /user/{id}/settings/main`.
async fn main_settings(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let u = account(&st, &s, id)?;
    let (audio, subs) = super::anime_prefs(&st.db, id)?;
    ok(json!({
        "username": u.s("username"),
        "email": u.s("email"),
        "movieQuotaLimit": limit(u.i("movie_quota_limit")),
        "movieQuotaDays": u.i("movie_quota_days"),
        "tvQuotaLimit": limit(u.i("tv_quota_limit")),
        "tvQuotaDays": u.i("tv_quota_days"),
        "animeAudio": audio,
        "animeSubtitles": subs,
    }))
}

/// `POST /user/{id}/settings/main` body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainSettings {
    /// movies per period, null or 0 for no limit (admins)
    pub movie_quota_limit: Option<Value>,
    /// the movie period in days (admins)
    pub movie_quota_days: Option<i64>,
    /// seasons per period, null or 0 for no limit (admins)
    pub tv_quota_limit: Option<Value>,
    /// the TV period in days (admins)
    pub tv_quota_days: Option<i64>,
    /// the default anime audio (the account itself or admins)
    pub anime_audio: Option<String>,
    /// the default anime subtitle language
    pub anime_subtitles: Option<String>,
}

/// `POST /user/{id}/settings/main`: limits (admins) and the default anime language choice.
async fn set_main_settings(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<MainSettings>) -> Answer {
    account(&st, &s, id)?;
    let limits = [("movie_quota_limit", &b.movie_quota_limit), ("tv_quota_limit", &b.tv_quota_limit)];
    let days = [("movie_quota_days", b.movie_quota_days), ("tv_quota_days", b.tv_quota_days)];
    let changes_limits = limits.iter().any(|(_, v)| v.is_some()) || days.iter().any(|(_, v)| v.is_some());
    if changes_limits {
        s.admin()?;
    }
    for (col, v) in limits {
        if let Some(v) = v {
            let n = v.as_i64().unwrap_or(0);
            if n < 0 {
                return Err(Fail(err(400, "A request limit can't be below 0 (0 means no limit)")));
            }
            // the column names come from the fixed list above, never from the request
            st.db.execute(&format!("UPDATE users SET {col}=? WHERE id=?"), p![n, id])?;
        }
    }
    for (col, v) in days {
        if let Some(d) = v {
            if !(1..=365).contains(&d) {
                return Err(Fail(err(400, "A request limit's period must be 1 to 365 days")));
            }
            st.db.execute(&format!("UPDATE users SET {col}=? WHERE id=?"), p![d, id])?;
        }
    }
    if b.anime_audio.is_some() || b.anime_subtitles.is_some() {
        let (da, ds) = super::anime_prefs(&st.db, id)?;
        super::set_anime_prefs(&st.db, id, &b.anime_audio.unwrap_or(da), &b.anime_subtitles.unwrap_or(ds))?;
    }
    main_settings(State(st), s, Path(id)).await
}

/// `GET /user/{id}/settings/permissions`.
async fn permissions_of(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let u = account(&st, &s, id)?;
    ok(json!({ "permissions": permissions(&u) }))
}

/// `POST /user/{id}/settings/permissions` body.
#[derive(Deserialize)]
pub struct PermissionsBody {
    /// Seerr's permission bits
    pub permissions: i64,
}

/// The role and auto-approve choices that Seerr's permission bits `p` stand for.
pub fn role_of(p: i64) -> (Role, bool, bool) {
    if p & perm::ADMIN != 0 {
        return (Role::Admin, false, false);
    }
    let both = p & perm::AUTO_APPROVE != 0;
    let can = p & (perm::REQUEST | 262144 | 524288) != 0;
    if !can {
        return (Role::Viewer, false, false);
    }
    (Role::Requester, both || p & perm::AUTO_APPROVE_MOVIE != 0, both || p & perm::AUTO_APPROVE_TV != 0)
}

/// `POST /user/{id}/settings/permissions`: Seerr's bits become Marquee's role and auto-approve (admins; never the
/// owner's or your own).
async fn set_permissions(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<PermissionsBody>) -> Answer {
    s.admin()?;
    let u = user(&st.db, id)?;
    if Role::parse(&u.s("role")) == Role::Owner || id == s.0.id() {
        return Err(Fail(err(400, "The owner's and your own permissions can't be changed here")));
    }
    let (role, movies, tv) = role_of(b.permissions);
    st.db.execute(
        "UPDATE users SET role=?, auto_approve_movies=?, auto_approve_tv=? WHERE id=?",
        p![role.as_str(), movies as i64, tv as i64, id],
    )?;
    let u = user(&st.db, id)?;
    ok(json!({ "permissions": permissions(&u) }))
}

/// The discovery query parameters.
#[derive(Deserialize, Default)]
pub struct DiscoverQuery {
    /// the page, from 1
    pub page: Option<i64>,
    /// the language, `en`
    pub language: Option<String>,
    /// what to search for (`/search`)
    pub query: Option<String>,
    /// a genre id
    pub genre: Option<String>,
    /// TMDB's sort (`popularity.desc`)
    #[serde(rename = "sortBy")]
    pub sort_by: Option<String>,
}

impl DiscoverQuery {
    /// TMDB's parameters for a discover call.
    fn tmdb(&self) -> Vec<(&'static str, String)> {
        let mut q = discover::base_query(self.page, self.language.as_deref());
        q.push(("sort_by", self.sort_by.clone().unwrap_or_else(|| "popularity.desc".into())));
        if let Some(g) = self.genre.as_ref().filter(|g| !g.is_empty()) {
            q.push(("with_genres", g.clone()));
        }
        q
    }
}

/// `GET /discover/movies`.
async fn discover_movies(State(st): State<AppState>, _s: Seerr, Query(q): Query<DiscoverQuery>) -> Answer {
    ok(discover::tmdb_page(&st, "/discover/movie", &q.tmdb(), Some("movie")).await?)
}

/// `GET /discover/movies/upcoming`.
async fn upcoming_movies(State(st): State<AppState>, _s: Seerr, Query(q): Query<DiscoverQuery>) -> Answer {
    let mut t = q.tmdb();
    t.push(("primary_release_date.gte", discover::today()));
    ok(discover::tmdb_page(&st, "/discover/movie", &t, Some("movie")).await?)
}

/// `GET /discover/tv`.
async fn discover_tv(State(st): State<AppState>, _s: Seerr, Query(q): Query<DiscoverQuery>) -> Answer {
    ok(discover::tmdb_page(&st, "/discover/tv", &q.tmdb(), Some("tv")).await?)
}

/// `GET /discover/tv/upcoming`.
async fn upcoming_tv(State(st): State<AppState>, _s: Seerr, Query(q): Query<DiscoverQuery>) -> Answer {
    let mut t = q.tmdb();
    t.push(("first_air_date.gte", discover::today()));
    ok(discover::tmdb_page(&st, "/discover/tv", &t, Some("tv")).await?)
}

/// `GET /discover/trending`: movies and TV trending today.
async fn trending(State(st): State<AppState>, _s: Seerr, Query(q): Query<DiscoverQuery>) -> Answer {
    let t = discover::base_query(q.page, q.language.as_deref());
    ok(discover::tmdb_page(&st, "/trending/all/day", &t, None).await?)
}

/// `GET /search?query=`: movies, TV and people.
async fn search(State(st): State<AppState>, _s: Seerr, Query(q): Query<DiscoverQuery>) -> Answer {
    let term = q.query.clone().unwrap_or_default();
    if term.trim().is_empty() {
        return ok(json!({ "page": 1, "totalPages": 1, "totalResults": 0, "results": [] }));
    }
    let mut t = discover::base_query(q.page, q.language.as_deref());
    t.push(("query", term.trim().to_string()));
    ok(discover::tmdb_page(&st, "/search/multi", &t, None).await?)
}

/// `GET /movie/{id}`.
async fn movie(State(st): State<AppState>, _s: Seerr, Path(id): Path<i64>, Query(q): Query<DiscoverQuery>) -> Answer {
    ok(discover::movie(&st, id, q.language.as_deref()).await?)
}

/// `GET /movie/{id}/recommendations` or `/similar`.
async fn movie_list(
    State(st): State<AppState>,
    _s: Seerr,
    Path((id, list)): Path<(i64, String)>,
    Query(q): Query<DiscoverQuery>,
) -> Answer {
    if list != "recommendations" && list != "similar" {
        return Err(Fail(err(404, "Unknown list: recommendations or similar")));
    }
    let t = discover::base_query(q.page, q.language.as_deref());
    ok(discover::tmdb_page(&st, &format!("/movie/{id}/{list}"), &t, Some("movie")).await?)
}

/// `GET /tv/{id}`.
async fn tv(State(st): State<AppState>, _s: Seerr, Path(id): Path<i64>, Query(q): Query<DiscoverQuery>) -> Answer {
    ok(discover::tv(&st, id, q.language.as_deref()).await?)
}

/// `GET /tv/{id}/season/{n}`.
async fn tv_season(State(st): State<AppState>, _s: Seerr, Path((id, n)): Path<(i64, i64)>, Query(q): Query<DiscoverQuery>) -> Answer {
    let t: Vec<(&str, String)> = discover::base_query(None, q.language.as_deref()).into_iter().filter(|(k, _)| *k == "language").collect();
    let v = crate::movies::tmdb(&st).get(&format!("/tv/{id}/season/{n}"), &t).await?;
    ok(discover::camel(&v))
}

/// `GET /tv/{id}/recommendations` or `/similar`.
async fn tv_list(State(st): State<AppState>, _s: Seerr, Path((id, list)): Path<(i64, String)>, Query(q): Query<DiscoverQuery>) -> Answer {
    if list != "recommendations" && list != "similar" {
        return Err(Fail(err(404, "Unknown list: recommendations or similar")));
    }
    let t = discover::base_query(q.page, q.language.as_deref());
    ok(discover::tmdb_page(&st, &format!("/tv/{id}/{list}"), &t, Some("tv")).await?)
}

/// `GET /tv/{id}/anilist`: AniList's entries (each season on its own, with its airing status) for an anime.
async fn anilist(State(st): State<AppState>, _s: Seerr, Path(id): Path<i64>) -> Answer {
    let title = match super::media_by_tmdb(&st.db, "tv", id)? {
        Some(m) if !m.title.is_empty() => m.title,
        _ => super::facts(&st, "tv", id).await?.title,
    };
    let entries = super::anilist::search(&st.http, &st.settings.str("request_anilist_url"), &title).await?;
    ok(json!({ "title": title, "entries": entries }))
}

/// `POST /settings/notifications/test` (admins): a test to every place set up, and how each went.
async fn test_notifications(State(st): State<AppState>, s: Seerr) -> Answer {
    s.admin()?;
    let n = super::notify::note(super::notify::Event::Test, &Request::default(), &Media::default(), "");
    let sent = super::notify::send(&st, &n).await;
    if sent.is_empty() {
        return Err(Fail(err(400, "No place is set up yet. Add a Discord webhook or an ntfy topic in Settings → Request news.")));
    }
    let list: Vec<Value> = sent
        .into_iter()
        .map(|(place, r)| json!({ "place": place, "ok": r.is_ok(), "detail": r.err().map(|e| e.detail).unwrap_or_default() }))
        .collect();
    ok(json!({ "results": list }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_count_like_seerr() {
        let items: Vec<Value> = (0..25).map(|i| json!(i)).collect();
        let p = paged(items.clone(), &Paging { take: Some(10), skip: Some(20) }, 10);
        assert_eq!(p["pageInfo"], json!({ "pages": 3, "pageSize": 10, "results": 25, "page": 3 }));
        assert_eq!(p["results"].as_array().map(Vec::len), Some(5));
        assert_eq!(paged(vec![], &Paging::default(), 20)["pageInfo"]["pages"], 0);
    }

    #[test]
    fn seerr_permissions_map_to_roles() {
        assert_eq!(role_of(perm::ADMIN), (Role::Admin, false, false));
        assert_eq!(role_of(perm::REQUEST), (Role::Requester, false, false));
        assert_eq!(role_of(perm::REQUEST | perm::AUTO_APPROVE), (Role::Requester, true, true));
        assert_eq!(role_of(perm::REQUEST | perm::AUTO_APPROVE_TV), (Role::Requester, false, true));
        assert_eq!(role_of(0), (Role::Viewer, false, false));
        assert_eq!(slug("Game of Thrones"), "game-of-thrones");
        assert_eq!(slug("Mission: Impossible – 2"), "mission-impossible-2");
        assert_eq!(season_list(&Some(json!("all"))), Vec::<i64>::new());
        assert_eq!(season_list(&Some(json!([1, 2]))), vec![1, 2]);
    }
}
