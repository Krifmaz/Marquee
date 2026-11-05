//! Password reset by mail: a person who forgot their password asks for a link, opens
//! `<public address>/#/reset/<token>` and picks a new one. Marquee and Jellyfin both take it. The link works once,
//! for an hour; the answer never says whether an account exists.
//!
//! ```text
//! POST /api/public/password/forgot   {login} (name or mail address) → {ok} (anyone; 5 an hour per address)
//! POST /api/public/password/reset    {token, password} → {ok}: the new password, every session signed out (anyone)
//! ```

use crate::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// How long a reset link works: an hour.
pub const LINK_SECS: f64 = 3600.0;

/// The reset link table: the SHA-256 of each token, never the token.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS password_resets (
            token TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            expires_at REAL NOT NULL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The password reset table can't be made: {}", e.detail))
}

/// The password reset routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/public/password/forgot", post(forgot)).route("/api/public/password/reset", post(reset))
}

/// `POST /api/public/password/forgot` body.
#[derive(Deserialize)]
pub struct Forgot {
    /// the account's name or mail address
    pub login: String,
}

/// `POST /api/public/password/forgot`: mail a reset link to the account's address, if there is one.
async fn forgot(State(st): State<AppState>, headers: HeaderMap, Json(b): Json<Forgot>) -> Result<Json<Value>> {
    if !crate::mail::enabled(&st) {
        return Err(err(400, "This server can't send mail, so ask an admin to set a new password for you."));
    }
    let key = format!("forgot:{}", crate::auth::client_ip(&headers));
    if st.limits.limited(&key, 5, 3600.0, true) {
        return Err(err(429, "Too many tries from here. Wait an hour and try again."));
    }
    let login = b.login.trim();
    let user = st.db.one(
        "SELECT * FROM users WHERE (username=? COLLATE NOCASE OR (email!='' AND email=? COLLATE NOCASE)) AND disabled=0",
        p![login, login],
    )?;
    if let Some(u) = user.filter(|u| !u.s("email").trim().is_empty()) {
        let token = crate::auth::random_token(32);
        st.db.execute("DELETE FROM password_resets WHERE user_id=? OR expires_at<?", p![u.i("id"), crate::auth::now()])?;
        st.db.execute(
            "INSERT INTO password_resets (token, user_id, expires_at) VALUES (?,?,?)",
            p![crate::auth::app_token_key(&token), u.i("id"), crate::auth::now() + LINK_SECS],
        )?;
        let app = st.app_name();
        let text = format!(
            "Hello {},\n\nSomeone asked for a new {app} password for your account. If it was you, open this link within an hour and pick a new one:\n\n{}/#/reset/{token}\n\nYour Jellyfin password changes with it. If it wasn't you, ignore this mail: nothing changes.\n",
            u.s("username"),
            crate::mail::public_url(&st)
        );
        crate::mail::send_later(&st, crate::mail::Mail { to: u.s("email"), subject: format!("Your new {app} password"), text });
    }
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/public/password/reset` body.
#[derive(Deserialize)]
pub struct Reset {
    /// the token from the link
    pub token: String,
    /// the new password
    pub password: String,
}

/// `POST /api/public/password/reset`: set the new password here and on Jellyfin, sign out everywhere.
async fn reset(State(st): State<AppState>, headers: HeaderMap, Json(b): Json<Reset>) -> Result<Json<Value>> {
    let key = format!("reset:{}", crate::auth::client_ip(&headers));
    if st.limits.limited(&key, 20, 3600.0, true) {
        return Err(err(429, "Too many tries from here. Wait an hour and try again."));
    }
    let gone = || err(410, "This link has ended or was already used. Ask for a new one on the sign-in page.");
    let row = st.db.one("SELECT * FROM password_resets WHERE token=?", p![crate::auth::app_token_key(b.token.trim())])?.ok_or_else(gone)?;
    if row.f("expires_at") < crate::auth::now() {
        st.db.execute("DELETE FROM password_resets WHERE token=?", p![row.s("token")])?;
        return Err(gone());
    }
    let id = row.i("user_id");
    let u = st.db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(gone)?;
    crate::auth::validate_credentials(&u.s("username"), &b.password)?;
    let pw = b.password.clone();
    let hash = tokio::task::spawn_blocking(move || crate::auth::hash_password(&pw)).await.unwrap_or_default();
    st.db.execute("UPDATE users SET password_hash=? WHERE id=?", p![hash, id])?;
    st.db.execute("DELETE FROM password_resets WHERE user_id=?", p![id])?;
    st.db.execute("DELETE FROM sessions WHERE user_id=?", p![id])?;
    let errors = super::account_sync::password_changed(&st, id, &b.password).await;
    tracing::info!("'{}' set a new password with a reset link", u.s("username"));
    Ok(Json(json!({ "ok": true, "media_errors": errors })))
}
