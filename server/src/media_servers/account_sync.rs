//! Each account's Jellyfin user and Plex share, kept matched to Marquee. Marquee is the one place accounts
//! are made: when an account is made, changed, turned off or removed here, its Jellyfin user and Plex share
//! follow.
//!
//! | In Marquee | Jellyfin | Plex |
//! | --- | --- | --- |
//! | account made with access | a user with the same name and password | the chosen libraries shared with the person's Plex mail address |
//! | password set or reset | the same password | – (Plex has its own sign-in) |
//! | libraries changed | the user's libraries | the share's libraries |
//! | admin or owner role | every library | every library |
//! | turned off, access off, or expired | the user is turned off | the share is removed |
//! | turned on again | the user is turned on | shared again |
//! | removed | the user is removed | the share is removed |
//!
//! When Marquee doesn't know the password (access given to an account that already exists), the Jellyfin user
//! gets a random one, and takes the person's Marquee password the next time they sign in to Marquee.
//!
//! ```text
//! GET    /api/users/{id}/media                  → {jellyfin, plex, expires_at}: the account's access (admins, or the account itself)
//! PUT    /api/users/{id}/media                  {jellyfin?: {active?, libraries?} | null, plex?: {active?, libraries?, email?} | null}
//!                                               → the access after syncing; null turns access off (admins)
//! POST   /api/users/{id}/media/sync             → the access after syncing again (admins)
//! POST   /api/users/{id}/media/{server}/link    {remote_id} → link an existing Jellyfin user or Plex share (admins)
//! DELETE /api/users/{id}/media/{server}         → {ok}: forget the link; the Jellyfin user or Plex share stays (admins)
//! ```
//!
//! `libraries` is `"*"` (every library) or a list of library ids from `GET /api/media-servers/{server}/libraries`.

use super::plex_machine;
use crate::roles::{Me, Role};
use crate::AppState;
use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The table of links between accounts and media server users.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS media_accounts (
            user_id INTEGER NOT NULL,
            server TEXT NOT NULL,
            remote_id TEXT NOT NULL DEFAULT '',
            remote_name TEXT NOT NULL DEFAULT '',
            email TEXT NOT NULL DEFAULT '',
            share_id TEXT NOT NULL DEFAULT '',
            libraries TEXT NOT NULL DEFAULT '*',
            active INTEGER NOT NULL DEFAULT 1,
            password_pending INTEGER NOT NULL DEFAULT 0,
            sync_error TEXT NOT NULL DEFAULT '',
            synced_at REAL,
            PRIMARY KEY (user_id, server)
        );",
    )
    .map_err(|e| anyhow::anyhow!("The media account table can't be made: {}", e.detail))
}

/// The account routes of this module.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users/{id}/media", get(show).put(change))
        .route("/api/users/{id}/media/sync", post(sync_again))
        .route("/api/users/{id}/media/{server}/link", post(link))
        .route("/api/users/{id}/media/{server}", delete(unlink))
}

/// Which libraries a person sees: every one, or these ids.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Libraries {
    /// `"*"`: every library
    All(String),
    /// these library ids
    Some(Vec<String>),
}

impl Libraries {
    /// How the table keeps it: `*`, or a JSON list.
    pub fn stored(&self) -> String {
        match self {
            Libraries::All(_) => "*".into(),
            Libraries::Some(l) => json!(l).to_string(),
        }
    }

    /// From the table: `None` for every library. (`keep`, a Plex share linked as it was, counts as every
    /// library here; [`sync`] leaves such a share's libraries alone.)
    pub fn parse(s: &str) -> Option<Vec<String>> {
        if s == "*" || s.is_empty() || s == KEEP {
            None
        } else {
            Some(serde_json::from_str(s).unwrap_or_default())
        }
    }
}

/// The libraries of a Plex share that was linked as it was: Marquee doesn't change them until an admin picks
/// libraries.
pub const KEEP: &str = "keep";

/// The access an account should have on one server.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Access {
    /// whether it has access (default on)
    pub active: Option<bool>,
    /// its libraries (default: unchanged, or every library for new access)
    pub libraries: Option<Libraries>,
    /// Plex: the person's plex.tv mail address (default: the account's own)
    pub email: Option<String>,
}

/// The access an account should have on each server; `None` leaves a server as it is, `Some(None)` turns
/// access off.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Choice {
    /// Jellyfin
    #[serde(default, deserialize_with = "some")]
    pub jellyfin: Option<Option<Access>>,
    /// Plex
    #[serde(default, deserialize_with = "some")]
    pub plex: Option<Option<Access>>,
}

/// Tells a field that is `null` (`Some(None)`) from one that is missing (`None`).
fn some<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Option<Access>>, D::Error> {
    Option::<Access>::deserialize(d).map(Some)
}

/// The link row of account `user_id` on `server`.
fn row(db: &SqliteDb, user_id: i64, server: &str) -> Result<Option<Row>> {
    db.one("SELECT * FROM media_accounts WHERE user_id=? AND server=?", p![user_id, server])
}

/// Saves `choice` for account `user_id` (without syncing).
pub fn save_choice(db: &SqliteDb, user_id: i64, choice: &Choice) -> Result<()> {
    for (server, c) in [("jellyfin", &choice.jellyfin), ("plex", &choice.plex)] {
        let Some(c) = c else { continue };
        let existing = row(db, user_id, server)?;
        match (c, existing) {
            (None, None) => {}
            (None, Some(_)) => {
                db.execute("UPDATE media_accounts SET active=0 WHERE user_id=? AND server=?", p![user_id, server])?;
            }
            (Some(a), existing) => {
                if existing.is_none() {
                    db.execute("INSERT INTO media_accounts (user_id, server) VALUES (?,?)", p![user_id, server])?;
                }
                db.execute(
                    "UPDATE media_accounts SET active=? WHERE user_id=? AND server=?",
                    p![a.active.unwrap_or(true) as i64, user_id, server],
                )?;
                if let Some(l) = &a.libraries {
                    db.execute("UPDATE media_accounts SET libraries=? WHERE user_id=? AND server=?", p![l.stored(), user_id, server])?;
                }
                if let Some(e) = &a.email {
                    db.execute("UPDATE media_accounts SET email=? WHERE user_id=? AND server=?", p![e.trim(), user_id, server])?;
                }
            }
        }
    }
    Ok(())
}

/// The account `id`, 404 when there is none.
fn user(db: &SqliteDb, id: i64) -> Result<Row> {
    db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| err(404, "There is no such account"))
}

/// Records the outcome of a sync of one row.
fn record(db: &SqliteDb, user_id: i64, server: &str, error: &str) -> Result<()> {
    db.execute(
        "UPDATE media_accounts SET sync_error=?, synced_at=? WHERE user_id=? AND server=?",
        p![error, crate::auth::now(), user_id, server],
    )
    .map(|_| ())
}

/// Makes each media server match account `user_id`: its Jellyfin user and Plex share made, changed, turned
/// off or on. `password` is the account's password when it was just set. With `adopt`, a Jellyfin user of the
/// same name that no account is linked to becomes this account's; without, it is an error (a stranger's
/// sign-up must not take over someone's Jellyfin user). The errors in words, also kept on each link.
pub async fn sync(st: &AppState, user_id: i64, password: Option<&str>, adopt: bool) -> Vec<String> {
    let mut errors = Vec::new();
    for server in super::SERVERS {
        let r = match server {
            "jellyfin" => sync_jellyfin(st, user_id, password, adopt).await,
            _ => sync_plex(st, user_id).await,
        };
        let msg = match &r {
            Ok(()) => String::new(),
            Err(e) => e.detail.clone(),
        };
        if row(&st.db, user_id, server).ok().flatten().is_some() {
            let _ = record(&st.db, user_id, server, &msg);
        }
        if !msg.is_empty() {
            tracing::warn!("Syncing account {user_id} with {server}: {msg}");
            errors.push(msg);
        }
    }
    errors
}

/// The libraries account `u` should see on a server, from its link row `r`: `None` for every library.
fn libraries_for(u: &Row, r: &Row) -> Option<Vec<String>> {
    if Role::parse(&u.s("role")).is_admin() {
        None
    } else {
        Libraries::parse(&r.s("libraries"))
    }
}

/// [`sync`] for Jellyfin.
async fn sync_jellyfin(st: &AppState, user_id: i64, password: Option<&str>, adopt: bool) -> Result<()> {
    let Some(r) = row(&st.db, user_id, "jellyfin")? else { return Ok(()) };
    let u = user(&st.db, user_id)?;
    let jf = super::jellyfin(st)?;
    let off = u.b("disabled") || !r.b("active");
    let mut remote = r.s("remote_id");
    let mut password_done = false;
    if remote.is_empty() {
        if off {
            return Ok(());
        }
        let name = u.s("username");
        let existing = jf.users().await?.into_iter().find(|j| j.name.eq_ignore_ascii_case(&name));
        match existing {
            Some(j) => {
                let taken = st.db.one(
                    "SELECT user_id FROM media_accounts WHERE server='jellyfin' AND remote_id=? AND user_id!=?",
                    p![j.id.clone(), user_id],
                )?;
                if !adopt || taken.is_some() || j.admin {
                    return Err(err(
                        409,
                        format!(
                            "Jellyfin already has a user called {}. Pick another name, or an admin links that user in Media access.",
                            j.name
                        ),
                    ));
                }
                remote = j.id;
                st.db.execute("UPDATE media_accounts SET password_pending=1 WHERE user_id=? AND server='jellyfin'", p![user_id])?;
            }
            None => {
                let (pw, pending) = match password {
                    Some(p) => (p.to_string(), false),
                    None => (crate::auth::random_token(24), true),
                };
                remote = jf.create_user(&name, &pw).await?;
                password_done = !pending;
                st.db.execute(
                    "UPDATE media_accounts SET password_pending=? WHERE user_id=? AND server='jellyfin'",
                    p![pending as i64, user_id],
                )?;
            }
        }
        st.db.execute(
            "UPDATE media_accounts SET remote_id=?, remote_name=? WHERE user_id=? AND server='jellyfin'",
            p![remote.clone(), name, user_id],
        )?;
    }
    jf.set_policy(&remote, off, libraries_for(&u, &r).as_deref()).await?;
    if let (Some(pw), false) = (password, password_done) {
        jf.set_password(&remote, pw).await?;
        st.db.execute("UPDATE media_accounts SET password_pending=0 WHERE user_id=? AND server='jellyfin'", p![user_id])?;
    }
    Ok(())
}

/// [`sync`] for Plex.
async fn sync_plex(st: &AppState, user_id: i64) -> Result<()> {
    let Some(r) = row(&st.db, user_id, "plex")? else { return Ok(()) };
    let u = user(&st.db, user_id)?;
    let email = if r.s("email").is_empty() { u.s("email") } else { r.s("email") };
    let want = !u.b("disabled") && r.b("active");
    let share = r.s("share_id");
    if !want && share.is_empty() {
        return Ok(());
    }
    let px = super::plex(st)?;
    let machine = plex_machine(st, &px).await?;
    let libs = libraries_for(&u, &r);
    if !want {
        px.remove_share(&machine, &share).await?;
        st.db.execute("UPDATE media_accounts SET share_id='' WHERE user_id=? AND server='plex'", p![user_id])?;
        return Ok(());
    }
    if !share.is_empty() {
        if r.s("libraries") == KEEP && !Role::parse(&u.s("role")).is_admin() {
            return Ok(());
        }
        return px.update_share(&machine, &share, libs.as_deref()).await;
    }
    if email.trim().is_empty() {
        return Err(err(400, "Plex needs the person's Plex mail address. Add it to the account."));
    }
    let existing = px.shares(&machine).await?.into_iter().find(|s| s.email.eq_ignore_ascii_case(email.trim()));
    let s = match existing {
        Some(s) => {
            px.update_share(&machine, &s.id, libs.as_deref()).await?;
            s
        }
        None => px.invite(&machine, email.trim(), libs.as_deref()).await?,
    };
    st.db.execute(
        "UPDATE media_accounts SET share_id=?, remote_id=?, remote_name=?, email=? WHERE user_id=? AND server='plex'",
        p![s.id, s.user_id, if s.username.is_empty() { email.trim().to_string() } else { s.username }, email.trim(), user_id],
    )?;
    Ok(())
}

/// Saves `choice` for account `user_id` and syncs; the errors in words.
pub async fn apply(st: &AppState, user_id: i64, choice: &Choice, password: Option<&str>, adopt: bool) -> Result<Vec<String>> {
    save_choice(&st.db, user_id, choice)?;
    Ok(sync(st, user_id, password, adopt).await)
}

/// The account's password was set to `password` (by the person, an admin or a reset link): Jellyfin takes it.
pub async fn password_changed(st: &AppState, user_id: i64, password: &str) -> Vec<String> {
    if row(&st.db, user_id, "jellyfin").ok().flatten().is_none() {
        return vec![];
    }
    sync(st, user_id, Some(password), false).await
}

/// The person signed in to Marquee with `password`: a Jellyfin user still waiting for it takes it.
pub async fn signed_in(st: &AppState, user_id: i64, password: &str) {
    let pending = st
        .db
        .one("SELECT 1 FROM media_accounts WHERE user_id=? AND server='jellyfin' AND password_pending=1 AND remote_id!=''", p![user_id])
        .ok()
        .flatten()
        .is_some();
    if pending {
        let _ = sync(st, user_id, Some(password), false).await;
    }
}

/// Removes account `user_id`'s Jellyfin user and Plex share and forgets the links (before the account itself
/// is removed); the errors in words.
pub async fn remove_everywhere(st: &AppState, user_id: i64) -> Vec<String> {
    let mut errors = Vec::new();
    if let Ok(Some(r)) = row(&st.db, user_id, "jellyfin") {
        if !r.s("remote_id").is_empty() {
            match super::jellyfin(st) {
                Ok(jf) => {
                    if let Err(e) = jf.delete_user(&r.s("remote_id")).await {
                        errors.push(e.detail)
                    }
                }
                Err(e) => errors.push(e.detail),
            }
        }
    }
    if let Ok(Some(r)) = row(&st.db, user_id, "plex") {
        if !r.s("share_id").is_empty() {
            let res = async {
                let px = super::plex(st)?;
                let machine = plex_machine(st, &px).await?;
                px.remove_share(&machine, &r.s("share_id")).await
            };
            if let Err(e) = res.await {
                errors.push(e.detail);
            }
        }
    }
    let _ = st.db.execute("DELETE FROM media_accounts WHERE user_id=?", p![user_id]);
    errors
}

/// The account's access as clients see it.
pub fn access(db: &SqliteDb, user_id: i64) -> Result<Value> {
    let u = user(db, user_id)?;
    let mut out = json!({ "expires_at": super::expiry::expires_at(&u) });
    for server in super::SERVERS {
        out[server] = match row(db, user_id, server)? {
            None => Value::Null,
            Some(r) => json!({
                "active": r.b("active"),
                "linked": !r.s("remote_id").is_empty() || !r.s("share_id").is_empty(),
                "remote_id": r.s("remote_id"),
                "remote_name": r.s("remote_name"),
                "email": r.s("email"),
                "libraries": match Libraries::parse(&r.s("libraries")) { _ if r.s("libraries") == KEEP => json!(KEEP), None => json!("*"), Some(l) => json!(l) },
                "password_pending": r.b("password_pending"),
                "error": r.s("sync_error"),
                "synced_at": r.f("synced_at"),
            }),
        };
    }
    Ok(out)
}

/// `GET /api/users/{id}/media`.
async fn show(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    if me.account.id != id {
        me.admin()?;
    }
    Ok(Json(access(&st.db, id)?))
}

/// `PUT /api/users/{id}/media`.
async fn change(State(st): State<AppState>, me: Me, Path(id): Path<i64>, Json(b): Json<Choice>) -> Result<Json<Value>> {
    me.admin()?;
    user(&st.db, id)?;
    let errors = apply(&st, id, &b, None, true).await?;
    let mut v = access(&st.db, id)?;
    v["errors"] = json!(errors);
    Ok(Json(v))
}

/// `POST /api/users/{id}/media/sync`.
async fn sync_again(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    user(&st.db, id)?;
    let errors = sync(&st, id, None, true).await;
    let mut v = access(&st.db, id)?;
    v["errors"] = json!(errors);
    Ok(Json(v))
}

/// `POST /api/users/{id}/media/{server}/link` body.
#[derive(Deserialize)]
pub struct LinkBody {
    /// the Jellyfin user's id, or the Plex share's id
    pub remote_id: String,
}

/// `POST /api/users/{id}/media/{server}/link`: link an existing Jellyfin user or Plex share to account `id`.
async fn link(State(st): State<AppState>, me: Me, Path((id, server)): Path<(i64, String)>, Json(b): Json<LinkBody>) -> Result<Json<Value>> {
    me.admin()?;
    super::known(&server)?;
    user(&st.db, id)?;
    let remote = b.remote_id.trim().to_string();
    if remote.is_empty() {
        return Err(err(400, "Pick the user to link"));
    }
    let col = if server == "plex" { "share_id" } else { "remote_id" };
    let taken = st.db.one(
        &format!("SELECT user_id FROM media_accounts WHERE server=? AND {col}=? AND user_id!=?"),
        p![server.clone(), remote.clone(), id],
    )?;
    if taken.is_some() {
        return Err(err(409, "That user is already linked to another account"));
    }
    let (name, email, plex_id, libraries) = if server == "jellyfin" {
        let u = super::jellyfin(&st)?
            .users()
            .await?
            .into_iter()
            .find(|u| u.id == remote)
            .ok_or_else(|| err(404, "Jellyfin has no such user"))?;
        let libs = match u.libraries {
            None => "*".to_string(),
            Some(l) => json!(l).to_string(),
        };
        (u.name, String::new(), String::new(), libs)
    } else {
        let px = super::plex(&st)?;
        let machine = plex_machine(&st, &px).await?;
        let s = px.shares(&machine).await?.into_iter().find(|s| s.id == remote).ok_or_else(|| err(404, "Plex has no such share"))?;
        (if s.username.is_empty() { s.email.clone() } else { s.username }, s.email, s.user_id, KEEP.to_string())
    };
    if row(&st.db, id, &server)?.is_none() {
        st.db.execute("INSERT INTO media_accounts (user_id, server) VALUES (?,?)", p![id, server.clone()])?;
    }
    if server == "jellyfin" {
        st.db.execute(
            "UPDATE media_accounts SET remote_id=?, remote_name=?, libraries=?, active=1, password_pending=1, sync_error='' WHERE user_id=? AND server='jellyfin'",
            p![remote, name, libraries, id],
        )?;
    } else {
        st.db.execute(
            "UPDATE media_accounts SET share_id=?, remote_id=?, remote_name=?, email=?, libraries=?, active=1, sync_error='' WHERE user_id=? AND server='plex'",
            p![remote, plex_id, name, email, libraries, id],
        )?;
    }
    Ok(Json(access(&st.db, id)?))
}

/// `DELETE /api/users/{id}/media/{server}`: forget the link only.
async fn unlink(State(st): State<AppState>, me: Me, Path((id, server)): Path<(i64, String)>) -> Result<Json<Value>> {
    me.admin()?;
    super::known(&server)?;
    st.db.execute("DELETE FROM media_accounts WHERE user_id=? AND server=?", p![id, server])?;
    Ok(Json(json!({ "ok": true })))
}
