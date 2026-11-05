//! The Seerr API at `/api/v1`, so the tools that talk to Seerr (Overseerr, Jellyseerr) keep working against
//! Marquee unchanged: LunaSea, nzb360, Homepage's widget, Requestrr-style bots. The web app uses it too. Answers
//! have Seerr's shape, checked against real Jellyseerr 2.7.3 answers in `tests/contracts/seerr/`.
//!
//! The request routes are ported from Seerr 3.5.1's `server/routes/request.ts` (the commit `UPSTREAM.lock` pins):
//! the same permission bits, statuses and words.
//!
//! Sign-in: the server's API key in `X-Api-Key` (as the owner; with `X-Api-User: <id>` as that account, like
//! Seerr), an app token, or the web session. Errors are `{"message", "detail"}`: Seerr's name and the kit's.
//!
//! ```text
//! GET    /status, /status/appdata, /settings/public       (settings/public needs no sign-in)
//! GET    /auth/me
//! GET    /request?take&skip&filter&sort&sortDirection&requestedBy&mediaType   POST /request (201)
//! GET    /request/count
//! GET    /request/{id}   PUT /request/{id} (a waiting one: yours, or any with MANAGE_REQUESTS)
//! DELETE /request/{id} (MANAGE_REQUESTS, or your own waiting one)
//! POST   /request/{id}/approve | /decline | /retry (MANAGE_REQUESTS)
//! GET    /media?take&skip&filter&sort    DELETE /media/{id}    DELETE /media/{id}/file    GET /media/{id}/watch_data
//! POST   /media/{id}/{available|partial|processing|pending|unknown}   (MANAGE_REQUESTS; watch_data: admins)
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

/// Seerr's permission bits (`server/lib/permissions.ts`); Marquee's roles map to some of them.
pub mod perm {
    /// everything
    pub const ADMIN: i64 = 2;
    /// change settings
    pub const MANAGE_SETTINGS: i64 = 4;
    /// manage accounts
    pub const MANAGE_USERS: i64 = 8;
    /// approve and change requests
    pub const MANAGE_REQUESTS: i64 = 16;
    /// ask for movies and TV
    pub const REQUEST: i64 = 32;
    /// vote (unused by Seerr)
    pub const VOTE: i64 = 64;
    /// movie and TV requests skip approval
    pub const AUTO_APPROVE: i64 = 128;
    /// movie requests skip approval
    pub const AUTO_APPROVE_MOVIE: i64 = 256;
    /// TV requests skip approval
    pub const AUTO_APPROVE_TV: i64 = 512;
    /// ask for 4K movies and TV
    pub const REQUEST_4K: i64 = 1024;
    /// ask for 4K movies
    pub const REQUEST_4K_MOVIE: i64 = 2048;
    /// ask for 4K TV
    pub const REQUEST_4K_TV: i64 = 4096;
    /// choose the server, profile, folder and tags of a request
    pub const REQUEST_ADVANCED: i64 = 8192;
    /// see everybody's requests
    pub const REQUEST_VIEW: i64 = 16384;
    /// 4K movie and TV requests skip approval
    pub const AUTO_APPROVE_4K: i64 = 32768;
    /// 4K movie requests skip approval
    pub const AUTO_APPROVE_4K_MOVIE: i64 = 65536;
    /// 4K TV requests skip approval
    pub const AUTO_APPROVE_4K_TV: i64 = 131072;
    /// ask for movies
    pub const REQUEST_MOVIE: i64 = 262144;
    /// ask for TV
    pub const REQUEST_TV: i64 = 524288;
}

/// Seerr's `hasPermission(bits, value, {type: 'or'})` for the account `u`: admins have every bit.
pub fn has_any(u: &Row, bits: &[i64]) -> bool {
    let v = permissions(u);
    v & perm::ADMIN != 0 || bits.iter().any(|b| v & b != 0)
}

/// Seerr's `hasPermission(bits, value)` (`and`) for the account `u`: admins have every bit.
pub fn has_all(u: &Row, bits: &[i64]) -> bool {
    let v = permissions(u);
    v & perm::ADMIN != 0 || bits.iter().all(|b| v & b != 0)
}

/// Seerr's answer when an account lacks the permission a route needs (`isAuthenticated(permission)`).
fn no_access() -> Fail {
    Fail(err(403, "You do not have permission to access this endpoint"))
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
    /// `Ok` when the account has any of `bits`, else Seerr's 403.
    fn needs(&self, bits: &[i64]) -> std::result::Result<(), Fail> {
        if self.0.has(bits) {
            Ok(())
        } else {
            Err(no_access())
        }
    }

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
        .route("/media/{id}/file", axum::routing::delete(delete_media_file))
        .route("/media/{id}/watch_data", get(watch_data))
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
        "pageInfo": { "pages": total.div_ceil(take), "pageSize": take, "results": total, "page": skip.div_ceil(take) + 1 },
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

/// The request statuses and title statuses Seerr's `filter` keeps (`routes/request.ts`), or `None` for all.
/// `declined` is Marquee's own (the web app's chip); Seerr has no such filter and keeps everything.
fn filter_sets(filter: &str) -> (Option<Vec<i64>>, Option<Vec<i64>>) {
    use super::media_status as m;
    let not_there = vec![m::UNKNOWN, m::PENDING, m::PROCESSING, m::PARTIALLY_AVAILABLE];
    match filter {
        "approved" => (Some(vec![status::APPROVED]), None),
        "processing" => (Some(vec![status::APPROVED]), Some(not_there)),
        "pending" => (Some(vec![status::PENDING]), None),
        "unavailable" => (Some(vec![status::PENDING, status::APPROVED]), Some(not_there)),
        "failed" => (Some(vec![status::FAILED]), None),
        "completed" => (Some(vec![status::COMPLETED]), None),
        "available" => (Some(vec![status::COMPLETED]), Some(vec![m::AVAILABLE])),
        "deleted" => (Some(vec![status::COMPLETED]), Some(vec![m::DELETED])),
        "declined" => (Some(vec![status::DECLINED]), None),
        _ => (None, None),
    }
}

/// The title status a request is judged by (Marquee has one library, so 4K requests use the same one).
fn title_status(db: &SqliteDb, r: &Request) -> i64 {
    media(db, r.media_id).ok().flatten().and_then(|m| availability(db, &m).ok()).map_or(super::media_status::UNKNOWN, |a| a.status)
}

/// Whether request `r` matches Seerr's `filter`.
pub fn matches(db: &SqliteDb, r: &Request, filter: &str) -> bool {
    let (statuses, titles) = filter_sets(filter);
    statuses.is_none_or(|s| s.contains(&r.status)) && titles.is_none_or(|t| t.contains(&title_status(db, r)))
}

/// The requests `s` may see, filtered and sorted as `q` asks (Seerr's `GET /request`): accounts without
/// MANAGE_REQUESTS or REQUEST_VIEW see only their own, and asking for somebody else's is 403.
fn requests_for(st: &AppState, s: &Seerr, q: &RequestQuery, only: Option<i64>) -> std::result::Result<Vec<Request>, Fail> {
    let filter = q.filter.clone().unwrap_or_else(|| "all".into());
    let kind = q.media_type.clone().unwrap_or_else(|| "all".into());
    let wanted = only.or(q.requested_by);
    let sees_all = s.0.has(&[perm::MANAGE_REQUESTS, perm::REQUEST_VIEW]);
    if !sees_all && wanted.is_some_and(|u| u != s.0.id()) {
        return Err(Fail(err(403, "You do not have permission to view this user's requests.")));
    }
    let who = if sees_all { wanted } else { Some(s.0.id()) };
    let mut list: Vec<Request> = all(&st.db)?
        .into_iter()
        .filter(|r| who.is_none_or(|u| r.requested_by == u))
        .filter(|r| kind != "movie" && kind != "tv" || r.kind == kind)
        .filter(|r| matches(&st.db, r, &filter))
        .collect();
    if q.sort.as_deref() == Some("modified") {
        list.sort_by_key(|r| (when(&r.updated_at), r.id));
    } else {
        list.sort_by_key(|r| r.id);
    }
    if q.sort_direction.as_deref() != Some("asc") {
        list.reverse();
    }
    Ok(list)
}

/// One request of a list, with Seerr's `profileName` (the quality profile's name, when it has one).
fn listed(st: &AppState, r: &Request, viewer: &Asker) -> kit_server_core::Result<Value> {
    let mut v = request_json(&st.db, r, Some(viewer))?;
    if let Some(p) = r.profile_id.and_then(|id| crate::quality::profile(&st.db, id).ok().flatten()) {
        v["profileName"] = p["name"].clone();
    }
    Ok(v)
}

/// One page of requests in Seerr's shape, with `serviceErrors` (none: Marquee is its own Radarr and Sonarr).
fn request_page(st: &AppState, s: &Seerr, list: Vec<Request>, q: &RequestQuery, default_take: usize) -> Answer {
    let take = q.take.filter(|t| *t > 0).unwrap_or(default_take).min(1000);
    let skip = q.skip.unwrap_or(0);
    let total = list.len();
    let results = list.iter().skip(skip).take(take).map(|r| listed(st, r, &s.0)).collect::<kit_server_core::Result<Vec<_>>>()?;
    ok(json!({
        "pageInfo": { "pages": total.div_ceil(take), "pageSize": take, "results": total, "page": skip.div_ceil(take) + 1 },
        "results": results,
        "serviceErrors": { "radarr": [], "sonarr": [] },
    }))
}

/// `GET /request`.
async fn list_requests(State(st): State<AppState>, s: Seerr, Query(q): Query<RequestQuery>) -> Answer {
    let list = requests_for(&st, &s, &q, None)?;
    request_page(&st, &s, list, &q, 10)
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

/// `GET /request/count`: every request, counted like Seerr (for everybody signed in).
async fn count(State(st): State<AppState>, _s: Seerr) -> Answer {
    let list: Vec<(Request, i64)> = all(&st.db)?.into_iter().map(|r| (title_status(&st.db, &r), r)).map(|(t, r)| (r, t)).collect();
    let n = |f: &dyn Fn(&Request, i64) -> bool| list.iter().filter(|(r, t)| f(r, *t)).count();
    let there = super::media_status::AVAILABLE;
    ok(json!({
        "total": list.len(),
        "movie": n(&|r, _| r.kind == "movie"),
        "tv": n(&|r, _| r.kind == "tv"),
        "pending": n(&|r, _| r.status == status::PENDING),
        "approved": n(&|r, _| r.status == status::APPROVED),
        "declined": n(&|r, _| r.status == status::DECLINED),
        "processing": n(&|r, t| r.status == status::APPROVED && t != there),
        "available": n(&|r, t| r.status == status::APPROVED && t == there),
        "completed": n(&|r, _| r.status == status::COMPLETED),
    }))
}

/// The request `id`: 404 "Request not found." (Seerr's words).
fn found(st: &AppState, id: i64) -> std::result::Result<Request, Fail> {
    Ok(find(&st.db, id)?.ok_or_else(|| err(404, "Request not found."))?)
}

/// `GET /request/{id}`: your own, or any with MANAGE_REQUESTS or REQUEST_VIEW.
async fn one_request(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let r = found(&st, id)?;
    if r.requested_by != s.0.id() && !s.0.has(&[perm::MANAGE_REQUESTS, perm::REQUEST_VIEW]) {
        return Err(Fail(err(403, "You do not have permission to view this request.")));
    }
    ok(request_json(&st.db, &r, Some(&s.0))?)
}

/// `PUT /request/{id}` body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestChange {
    /// `movie` or `tv`: which of Seerr's two edits this is
    pub media_type: Option<String>,
    /// the seasons (TV)
    pub seasons: Option<Value>,
    /// Seerr's Radarr/Sonarr server (kept as given; Marquee is both)
    pub server_id: Option<i64>,
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
    /// anime: `dub`, `dual` or `sub` (Marquee's own)
    pub anime_audio: Option<String>,
    /// anime: the subtitle language (Marquee's own)
    pub anime_subtitles: Option<String>,
}

/// `PUT /request/{id}`, ported from Seerr: a waiting request's profile, folder, tags, account and (TV) seasons.
/// The asker may change their own TV request, and their own movie request with REQUEST_ADVANCED; MANAGE_REQUESTS
/// may change any. Moving it to another account charges that account's limit in full; adding seasons charges
/// the seasons not counted yet.
async fn change_request(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<RequestChange>) -> Answer {
    let mut r = found(&st, id)?;
    let kind = b.media_type.clone().unwrap_or_default();
    let manager = s.0.has(&[perm::MANAGE_REQUESTS]);
    if (r.requested_by != s.0.id() || (kind != "tv" && !s.0.has(&[perm::REQUEST_ADVANCED]))) && !manager {
        return Err(Fail(err(403, "You do not have permission to modify this request.")));
    }
    if r.status != status::PENDING {
        return Err(Fail(err(409, "Only pending requests can be modified.")));
    }
    if (kind == "movie" || kind == "tv") && kind != r.kind {
        return Err(Fail(err(400, "A request can't change between movie and TV")));
    }
    let previous_owner = r.requested_by;
    let owner = match b.user_id {
        Some(u) if u != previous_owner && !s.0.has_all(&[perm::MANAGE_USERS, perm::MANAGE_REQUESTS]) => {
            return Err(Fail(err(403, "You do not have permission to modify the request user.")));
        }
        Some(u) => user(&st.db, u)?,
        None => user(&st.db, previous_owner)?,
    };
    let owner_changing = owner.i("id") != previous_owner;
    let (mq, tq) = super::quotas(&st.db, &owner)?;
    if kind == "movie" {
        if owner_changing && !r.ignore_quota && mq.restricted() {
            return Err(Fail(err(403, "Movie Quota exceeded.")));
        }
    } else if kind == "tv" {
        let requested = season_list(&b.seasons);
        if requested.is_empty() {
            return Err(Fail(err(500, "Missing seasons. If you want to cancel a series request, use the DELETE method.")));
        }
        let m = media(&st.db, r.media_id)?.ok_or_else(|| err(404, "Request not found."))?;
        let others: Vec<i64> = super::for_media(&st.db, m.id)?
            .into_iter()
            .filter(|o| o.is4k == r.is4k && o.id != r.id && !matches!(o.status, status::DECLINED | status::COMPLETED))
            .flat_map(|o| o.seasons)
            .collect();
        let current = r.seasons.clone();
        let covered: Vec<i64> = availability(&st.db, &m)?
            .seasons
            .iter()
            .filter(|(n, st)| *st != super::media_status::UNKNOWN && !current.contains(n))
            .map(|(n, _)| *n)
            .collect();
        let filtered: Vec<i64> = requested.into_iter().filter(|n| !others.contains(n)).collect();
        let kept: Vec<i64> = filtered.iter().copied().filter(|n| current.contains(n)).collect();
        let new: Vec<i64> = filtered.iter().copied().filter(|n| !current.contains(n) && !covered.contains(n)).collect();
        let resulting = (kept.len() + new.len()) as i64;
        if resulting == 0 {
            return Err(Fail(err(202, "No seasons available to request")));
        }
        if !r.ignore_quota {
            // only what the owner's quota already counted for this request is paid for
            let window =
                (tq.days > 0).then(|| (chrono::Utc::now() - chrono::Duration::days(tq.days)).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
            let counted = !owner_changing && window.is_none_or(|w| r.created_at > w);
            let prior = if counted { current.len() as i64 } else { 0 };
            if tq.limit > 0 && resulting - prior > tq.remaining().unwrap_or(0) {
                return Err(Fail(err(403, "Series Quota exceeded.")));
            }
        }
        let mut seasons: Vec<i64> = current.into_iter().filter(|n| kept.contains(n)).collect();
        seasons.extend(new);
        r.seasons = seasons;
        r.language_profile_id = b.language_profile_id;
    }
    if kind == "movie" || kind == "tv" {
        let _ = b.server_id;
        r.profile_id = b.profile_id;
        r.root_folder = b.root_folder.filter(|f| !f.trim().is_empty());
        r.tags = b.tags.unwrap_or_default();
        r.requested_by = owner.i("id");
    }
    if b.anime_audio.is_some() || b.anime_subtitles.is_some() {
        let audio = b.anime_audio.unwrap_or(r.anime_audio.clone());
        let subs = b.anime_subtitles.unwrap_or(r.anime_subtitles.clone()).to_lowercase();
        super::check_anime_choice(&audio, &subs)?;
        (r.anime_audio, r.anime_subtitles) = (audio, subs);
    }
    r.updated_at = stamp();
    save(&st.db, &r)?;
    ok(request_json(&st.db, &r, Some(&s.0))?)
}

/// `DELETE /request/{id}`: 204. MANAGE_REQUESTS any; the asker their own while it waits for approval (else 401,
/// as Seerr answers).
async fn delete_request(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let r = found(&st, id)?;
    if !s.0.has(&[perm::MANAGE_REQUESTS]) && (r.requested_by != s.0.id() || r.status != status::PENDING) {
        return Err(Fail(err(401, "You do not have permission to delete this request.")));
    }
    remove(&st.db, id)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /request/{id}/approve` or `/decline` (MANAGE_REQUESTS): only a waiting request.
async fn approve_or_decline(State(st): State<AppState>, s: Seerr, Path((id, action)): Path<(i64, String)>) -> Answer {
    s.needs(&[perm::MANAGE_REQUESTS])?;
    let r = found(&st, id)?;
    let yes = match action.as_str() {
        "approve" => true,
        "decline" => false,
        _ => return Err(Fail(err(400, "Status must be approve or decline."))),
    };
    if r.status != status::PENDING {
        return Err(Fail(err(409, "Only pending requests can be approved or declined.")));
    }
    let r = decide(&st, r, &s.0, yes).await?;
    ok(request_json(&st.db, &r, Some(&s.0))?)
}

/// `POST /request/{id}/retry` (MANAGE_REQUESTS): approve again a request that didn't work.
async fn retry(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_REQUESTS])?;
    let r = found(&st, id)?;
    if r.status != status::FAILED {
        return Err(Fail(err(409, "Only failed requests can be retried.")));
    }
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

/// `GET /media`, ported from Seerr's `routes/media.ts`: newest id first (`modified`: last changed first;
/// `mediaAdded`: newest in the library first), filtered by the title's status.
async fn list_media(State(st): State<AppState>, _s: Seerr, Query(q): Query<MediaQuery>) -> Answer {
    use super::media_status as ms;
    let wanted: Option<Vec<i64>> = match q.filter.as_deref().unwrap_or("all") {
        "available" => Some(vec![ms::AVAILABLE]),
        "partial" => Some(vec![ms::PARTIALLY_AVAILABLE]),
        "allavailable" => Some(vec![ms::AVAILABLE, ms::PARTIALLY_AVAILABLE]),
        "processing" => Some(vec![ms::PROCESSING]),
        "pending" => Some(vec![ms::PENDING]),
        _ => None,
    };
    let mut list = Vec::new();
    for m in all_media(&st.db)? {
        let a = availability(&st.db, &m)?;
        if wanted.as_ref().is_none_or(|w| w.contains(&a.status)) {
            list.push(m);
        }
    }
    match q.sort.as_deref() {
        Some("modified") => list.sort_by_key(|m| std::cmp::Reverse((when(&m.updated_at), m.id))),
        Some("mediaAdded") => list.sort_by_key(|m| std::cmp::Reverse((when(&m.created_at), m.id))),
        _ => list.sort_by_key(|m| std::cmp::Reverse(m.id)),
    }
    let items = list.iter().map(|m| media_json(&st.db, m, true)).collect::<kit_server_core::Result<Vec<_>>>()?;
    ok(paged(items, &Paging { take: q.take, skip: q.skip }, 20))
}

/// `DELETE /media/{id}` (MANAGE_REQUESTS): forget a title and its requests; 204 also when there is no such
/// title, like Seerr. The library keeps the movie or series.
async fn delete_media(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_REQUESTS])?;
    if media(&st.db, id)?.is_some() {
        remove_media(&st.db, id)?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /media/{id}/file` (MANAGE_REQUESTS): Seerr's "Remove from Radarr/Sonarr". The movie or series leaves
/// Marquee's library and the title is marked deleted (asking for it again starts over). Its files stay on disk
/// until the Radarr/Sonarr port follows `deleteFiles`.
async fn delete_media_file(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_REQUESTS])?;
    let m = media(&st.db, id)?.ok_or_else(|| err(404, "Media not found"))?;
    if let Some(lib) = availability(&st.db, &m)?.library_id {
        if m.kind == "movie" {
            crate::movies::remove(&st.db, lib)?;
        } else {
            crate::series::remove(&st.db, lib)?;
        }
    }
    st.db
        .execute("UPDATE request_media SET status_override=?, updated_at=? WHERE id=?", p![super::media_status::DELETED, stamp(), m.id])?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /media/{id}/watch_data` (admins): Seerr reads it from Tautulli, which Marquee doesn't connect to, so
/// it answers Seerr's "not configured".
async fn watch_data(s: Seerr, Path(_id): Path<i64>) -> Answer {
    s.needs(&[perm::ADMIN])?;
    Err(Fail(err(404, "Tautulli API not configured.")))
}

/// `POST /media/{id}/{status}` (MANAGE_REQUESTS): set a title's availability by hand. `unknown` goes back to
/// what the library says; a word Seerr doesn't know changes nothing.
async fn set_media_status(State(st): State<AppState>, s: Seerr, Path((id, word)): Path<(i64, String)>) -> Answer {
    s.needs(&[perm::MANAGE_REQUESTS])?;
    let m = media(&st.db, id)?.ok_or_else(|| err(404, "Media does not exist."))?;
    let value: Option<Option<i64>> = match word.as_str() {
        "available" => Some(Some(super::media_status::AVAILABLE)),
        "partial" if m.kind == "movie" => return Err(Fail(err(400, "Only series can be set to be partially available"))),
        "partial" => Some(Some(super::media_status::PARTIALLY_AVAILABLE)),
        "processing" => Some(Some(super::media_status::PROCESSING)),
        "pending" => Some(Some(super::media_status::PENDING)),
        "unknown" => Some(None),
        _ => None,
    };
    if let Some(v) = value {
        st.db.execute("UPDATE request_media SET status_override=?, updated_at=? WHERE id=?", p![v, stamp(), m.id])?;
    }
    let m = media(&st.db, id)?.ok_or_else(|| err(404, "Media does not exist."))?;
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
    let list = requests_for(&st, &s, &q, Some(id))?;
    request_page(&st, &s, list, &q, 20)
}

/// One quota in Seerr's shape.
fn quota_json(q: &super::Quota) -> Value {
    json!({
        "days": q.days, "limit": q.limit, "used": q.used, "remaining": q.remaining(),
        "restricted": q.restricted(),
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
