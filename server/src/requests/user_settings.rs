//! An account's own settings, ported from Seerr 3.5.1's `routes/user/usersettings.ts`: the general page (name,
//! mail address, display language and regions, request limits), the password, and the permissions. Marquee adds
//! the default anime language choice to the general page.
//!
//! ```text
//! GET  /api/v1/user/{id}/settings/main          the general settings (the account itself or MANAGE_USERS)
//! POST /api/v1/user/{id}/settings/main          change them; limits only for somebody else without MANAGE_USERS
//! GET  /api/v1/user/{id}/settings/password      whether the account has a password
//! POST /api/v1/user/{id}/settings/password      change it (the current one is needed for your own)
//! GET  /api/v1/user/{id}/settings/permissions   Seerr's permission bits (MANAGE_USERS)
//! POST /api/v1/user/{id}/settings/permissions   set them (MANAGE_USERS; never the owner's or your own)
//! GET  /api/v1/user/{id}/settings/notifications where the account's own news goes, and which news
//! POST /api/v1/user/{id}/settings/notifications change that
//! ```
//!
//! Linked accounts come in their own port.

use super::seerr::{limit, may_grant, perm, permissions, set_role_bits, Answer, Fail, Seerr};
use super::{anime_prefs, set_anime_prefs, user};
use crate::roles::Role;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The per-account settings table (Seerr's `UserSettings`, the parts the general page keeps).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_user_settings (
            user_id INTEGER PRIMARY KEY,
            locale TEXT,
            discover_region TEXT,
            streaming_region TEXT,
            original_language TEXT,
            watchlist_sync_movies INTEGER,
            watchlist_sync_tv INTEGER
        );
        CREATE TABLE IF NOT EXISTS request_user_notify (
            user_id INTEGER PRIMARY KEY,
            data TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .map_err(|e| anyhow::anyhow!("The account settings table can't be made: {}", e.detail))
}

/// The routes.
pub fn router() -> Router<AppState> {
    let v1 = Router::new()
        .route("/user/{id}/settings/main", get(main).post(set_main))
        .route("/user/{id}/settings/password", get(password).post(set_password))
        .route("/user/{id}/settings/permissions", get(permissions_of).post(set_permissions))
        .route("/user/{id}/settings/notifications", get(notifications).post(set_notifications));
    Router::new().nest("/api/v1", v1)
}

/// Seerr's `isOwnProfileOrAdmin`: the account itself or MANAGE_USERS, then the account (404 "User not found.").
fn own_or_manager(st: &AppState, s: &Seerr, id: i64) -> std::result::Result<Row, Fail> {
    if s.0.id() != id && !s.0.has(&[perm::MANAGE_USERS]) {
        return Err(Fail(err(403, "You do not have permission to view this user's settings.")));
    }
    st.db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| Fail(err(404, "User not found.")))
}

/// A stored text setting, or `null`.
fn text(r: &Option<Row>, k: &str) -> Value {
    r.as_ref().and_then(|r| r.get(k)).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null)
}

/// A stored yes/no setting, or `null`.
fn flag(r: &Option<Row>, k: &str) -> Value {
    r.as_ref().and_then(|r| r.get(k)).and_then(Value::as_i64).map(|v| json!(v != 0)).unwrap_or(Value::Null)
}

/// The general page in Seerr's shape (`UserSettingsGeneralResponse`), with Marquee's anime choice.
fn main_json(db: &SqliteDb, u: &Row) -> kit_server_core::Result<Value> {
    let id = u.i("id");
    let set = db.one("SELECT * FROM request_user_settings WHERE user_id=?", p![id])?;
    let (audio, subs) = anime_prefs(db, id)?;
    Ok(json!({
        "username": u.s("username"),
        "email": u.s("email"),
        "locale": text(&set, "locale"),
        "discoverRegion": text(&set, "discover_region"),
        "streamingRegion": text(&set, "streaming_region"),
        "originalLanguage": text(&set, "original_language"),
        "movieQuotaLimit": limit(u.i("movie_quota_limit")),
        "movieQuotaDays": u.i("movie_quota_days"),
        "tvQuotaLimit": limit(u.i("tv_quota_limit")),
        "tvQuotaDays": u.i("tv_quota_days"),
        "globalMovieQuotaDays": Value::Null,
        "globalMovieQuotaLimit": Value::Null,
        "globalTvQuotaDays": Value::Null,
        "globalTvQuotaLimit": Value::Null,
        "watchlistSyncMovies": flag(&set, "watchlist_sync_movies"),
        "watchlistSyncTv": flag(&set, "watchlist_sync_tv"),
        "animeAudio": audio,
        "animeSubtitles": subs,
    }))
}

/// `GET /user/{id}/settings/main`.
async fn main(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let u = own_or_manager(&st, &s, id)?;
    Ok(Json(main_json(&st.db, &u)?).into_response())
}

/// What `POST /user/{id}/settings/main` takes. Fields left out stay as they are.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MainSettings {
    /// the sign-in name
    pub username: Option<String>,
    /// the mail address
    pub email: Option<String>,
    /// the display language
    pub locale: Option<String>,
    /// the region discover shows
    pub discover_region: Option<String>,
    /// the region whose streaming services are shown
    pub streaming_region: Option<String>,
    /// the original languages discover shows (`|`-separated)
    pub original_language: Option<String>,
    /// add Plex watchlist movies as requests
    pub watchlist_sync_movies: Option<bool>,
    /// add Plex watchlist series as requests
    pub watchlist_sync_tv: Option<bool>,
    /// movies per period, null or 0 for no limit
    pub movie_quota_limit: Option<Value>,
    /// the movie period in days
    pub movie_quota_days: Option<i64>,
    /// seasons per period, null or 0 for no limit
    pub tv_quota_limit: Option<Value>,
    /// the TV period in days
    pub tv_quota_days: Option<i64>,
    /// the default anime audio (Marquee's own)
    pub anime_audio: Option<String>,
    /// the default anime subtitle language (Marquee's own)
    pub anime_subtitles: Option<String>,
}

/// `POST /user/{id}/settings/main`: like Seerr, limits change only when somebody else changes them for an account
/// without MANAGE_USERS; on your own page they are left as they are.
async fn set_main(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<MainSettings>) -> Answer {
    let u = own_or_manager(&st, &s, id)?;
    if Role::parse(&u.s("role")) == Role::Owner && s.0.role != Role::Owner {
        return Err(Fail(err(403, "You do not have permission to modify this user's settings.")));
    }
    if let Some(name) = b.username.as_deref().map(str::trim).filter(|n| !n.is_empty() && *n != u.s("username")) {
        crate::auth::validate_username(name)?;
        if st.db.one("SELECT id FROM users WHERE username=? AND id<>?", p![name, id])?.is_some() {
            return Err(Fail(err(400, "That name is already used by another account.")));
        }
        st.db.execute("UPDATE users SET username=? WHERE id=?", p![name, id])?;
    }
    if let Some(mail) = b.email.as_deref().map(str::trim).filter(|m| *m != u.s("email")) {
        if !mail.is_empty() && st.db.one("SELECT id FROM users WHERE email=? AND id<>?", p![mail, id])?.is_some() {
            return Err(Fail(err(400, "That email is already used by another account.")));
        }
        st.db.execute("UPDATE users SET email=? WHERE id=?", p![(!mail.is_empty()).then_some(mail), id])?;
    }
    let target_manages = super::seerr::has_any(&u, &[perm::MANAGE_USERS]);
    if !target_manages && s.0.id() != id {
        set_limits(&st.db, id, &b)?;
    }
    st.db.execute("INSERT INTO request_user_settings (user_id) VALUES (?) ON CONFLICT(user_id) DO NOTHING", p![id])?;
    let texts = [
        ("locale", &b.locale),
        ("discover_region", &b.discover_region),
        ("streaming_region", &b.streaming_region),
        ("original_language", &b.original_language),
    ];
    for (col, v) in texts {
        if let Some(v) = v {
            // the column names come from the fixed list above, never from the request
            st.db.execute(&format!("UPDATE request_user_settings SET {col}=? WHERE user_id=?"), p![v, id])?;
        }
    }
    for (col, v) in [("watchlist_sync_movies", b.watchlist_sync_movies), ("watchlist_sync_tv", b.watchlist_sync_tv)] {
        if let Some(v) = v {
            st.db.execute(&format!("UPDATE request_user_settings SET {col}=? WHERE user_id=?"), p![v as i64, id])?;
        }
    }
    if b.anime_audio.is_some() || b.anime_subtitles.is_some() {
        let (da, ds) = anime_prefs(&st.db, id)?;
        set_anime_prefs(&st.db, id, &b.anime_audio.unwrap_or(da), &b.anime_subtitles.unwrap_or(ds))?;
    }
    Ok(Json(main_json(&st.db, &user(&st.db, id)?)?).into_response())
}

/// Saves the request limits in `b` that were given; 400 for a limit below 0 or a period outside 1 to 365 days.
fn set_limits(db: &SqliteDb, id: i64, b: &MainSettings) -> std::result::Result<(), Fail> {
    for (col, v) in [("movie_quota_limit", &b.movie_quota_limit), ("tv_quota_limit", &b.tv_quota_limit)] {
        if let Some(v) = v {
            let n = v.as_i64().unwrap_or(0);
            if n < 0 {
                return Err(Fail(err(400, "A request limit can't be below 0 (0 means no limit)")));
            }
            // the column names come from the fixed list above, never from the request
            db.execute(&format!("UPDATE users SET {col}=? WHERE id=?"), p![n, id])?;
        }
    }
    for (col, v) in [("movie_quota_days", b.movie_quota_days), ("tv_quota_days", b.tv_quota_days)] {
        if let Some(d) = v {
            if !(1..=365).contains(&d) {
                return Err(Fail(err(400, "A request limit's period must be 1 to 365 days")));
            }
            db.execute(&format!("UPDATE users SET {col}=? WHERE id=?"), p![d, id])?;
        }
    }
    Ok(())
}

/// `GET /user/{id}/settings/password`.
async fn password(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    let u = own_or_manager(&st, &s, id)?;
    Ok(Json(json!({ "hasPassword": !u.s("password_hash").is_empty() })).into_response())
}

/// What `POST /user/{id}/settings/password` takes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordChange {
    /// the password now (needed when you change your own)
    pub current_password: Option<String>,
    /// the new password, 8 characters or more
    #[serde(default)]
    pub new_password: String,
}

/// `POST /user/{id}/settings/password`: 204. Only the owner changes the owner's or another admin's password.
async fn set_password(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<PasswordChange>) -> Answer {
    let u = own_or_manager(&st, &s, id)?;
    if b.new_password.chars().count() < 8 {
        return Err(Fail(err(400, "Password must be at least 8 characters.")));
    }
    let role = Role::parse(&u.s("role"));
    let me = s.0.role == Role::Owner;
    if (role == Role::Owner && !me) || (role.is_admin() && id != s.0.id() && !me) {
        return Err(Fail(err(403, "You do not have permission to modify this user's password.")));
    }
    let overriding = s.0.has(&[perm::MANAGE_USERS]) && id != s.0.id();
    let hash = u.s("password_hash");
    if !overriding && !hash.is_empty() {
        let current = b.current_password.clone().unwrap_or_default();
        let ok = tokio::task::spawn_blocking(move || crate::auth::verify_password(&current, &hash)).await.unwrap_or(false);
        if !ok {
            return Err(Fail(err(403, "Current password is invalid.")));
        }
    }
    let pw = b.new_password.clone();
    let new_hash = tokio::task::spawn_blocking(move || crate::auth::hash_password(&pw)).await.unwrap_or_default();
    st.db.execute("UPDATE users SET password_hash=? WHERE id=?", p![new_hash, id])?;
    // the linked Jellyfin user takes the new password too
    for e in crate::media_servers::account_sync::password_changed(&st, id, &b.new_password).await {
        tracing::warn!("account settings: the new password didn't reach Jellyfin: {e}");
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /user/{id}/settings/permissions` (MANAGE_USERS).
async fn permissions_of(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_USERS])?;
    let u = st.db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| Fail(err(404, "User not found.")))?;
    Ok(Json(json!({ "permissions": permissions(&u) })).into_response())
}

/// What `POST /user/{id}/settings/permissions` takes.
#[derive(Deserialize)]
pub struct PermissionsBody {
    /// Seerr's permission bits
    pub permissions: i64,
}

/// `POST /user/{id}/settings/permissions`: Seerr's bits become Marquee's role and auto-approve (MANAGE_USERS;
/// never the owner's or your own).
async fn set_permissions(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<PermissionsBody>) -> Answer {
    s.needs(&[perm::MANAGE_USERS])?;
    let u = st.db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| Fail(err(404, "User not found.")))?;
    if Role::parse(&u.s("role")) == Role::Owner || id == s.0.id() {
        return Err(Fail(err(403, "You do not have permission to modify this user")));
    }
    may_grant(&s, b.permissions)?;
    set_role_bits(&st.db, id, b.permissions)?;
    Ok(Json(json!({ "permissions": permissions(&user(&st.db, id)?) })).into_response())
}

/// The per-account notification fields Seerr keeps (`UserSettings`), by their names in Seerr's answer.
const NOTIFY_FIELDS: [&str; 9] = [
    "pgpKey",
    "pushbulletAccessToken",
    "pushoverApplicationToken",
    "pushoverUserKey",
    "pushoverSound",
    "telegramChatId",
    "telegramMessageThreadId",
    "telegramSendSilently",
    "notificationTypes",
];

/// The account's stored notification settings (`{}` when none).
fn stored_notify(db: &SqliteDb, id: i64) -> kit_server_core::Result<Value> {
    Ok(db
        .one("SELECT data FROM request_user_notify WHERE user_id=?", p![id])?
        .and_then(|r| serde_json::from_str(&r.s("data")).ok())
        .unwrap_or_else(|| json!({})))
}

/// The account's own fields, as Seerr answers a save.
fn own_notify(v: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for k in NOTIFY_FIELDS {
        out.insert(k.into(), v.get(k).cloned().unwrap_or(Value::Null));
    }
    out.insert("discordIds".into(), v.get("discordIds").cloned().unwrap_or_else(|| json!([])));
    if out["notificationTypes"].is_null() {
        out.insert("notificationTypes".into(), json!({}));
    }
    Value::Object(out)
}

/// `GET /user/{id}/settings/notifications` (`UserSettingsNotificationsResponse`): which ways of sending are on
/// for the whole server, and the account's own fields. Marquee sends mail; Discord goes to one channel without
/// mentions, and Telegram and web push aren't there yet, so those answer off.
async fn notifications(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    own_or_manager(&st, &s, id)?;
    let mut v = own_notify(&stored_notify(&st.db, id)?);
    if let Some(o) = v.as_object_mut() {
        o.insert("emailEnabled".into(), json!(crate::mail::enabled(&st)));
        o.insert("discordEnabled".into(), json!(false));
        o.insert("discordEnabledTypes".into(), json!(0));
        o.insert("telegramEnabled".into(), json!(false));
        o.insert("telegramBotUsername".into(), Value::Null);
        o.insert("webPushEnabled".into(), json!(false));
    }
    Ok(Json(v).into_response())
}

/// `POST /user/{id}/settings/notifications`: Seerr's fields are kept as sent, empty Discord ids are dropped and
/// the news types are merged into the ones kept; the owner's only by the owner.
async fn set_notifications(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<Value>) -> Answer {
    let u = own_or_manager(&st, &s, id)?;
    if Role::parse(&u.s("role")) == Role::Owner && s.0.role != Role::Owner {
        return Err(Fail(err(403, "You do not have permission to modify this user's settings.")));
    }
    let mut kept = stored_notify(&st.db, id)?;
    let Some(k) = kept.as_object_mut() else { return Err(Fail(err(500, "The saved notification settings can't be read"))) };
    for f in NOTIFY_FIELDS.iter().filter(|f| **f != "notificationTypes") {
        k.insert((*f).into(), b.get(*f).cloned().unwrap_or(Value::Null));
    }
    let ids: Vec<Value> = b["discordIds"].as_array().into_iter().flatten().filter(|i| i.as_str() != Some("")).cloned().collect();
    k.insert("discordIds".into(), json!(ids));
    let mut types = k.get("notificationTypes").and_then(Value::as_object).cloned().unwrap_or_default();
    for (t, v) in b["notificationTypes"].as_object().into_iter().flatten() {
        types.insert(t.clone(), v.clone());
    }
    k.insert("notificationTypes".into(), Value::Object(types));
    st.db.execute(
        "INSERT INTO request_user_notify (user_id, data) VALUES (?,?) ON CONFLICT(user_id) DO UPDATE SET data=excluded.data",
        p![id, kept.to_string()],
    )?;
    Ok(Json(own_notify(&kept)).into_response())
}
