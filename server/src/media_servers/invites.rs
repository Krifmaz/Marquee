//! Invite links and the public sign-up page (what jfa-go did). An admin makes an invite: one-time or for
//! several people, until a date or for ever, with the role, the servers (Jellyfin and/or Plex) and libraries
//! the new account gets, and how long the account lasts. The person opens `<public address>/#/invite/<code>`,
//! picks a name and password, and gets a Marquee account, a Jellyfin user with the same password and a Plex
//! share at once, already signed in.
//!
//! ```text
//! GET    /api/invites                      → [invite] (admins)
//! POST   /api/invites                      {label?, max_uses?, expires_in_days?, account_days?, role?,
//!                                           jellyfin?: {libraries} | null, plex?: {libraries} | null, send_to?}
//!                                          → the invite, with its link (admins)
//! PATCH  /api/invites/{code}               {label?, disabled?} → the invite (admins)
//! DELETE /api/invites/{code}               → {ok} (admins; accounts made with it stay)
//! POST   /api/invites/{code}/send          {email} → {ok}: the link by mail (admins)
//! GET    /api/public/invites/{code}        → {app_name, label, jellyfin, plex, jellyfin_url, expires_at, uses_left}
//!                                            (anyone; 404 or 410 with the reason when it can't be used)
//! POST   /api/public/invites/{code}        {username, password, email?, plex_email?} → the new account, signed
//!                                            in (the session cookie), with `media_errors` (anyone; 10 tries an hour per address)
//! ```
//!
//! `max_uses` 0 means no limit; `expires_in_days` 0 means the link never ends; `account_days` 0 means the
//! accounts never end.

use super::account_sync::{Access, Choice, Libraries};
use crate::roles::{Me, Role};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The invite tables.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS invites (
            code TEXT PRIMARY KEY,
            label TEXT NOT NULL DEFAULT '',
            max_uses INTEGER NOT NULL DEFAULT 1,
            used INTEGER NOT NULL DEFAULT 0,
            expires_at REAL NOT NULL DEFAULT 0,
            account_days INTEGER NOT NULL DEFAULT 0,
            role TEXT NOT NULL DEFAULT 'requester',
            jellyfin INTEGER NOT NULL DEFAULT 0,
            jellyfin_libraries TEXT NOT NULL DEFAULT '*',
            plex INTEGER NOT NULL DEFAULT 0,
            plex_libraries TEXT NOT NULL DEFAULT '*',
            sent_to TEXT NOT NULL DEFAULT '',
            disabled INTEGER NOT NULL DEFAULT 0,
            created_by TEXT NOT NULL DEFAULT '',
            created_at REAL
        );
        CREATE TABLE IF NOT EXISTS invite_uses (
            code TEXT NOT NULL,
            user_id INTEGER NOT NULL,
            username TEXT NOT NULL DEFAULT '',
            at REAL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The invite tables can't be made: {}", e.detail))
}

/// The invite routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/invites", get(list).post(create))
        .route("/api/invites/{code}", patch(change).delete(remove))
        .route("/api/invites/{code}/send", post(send))
        .route("/api/public/invites/{code}", get(look).post(sign_up))
}

/// The link of invite `code`: `<public address>/#/invite/<code>` (or just the path when no address is set).
pub fn link(st: &AppState, code: &str) -> String {
    format!("{}/#/invite/{code}", crate::mail::public_url(st))
}

/// Why invite `r` can't be used now, `None` when it can.
pub fn unusable(r: &Row) -> Option<&'static str> {
    if r.b("disabled") {
        Some("This invite was turned off. Ask the person who sent it for a new one.")
    } else if r.f("expires_at") > 0.0 && r.f("expires_at") <= crate::auth::now() {
        Some("This invite has ended. Ask the person who sent it for a new one.")
    } else if r.i("max_uses") > 0 && r.i("used") >= r.i("max_uses") {
        Some("This invite was already used. Ask the person who sent it for a new one.")
    } else {
        None
    }
}

/// An invite as admins see it.
fn public(st: &AppState, r: &Row, uses: &[Row]) -> Value {
    let libs = |s: &str| match Libraries::parse(s) {
        None => json!("*"),
        Some(l) => json!(l),
    };
    json!({
        "code": r.s("code"),
        "link": link(st, &r.s("code")),
        "label": r.s("label"),
        "max_uses": r.i("max_uses"),
        "used": r.i("used"),
        "expires_at": if r.f("expires_at") > 0.0 { json!(r.f("expires_at")) } else { Value::Null },
        "account_days": r.i("account_days"),
        "role": r.s("role"),
        "jellyfin": if r.b("jellyfin") { json!({ "libraries": libs(&r.s("jellyfin_libraries")) }) } else { Value::Null },
        "plex": if r.b("plex") { json!({ "libraries": libs(&r.s("plex_libraries")) }) } else { Value::Null },
        "sent_to": r.s("sent_to"),
        "disabled": r.b("disabled"),
        "usable": unusable(r).is_none(),
        "why_not": unusable(r),
        "created_by": r.s("created_by"),
        "created_at": r.f("created_at"),
        "uses": uses.iter().filter(|u| u.s("code") == r.s("code")).map(|u| json!({ "user_id": u.i("user_id"), "username": u.s("username"), "at": u.f("at") })).collect::<Vec<_>>(),
    })
}

/// The invite `code`, 404 when there is none.
fn find(db: &SqliteDb, code: &str) -> Result<Row> {
    db.one("SELECT * FROM invites WHERE code=?", p![code])?
        .ok_or_else(|| err(404, "There is no such invite. Check the link, or ask for a new one."))
}

/// `GET /api/invites`: every invite, newest first.
async fn list(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    let rows = st.db.query("SELECT * FROM invites ORDER BY created_at DESC", p![])?;
    let uses = st.db.query("SELECT * FROM invite_uses ORDER BY at", p![])?;
    Ok(Json(json!(rows.iter().map(|r| public(&st, r, &uses)).collect::<Vec<_>>())))
}

/// The libraries of one server on a new invite.
#[derive(Deserialize, Default)]
pub struct InviteServer {
    /// `"*"` or library ids (default every library)
    pub libraries: Option<Libraries>,
}

/// `POST /api/invites` body.
#[derive(Deserialize, Default)]
pub struct NewInvite {
    /// a note for admins ("Family", "Sam's friends")
    #[serde(default)]
    pub label: String,
    /// how many accounts it can make (default 1; 0: no limit)
    pub max_uses: Option<i64>,
    /// days until the link ends (default 7; 0: never)
    pub expires_in_days: Option<i64>,
    /// days each new account lasts (default 0: for ever)
    pub account_days: Option<i64>,
    /// `requester` or `viewer` (default: the "New accounts" role)
    pub role: Option<String>,
    /// a Jellyfin user for each new account, with these libraries
    pub jellyfin: Option<InviteServer>,
    /// a Plex share for each new account, with these libraries
    pub plex: Option<InviteServer>,
    /// mail the link to this address at once
    pub send_to: Option<String>,
}

/// `POST /api/invites`.
async fn create(State(st): State<AppState>, me: Me, Json(b): Json<NewInvite>) -> Result<Json<Value>> {
    me.admin()?;
    let max_uses = b.max_uses.unwrap_or(1);
    let days = b.expires_in_days.unwrap_or(7);
    let account_days = b.account_days.unwrap_or(0);
    if !(0..=10000).contains(&max_uses) {
        return Err(err(400, "The number of uses must be 0 (no limit) to 10000"));
    }
    if !(0..=3650).contains(&days) || !(0..=3650).contains(&account_days) {
        return Err(err(400, "A number of days must be 0 to 3650"));
    }
    let role = match b.role.as_deref() {
        None | Some("") => crate::users::default_role(&st),
        Some(r) => Role::assignable(r).filter(|r| !r.is_admin()).ok_or_else(|| err(400, "An invite's role must be requester or viewer"))?,
    };
    if b.jellyfin.is_some() {
        super::jellyfin(&st)?;
    }
    if b.plex.is_some() {
        super::plex(&st)?;
    }
    let code = crate::auth::random_token(9);
    let libs = |s: &Option<InviteServer>| s.as_ref().and_then(|s| s.libraries.clone()).map(|l| l.stored()).unwrap_or_else(|| "*".into());
    let expires = if days == 0 { 0.0 } else { crate::auth::now() + days as f64 * 86400.0 };
    st.db.execute(
        "INSERT INTO invites (code, label, max_uses, expires_at, account_days, role, jellyfin, jellyfin_libraries, plex, plex_libraries, created_by, created_at)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        p![
            code.clone(),
            b.label.trim(),
            max_uses,
            expires,
            account_days,
            role.as_str(),
            b.jellyfin.is_some() as i64,
            libs(&b.jellyfin),
            b.plex.is_some() as i64,
            libs(&b.plex),
            me.account.username.clone(),
            crate::auth::now()
        ],
    )?;
    if let Some(to) = b.send_to.as_deref().filter(|t| !t.trim().is_empty()) {
        mail_invite(&st, &code, to).await.map_err(|e| err(400, format!("The invite was made, but the mail wasn't sent: {e}")))?;
    }
    Ok(Json(public(&st, &find(&st.db, &code)?, &[])))
}

/// `PATCH /api/invites/{code}` body.
#[derive(Deserialize)]
pub struct InviteChange {
    /// a new note
    pub label: Option<String>,
    /// turn it off or on
    pub disabled: Option<bool>,
}

/// `PATCH /api/invites/{code}`.
async fn change(State(st): State<AppState>, me: Me, Path(code): Path<String>, Json(b): Json<InviteChange>) -> Result<Json<Value>> {
    me.admin()?;
    find(&st.db, &code)?;
    if let Some(l) = b.label {
        st.db.execute("UPDATE invites SET label=? WHERE code=?", p![l.trim(), code.clone()])?;
    }
    if let Some(d) = b.disabled {
        st.db.execute("UPDATE invites SET disabled=? WHERE code=?", p![d as i64, code.clone()])?;
    }
    let uses = st.db.query("SELECT * FROM invite_uses WHERE code=?", p![code.clone()])?;
    Ok(Json(public(&st, &find(&st.db, &code)?, &uses)))
}

/// `DELETE /api/invites/{code}`.
async fn remove(State(st): State<AppState>, me: Me, Path(code): Path<String>) -> Result<Json<Value>> {
    me.admin()?;
    find(&st.db, &code)?;
    st.db.execute("DELETE FROM invites WHERE code=?", p![code])?;
    Ok(Json(json!({ "ok": true })))
}

/// Mails the link of invite `code` to `to`.
async fn mail_invite(st: &AppState, code: &str, to: &str) -> std::result::Result<(), String> {
    let app = st.app_name();
    if crate::mail::public_url(st).is_empty() {
        return Err("Set Marquee's address for people in Settings → Media servers first, so the link works.".into());
    }
    let text = format!(
        "Hello,\n\nYou're invited to {app}. Open this link to make your account:\n\n{}\n\nYou pick your own name and password.\n",
        link(st, code)
    );
    crate::mail::send(st, crate::mail::Mail { to: to.trim().to_string(), subject: format!("You're invited to {app}"), text }).await?;
    st.db.execute("UPDATE invites SET sent_to=? WHERE code=?", p![to.trim(), code]).map_err(|e| e.detail)?;
    Ok(())
}

/// `POST /api/invites/{code}/send` body.
#[derive(Deserialize)]
pub struct SendBody {
    /// the address
    pub email: String,
}

/// `POST /api/invites/{code}/send`.
async fn send(State(st): State<AppState>, me: Me, Path(code): Path<String>, Json(b): Json<SendBody>) -> Result<Json<Value>> {
    me.admin()?;
    find(&st.db, &code)?;
    mail_invite(&st, &code, &b.email).await.map_err(|e| err(400, e))?;
    Ok(Json(json!({ "ok": true })))
}

/// The invite `code` when it can be used; 404 or 410 with the reason otherwise.
fn usable(db: &SqliteDb, code: &str) -> Result<Row> {
    let r = find(db, code)?;
    match unusable(&r) {
        Some(why) => Err(err(410, why)),
        None => Ok(r),
    }
}

/// `GET /api/public/invites/{code}`: what the sign-up page needs.
async fn look(State(st): State<AppState>, Path(code): Path<String>) -> Result<Json<Value>> {
    let r = usable(&st.db, &code)?;
    let jf_url = {
        let p = st.settings.str("jellyfin_public_url");
        if p.trim().is_empty() {
            st.settings.str("jellyfin_url")
        } else {
            p
        }
    };
    Ok(Json(json!({
        "app_name": st.app_name(),
        "label": r.s("label"),
        "jellyfin": r.b("jellyfin"),
        "plex": r.b("plex"),
        "jellyfin_url": if r.b("jellyfin") { jf_url.trim().to_string() } else { String::new() },
        "expires_at": if r.f("expires_at") > 0.0 { json!(r.f("expires_at")) } else { Value::Null },
        "account_days": r.i("account_days"),
        "uses_left": if r.i("max_uses") == 0 { Value::Null } else { json!(r.i("max_uses") - r.i("used")) },
        "mail": crate::mail::enabled(&st),
    })))
}

/// `POST /api/public/invites/{code}` body.
#[derive(Deserialize)]
pub struct SignUp {
    /// the sign-in name (also the Jellyfin user's)
    pub username: String,
    /// the password (also Jellyfin's)
    pub password: String,
    /// a mail address, for password reset and reminders
    #[serde(default)]
    pub email: String,
    /// the plex.tv account's mail address, when it differs from `email`
    #[serde(default)]
    pub plex_email: String,
}

/// `POST /api/public/invites/{code}`: make the account, its Jellyfin user and Plex share, and sign in.
async fn sign_up(
    State(st): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(b): Json<SignUp>,
) -> Result<(CookieJar, Json<Value>)> {
    let key = format!("signup:{}", crate::auth::client_ip(&headers));
    if st.limits.limited(&key, 10, 3600.0, true) {
        return Err(err(429, "Too many tries from here. Wait an hour and try again."));
    }
    let r = usable(&st.db, &code)?;
    let username = b.username.trim().to_string();
    crate::auth::validate_credentials(&username, &b.password)?;
    let email = b.email.trim().to_string();
    let plex_email = if b.plex_email.trim().is_empty() { email.clone() } else { b.plex_email.trim().to_string() };
    if r.b("plex") && plex_email.is_empty() {
        return Err(err(400, "Plex needs the mail address of your Plex account"));
    }
    if !email.is_empty() && !email.contains('@') {
        return Err(err(400, format!("{email} isn't a mail address")));
    }
    if r.b("jellyfin") {
        let jf = super::jellyfin(&st)?;
        if jf.users().await?.iter().any(|u| u.name.eq_ignore_ascii_case(&username)) {
            return Err(err(409, "That name is already taken. Pick another one."));
        }
    }
    // the use counts before the account is made, so two people can't both take the last use
    let taken =
        st.db.execute("UPDATE invites SET used=used+1 WHERE code=? AND disabled=0 AND (max_uses=0 OR used<max_uses)", p![code.clone()])?;
    if taken == 0 {
        return Err(err(410, "This invite was already used. Ask the person who sent it for a new one."));
    }
    let role = Role::parse(&r.s("role"));
    let role = if role.is_admin() { Role::Requester } else { role };
    let id = match crate::users::create_account(&st, &username, &b.password, &email, role, Default::default()) {
        Ok(id) => id,
        Err(e) => {
            st.db.execute("UPDATE invites SET used=used-1 WHERE code=?", p![code.clone()])?;
            return Err(e);
        }
    };
    st.db.execute(
        "INSERT INTO invite_uses (code, user_id, username, at) VALUES (?,?,?,?)",
        p![code.clone(), id, username.clone(), crate::auth::now()],
    )?;
    if r.i("account_days") > 0 {
        super::expiry::set_expiry(&st.db, id, Some(crate::auth::now() + r.i("account_days") as f64 * 86400.0))?;
    }
    let access = |on: bool, libs: String, email: Option<String>| {
        Some(on.then(|| Access { active: Some(true), libraries: Some(stored_libraries(&libs)), email }))
    };
    let choice = Choice {
        jellyfin: if r.b("jellyfin") { access(true, r.s("jellyfin_libraries"), None) } else { None },
        plex: if r.b("plex") { access(true, r.s("plex_libraries"), Some(plex_email)) } else { None },
    };
    let errors = super::account_sync::apply(&st, id, &choice, Some(&b.password), false).await?;
    tracing::info!("'{username}' signed up with the invite {}", r.s("label"));
    let user = st.db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| err(500, "The new account can't be read back"))?;
    let (jar, Json(mut v)) = crate::auth::start_session(&st, &headers, jar, &user, false).await?;
    v["media_errors"] = json!(errors);
    Ok((jar, Json(v)))
}

/// The [`Libraries`] kept as `s` in the invites table.
fn stored_libraries(s: &str) -> Libraries {
    match Libraries::parse(s) {
        None => Libraries::All("*".into()),
        Some(l) => Libraries::Some(l),
    }
}
