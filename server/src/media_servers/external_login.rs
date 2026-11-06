//! Signing in with a Plex or Jellyfin account instead of a Marquee password.
//!
//! - **Jellyfin:** the name and password are checked by Jellyfin. A Jellyfin user linked to an account signs in
//!   as it (and the account's Marquee password becomes that password, so there is one password).
//! - **Plex:** the plex.tv PIN sign-in. The page opens plex.tv, the person signs in there, and the page asks
//!   `finish` until plex.tv says it is done. A Plex account linked to an account (by its share, or the same mail
//!   address) signs in as it.
//!
//! A person who isn't linked yet gets an account only when `signup_media_creates` is on and they can already
//! open the server (a Jellyfin user that isn't an administrator, or a Plex account the server is shared with).
//! Their access stays as it is on the server until an admin changes it.
//!
//! ```text
//! GET  /api/public/sign-in-options  → {jellyfin, plex, reset}: which buttons the sign-in page shows (anyone)
//! POST /api/auth/jellyfin           {username, password, app?} → like /api/auth/login (anyone; 10 tries in 5 minutes)
//! POST /api/auth/plex/start         → {id, code, url}: open `url`, then call finish (anyone)
//! POST /api/auth/plex/finish        {id, app?} → like /api/auth/login, or 202 {pending: true} while the person is
//!                                     still signing in on plex.tv (anyone)
//! ```

use super::account_sync::KEEP;
use super::jellyfin::Jellyfin;
use super::plex::Plex;
use crate::auth::AppDevice;
use crate::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use kit_server_core::{err, p, Db, Result, Row, RowExt};
use serde::Deserialize;
use serde_json::{json, Value};

/// The sign-in routes of this module.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/public/sign-in-options", get(options))
        .route("/api/auth/jellyfin", post(jellyfin))
        .route("/api/auth/plex/start", post(plex_start))
        .route("/api/auth/plex/finish", post(plex_finish))
}

/// Whether Jellyfin sign-in is on: the setting, and Jellyfin's address.
pub(crate) fn jellyfin_on(st: &AppState) -> bool {
    st.settings.bool("jellyfin_sign_in") && !st.settings.str("jellyfin_url").trim().is_empty()
}

/// Whether Plex sign-in is on: the setting, and the Plex server.
pub(crate) fn plex_on(st: &AppState) -> bool {
    st.settings.bool("plex_sign_in") && Plex::from_settings(st).is_some()
}

/// `GET /api/public/sign-in-options`.
async fn options(State(st): State<AppState>) -> Json<Value> {
    Json(json!({ "jellyfin": jellyfin_on(&st), "plex": plex_on(&st), "reset": crate::mail::enabled(&st) }))
}

/// `POST /api/auth/jellyfin` body.
#[derive(Deserialize)]
pub struct JellyfinLogin {
    /// the Jellyfin name
    pub username: String,
    /// the Jellyfin password
    pub password: String,
    /// the phone app's device, for an app token instead of a cookie
    pub app: Option<AppDevice>,
}

/// A name Marquee takes (3–32 letters, digits, `_`, `-`, `.`) made from `name`, and not taken yet.
pub(super) fn free_name(st: &AppState, name: &str) -> Result<String> {
    let mut base: String = name.chars().filter(|c| c.is_alphanumeric() || "_-.".contains(*c)).take(28).collect();
    while base.chars().count() < 3 {
        base.push('_');
    }
    for n in 0..1000 {
        let cand = if n == 0 { base.clone() } else { format!("{base}{n}") };
        if st.db.one("SELECT id FROM users WHERE username=? COLLATE NOCASE", p![cand.clone()])?.is_none() {
            return Ok(cand);
        }
    }
    Err(err(409, "No free name was found for this account"))
}

/// The account `id`, refusing one that is turned off.
fn active(st: &AppState, id: i64) -> Result<Row> {
    let u = st.db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| err(401, "This account no longer exists"))?;
    if u.b("disabled") {
        return Err(err(403, "This account is disabled"));
    }
    Ok(u)
}

/// `POST /api/auth/jellyfin`.
async fn jellyfin(
    State(st): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(b): Json<JellyfinLogin>,
) -> Result<(CookieJar, Json<Value>)> {
    if !jellyfin_on(&st) {
        return Err(err(400, "Signing in with Jellyfin is turned off on this server"));
    }
    let key = format!("login:{}", crate::auth::client_ip(&headers));
    if st.limits.limited(&key, 10, 300.0, false) {
        return Err(err(429, "Too many attempts – wait a few minutes"));
    }
    let Some(ju) = Jellyfin::authenticate(&st, b.username.trim(), &b.password).await? else {
        st.limits.record(&key);
        return Err(err(401, "Jellyfin says the name or password is wrong"));
    };
    let linked = st.db.one("SELECT user_id FROM media_accounts WHERE server='jellyfin' AND remote_id=?", p![ju.id.clone()])?;
    let user = match linked {
        Some(l) => {
            let u = active(&st, l.i("user_id"))?;
            // one password: Marquee's becomes the one Jellyfin just took
            let pw = b.password.clone();
            if !b.password.is_empty() && !crate::auth::verify_password(&b.password, &u.s("password_hash")) {
                let hash = tokio::task::spawn_blocking(move || crate::auth::hash_password(&pw)).await.unwrap_or_default();
                st.db.execute("UPDATE users SET password_hash=? WHERE id=?", p![hash, u.i("id")])?;
            }
            st.db.execute("UPDATE media_accounts SET password_pending=0 WHERE user_id=? AND server='jellyfin'", p![u.i("id")])?;
            u
        }
        None => {
            if !st.settings.bool("signup_media_creates") || ju.admin || ju.disabled {
                return Err(err(403, "This Jellyfin user isn't linked to an account here. Ask an admin to link it, or for an invite."));
            }
            let name = free_name(&st, &ju.name)?;
            let pw = if b.password.chars().count() >= 8 { b.password.clone() } else { crate::auth::random_token(24) };
            let id = crate::users::create_account(&st, &name, &pw, "", crate::users::default_role(&st), Default::default())?;
            let libs = match &ju.libraries {
                None => "*".to_string(),
                Some(l) => json!(l).to_string(),
            };
            st.db.execute(
                "INSERT INTO media_accounts (user_id, server, remote_id, remote_name, libraries, synced_at) VALUES (?,'jellyfin',?,?,?,?)",
                p![id, ju.id.clone(), ju.name.clone(), libs, crate::auth::now()],
            )?;
            tracing::info!("The Jellyfin user '{}' signed in for the first time and got the account '{name}'", ju.name);
            active(&st, id)?
        }
    };
    crate::auth::start_session(&st, &headers, jar, &user, b.app.is_some()).await
}

/// `POST /api/auth/plex/start`: a new plex.tv PIN and the address to sign in at.
async fn plex_start(State(st): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    if !plex_on(&st) {
        return Err(err(400, "Signing in with Plex is turned off on this server"));
    }
    let key = format!("plex-pin:{}", crate::auth::client_ip(&headers));
    if st.limits.limited(&key, 20, 600.0, true) {
        return Err(err(429, "Too many attempts – wait a few minutes"));
    }
    let (id, code) = Plex::person(&st, "").new_pin().await?;
    Ok(Json(json!({ "id": id, "code": code, "url": super::plex::auth_url(&st, &code) })))
}

/// `POST /api/auth/plex/finish` body.
#[derive(Deserialize)]
pub struct PlexFinish {
    /// the PIN's id from start
    pub id: String,
    /// the phone app's device, for an app token instead of a cookie
    pub app: Option<AppDevice>,
}

/// `POST /api/auth/plex/finish`.
async fn plex_finish(State(st): State<AppState>, headers: HeaderMap, jar: CookieJar, Json(b): Json<PlexFinish>) -> Result<Response> {
    if !plex_on(&st) {
        return Err(err(400, "Signing in with Plex is turned off on this server"));
    }
    let Some(token) = Plex::person(&st, "").pin_token(&b.id).await? else {
        return Ok((StatusCode::ACCEPTED, Json(json!({ "pending": true }))).into_response());
    };
    let person = Plex::person(&st, &token);
    let acct = person.account().await?;
    let linked = st.db.one(
        "SELECT user_id FROM media_accounts WHERE server='plex' AND ((remote_id!='' AND remote_id=?) OR (email!='' AND email=? COLLATE NOCASE))",
        p![acct.id.clone(), acct.email.clone()],
    )?;
    let user = match linked {
        Some(l) => {
            st.db.execute(
                "UPDATE media_accounts SET remote_id=?, remote_name=? WHERE user_id=? AND server='plex'",
                p![acct.id.clone(), acct.username.clone(), l.i("user_id")],
            )?;
            active(&st, l.i("user_id"))?
        }
        None => {
            let server = super::plex(&st)?;
            let machine = super::plex_machine(&st, &server).await?;
            if !st.settings.bool("signup_media_creates") || !person.can_open(&machine).await? {
                return Err(err(403, "This Plex account isn't linked to an account here. Ask an admin to link it, or for an invite."));
            }
            let share =
                server.shares(&machine).await?.into_iter().find(|s| s.user_id == acct.id || s.email.eq_ignore_ascii_case(&acct.email));
            let Some(share) = share else {
                return Err(err(403, "This Plex account isn't linked to an account here. Ask an admin to link it, or for an invite."));
            };
            let name = free_name(&st, &acct.username)?;
            let id = crate::users::create_account(
                &st,
                &name,
                &crate::auth::random_token(24),
                &acct.email,
                crate::users::default_role(&st),
                Default::default(),
            )?;
            st.db.execute(
                "INSERT INTO media_accounts (user_id, server, remote_id, remote_name, email, share_id, libraries, synced_at) VALUES (?,'plex',?,?,?,?,?,?)",
                p![id, acct.id.clone(), acct.username.clone(), acct.email.clone(), share.id, KEEP, crate::auth::now()],
            )?;
            tracing::info!("The Plex account '{}' signed in for the first time and got the account '{name}'", acct.username);
            active(&st, id)?
        }
    };
    let (jar, body) = crate::auth::start_session(&st, &headers, jar, &user, b.app.is_some()).await?;
    Ok((jar, body).into_response())
}
