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
//! POST   /api/v1/user/{id}/settings/linked-accounts/jellyfin  link your own Jellyfin user (its name and password)
//! DELETE /api/v1/user/{id}/settings/linked-accounts/jellyfin  forget the link (the account itself or MANAGE_USERS)
//! POST   /api/v1/user/{id}/settings/linked-accounts/plex      link your own Plex account (a plex.tv token)
//! DELETE /api/v1/user/{id}/settings/linked-accounts/plex      forget the link (the account itself or MANAGE_USERS)
//! ```
//!
//! Linked accounts use Marquee's own account links (`media_servers::account_sync`), so a self-linked Jellyfin
//! user or Plex share is kept in step like one an admin linked.

use super::seerr::{limit, may_grant, perm, permissions, set_role_bits, Answer, Fail, Seerr};
use super::{anime_prefs, set_anime_prefs, user};
use crate::roles::Role;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
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
        .route("/user/{id}/settings/notifications", get(notifications).post(set_notifications))
        .route("/user/{id}/settings/linked-accounts/jellyfin", post(link_jellyfin).delete(unlink_jellyfin))
        .route("/user/{id}/settings/linked-accounts/plex", post(link_plex).delete(unlink_plex));
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

/// Seerr's `isOwnProfile`: only the account itself (403 otherwise).
fn own(s: &Seerr, id: i64) -> std::result::Result<(), Fail> {
    if s.0.id() != id {
        return Err(Fail(err(403, "You can only link accounts to your own profile.")));
    }
    Ok(())
}

/// What `POST /user/{id}/settings/linked-accounts/jellyfin` takes.
#[derive(Deserialize)]
pub struct JellyfinLink {
    /// the Jellyfin name
    #[serde(default)]
    pub username: String,
    /// the Jellyfin password
    #[serde(default)]
    pub password: String,
}

/// `POST /user/{id}/settings/linked-accounts/jellyfin`: signs in to Jellyfin with the name and password and links
/// that Jellyfin user to your own account; 204.
async fn link_jellyfin(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<JellyfinLink>) -> Answer {
    own(&s, id)?;
    if !crate::media_servers::external_login::jellyfin_on(&st) {
        return Err(Fail(err(400, "Jellyfin/Emby login is disabled")));
    }
    let Some(ju) = crate::media_servers::jellyfin::Jellyfin::authenticate(&st, b.username.trim(), &b.password).await? else {
        return Err(Fail(err(401, "That Jellyfin name or password is wrong.")));
    };
    let taken =
        st.db.one("SELECT user_id FROM media_accounts WHERE server='jellyfin' AND remote_id=? AND user_id!=?", p![ju.id.clone(), id])?;
    if taken.is_some() {
        return Err(Fail(err(422, "The specified account is already linked to a Seerr user")));
    }
    crate::media_servers::account_sync::link_remote(&st, id, "jellyfin", &ju.id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// What `POST /user/{id}/settings/linked-accounts/plex` takes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexLink {
    /// the plex.tv token from Plex's sign-in window
    #[serde(default)]
    pub auth_token: String,
}

/// `POST /user/{id}/settings/linked-accounts/plex`: links the plex.tv account behind the token to your own
/// account, through its share of the Plex server; 204. The account's mail address must match yours.
async fn link_plex(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<PlexLink>) -> Answer {
    own(&s, id)?;
    if !crate::media_servers::external_login::plex_on(&st) {
        return Err(Fail(err(400, "Plex login is disabled")));
    }
    let account = crate::media_servers::plex::Plex::person(&st, b.auth_token.trim()).account().await?;
    let taken =
        st.db.one("SELECT user_id FROM media_accounts WHERE server='plex' AND remote_id=? AND user_id!=?", p![account.id.clone(), id])?;
    if taken.is_some() {
        return Err(Fail(err(422, "This Plex account is already linked to a Seerr user")));
    }
    let me = user(&st.db, id)?;
    if !me.s("email").eq_ignore_ascii_case(account.email.trim()) || account.email.trim().is_empty() {
        return Err(Fail(err(422, "This Plex account is registered under a different email address.")));
    }
    let px = crate::media_servers::plex(&st)?;
    let machine = crate::media_servers::plex_machine(&st, &px).await?;
    let share = px
        .shares(&machine)
        .await?
        .into_iter()
        .find(|sh| sh.user_id == account.id)
        .ok_or_else(|| Fail(err(422, "This Plex account has no share of the Plex server yet. Ask an admin to share it.")))?;
    crate::media_servers::account_sync::link_remote(&st, id, "plex", &share.id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Seerr's checks before a link is forgotten: the owner keeps theirs, and the account needs its own mail
/// address and password to still sign in.
fn may_unlink(st: &AppState, s: &Seerr, id: i64) -> std::result::Result<(), Fail> {
    let u = own_or_manager(st, s, id)?;
    if Role::parse(&u.s("role")) == Role::Owner {
        return Err(Fail(err(400, "Cannot unlink media server accounts for the primary administrator.")));
    }
    if u.s("email").is_empty() || u.s("password_hash").is_empty() {
        return Err(Fail(err(400, "User does not have a local email or password set.")));
    }
    Ok(())
}

/// `DELETE /user/{id}/settings/linked-accounts/jellyfin`: 204; the Jellyfin user itself stays.
async fn unlink_jellyfin(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    may_unlink(&st, &s, id)?;
    crate::media_servers::account_sync::forget(&st.db, id, "jellyfin")?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /user/{id}/settings/linked-accounts/plex`: 204; the Plex share itself stays.
async fn unlink_plex(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    may_unlink(&st, &s, id)?;
    crate::media_servers::account_sync::forget(&st.db, id, "plex")?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
