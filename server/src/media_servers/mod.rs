//! Plex and Jellyfin, and the accounts, invites, sign-up and password reset that replace jfa-go (the media
//! servers track, docs/TRACKS.md).
//!
//! | Part | What |
//! | --- | --- |
//! | this file | the connection to each server: status, libraries, scans after an import, what each server already has ("available") |
//! | [`jellyfin`], [`plex`] | the two servers' APIs |
//! | [`account_sync`] | each account's Jellyfin user and Plex share, kept matched to Marquee: made, turned off, removed, libraries, password |
//! | [`invites`] | invite links and the public sign-up |
//! | [`recovery`] | password reset by mail |
//! | [`expiry`] | accounts that end on a date, with reminders |
//! | [`external_login`] | signing in with a Plex or Jellyfin account |
//! | [`jfa_go`] | moving in from jfa-go once: its users and open invites, with a dry run first |
//!
//! ```text
//! GET  /api/media-servers                      → {jellyfin, plex, mail}: whether each is set up and answers (admins)
//! GET  /api/media-servers/{server}/libraries   → [{id, name, kind}] (admins)
//! GET  /api/media-servers/{server}/users       → [{id, name, disabled, admin, user_id, username}] the server's users and
//!                                                the Marquee account each is linked to (admins)
//! POST /api/media-servers/scan                 {server?} → {ok, errors}: scan Jellyfin and Plex now (admins, API key)
//! POST /api/media-servers/refresh              → {jellyfin, plex}: read again what each server has (admins)
//! GET  /api/media-servers/available?kind=movie&tmdb_ids=603,604
//!                                              → {"603": ["jellyfin", "plex"], "604": []} (signed in)
//! ```

pub mod account_sync;
pub mod expiry;
pub mod external_login;
pub mod invites;
pub mod jellyfin;
pub mod jfa_go;
pub mod plex;
pub mod recovery;

use crate::roles::Me;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use jellyfin::Jellyfin;
use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use plex::Plex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The two media servers, as Marquee names them.
pub const SERVERS: [&str; 2] = ["jellyfin", "plex"];

/// Makes or updates this track's tables (called once at start-up, after the core tables).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS media_available (
            server TEXT NOT NULL,
            kind TEXT NOT NULL,
            tmdb_id INTEGER NOT NULL,
            item_id TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (server, kind, tmdb_id)
        );
        CREATE TABLE IF NOT EXISTS media_state (
            server TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL DEFAULT '',
            refreshed_at REAL,
            error TEXT NOT NULL DEFAULT ''
        );",
    )
    .map_err(|e| anyhow::anyhow!("The media server tables can't be made: {}", e.detail))?;
    account_sync::migrate(db)?;
    invites::migrate(db)?;
    recovery::migrate(db)?;
    expiry::migrate(db)
}

/// This track's routes (merged into the server's router).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/media-servers", get(status))
        .route("/api/media-servers/{server}/libraries", get(libraries))
        .route("/api/media-servers/{server}/users", get(remote_users))
        .route("/api/media-servers/scan", post(scan_now))
        .route("/api/media-servers/refresh", post(refresh_now))
        .route("/api/media-servers/available", get(available))
        .merge(account_sync::router())
        .merge(invites::router())
        .merge(recovery::router())
        .merge(expiry::router())
        .merge(external_login::router())
        .merge(jfa_go::router())
}

/// Starts the background work: what each server has, read again every 30 minutes, and the hourly account
/// expiry check.
pub fn spawn(st: AppState) {
    let s = st.clone();
    tokio::spawn(async move {
        loop {
            refresh(&s).await;
            tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
        }
    });
    tokio::spawn(async move {
        loop {
            expiry::check(&st).await;
            tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
        }
    });
}

/// 404 unless `server` is `jellyfin` or `plex`.
fn known(server: &str) -> Result<()> {
    if SERVERS.contains(&server) {
        Ok(())
    } else {
        Err(err(404, "The server must be jellyfin or plex"))
    }
}

/// The Plex server's machine id (asked once, then kept); 400 when Plex isn't set up.
pub async fn plex_machine(st: &AppState, plex: &Plex) -> Result<String> {
    if let Some(r) = st.db.one("SELECT machine_id FROM media_state WHERE server='plex' AND machine_id!=''", p![])? {
        return Ok(r.s("machine_id"));
    }
    let (_, _, machine) = plex.info().await?;
    if machine.is_empty() {
        return Err(err(502, "Plex didn't say which server it is"));
    }
    st.db.execute(
        "INSERT INTO media_state (server, machine_id) VALUES ('plex', ?) ON CONFLICT(server) DO UPDATE SET machine_id=excluded.machine_id",
        p![machine.clone()],
    )?;
    Ok(machine)
}

/// The Plex client, 400 when Plex isn't set up.
pub fn plex(st: &AppState) -> Result<Plex> {
    Plex::from_settings(st).ok_or_else(|| err(400, "Plex isn't set up. Add its address and token in Settings → Media servers."))
}

/// The Jellyfin client, 400 when Jellyfin isn't set up.
pub fn jellyfin(st: &AppState) -> Result<Jellyfin> {
    Jellyfin::from_settings(st).ok_or_else(|| err(400, "Jellyfin isn't set up. Add its address and API key in Settings → Media servers."))
}

/// `GET /api/media-servers`: whether each server is set up and answers.
async fn status(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    let jf = match Jellyfin::from_settings(&st) {
        None => json!({ "configured": false }),
        Some(j) => match j.info().await {
            Ok((name, version)) => json!({ "configured": true, "ok": true, "name": name, "version": version }),
            Err(e) => json!({ "configured": true, "ok": false, "error": e.detail }),
        },
    };
    let px = match Plex::from_settings(&st) {
        None => json!({ "configured": false }),
        Some(p) => match p.info().await {
            Ok((name, version, _)) => json!({ "configured": true, "ok": true, "name": name, "version": version }),
            Err(e) => json!({ "configured": true, "ok": false, "error": e.detail }),
        },
    };
    Ok(Json(json!({ "jellyfin": jf, "plex": px, "mail": { "configured": crate::mail::enabled(&st) } })))
}

/// Every library of `server`, as `{id, name, kind}`.
pub async fn server_libraries(st: &AppState, server: &str) -> Result<Vec<Value>> {
    Ok(match server {
        "jellyfin" => jellyfin(st)?.libraries().await?.into_iter().map(|l| json!(l)).collect(),
        _ => plex(st)?.libraries().await?.into_iter().map(|l| json!(l)).collect(),
    })
}

/// `GET /api/media-servers/{server}/libraries`.
async fn libraries(State(st): State<AppState>, me: Me, Path(server): Path<String>) -> Result<Json<Value>> {
    me.admin()?;
    known(&server)?;
    Ok(Json(json!(server_libraries(&st, &server).await?)))
}

/// `GET /api/media-servers/{server}/users`: the server's users (Jellyfin) or shares (Plex), each with the Marquee
/// account it is linked to.
async fn remote_users(State(st): State<AppState>, me: Me, Path(server): Path<String>) -> Result<Json<Value>> {
    me.admin()?;
    known(&server)?;
    let links = st.db.query(
        "SELECT m.remote_id, m.share_id, m.user_id, u.username FROM media_accounts m JOIN users u ON u.id=m.user_id WHERE m.server=?",
        p![server.clone()],
    )?;
    let linked = |id: &str| links.iter().find(|l| !id.is_empty() && (l.s("remote_id") == id || l.s("share_id") == id));
    let list: Vec<Value> = if server == "jellyfin" {
        jellyfin(&st)?
            .users()
            .await?
            .into_iter()
            .map(|u| {
                let l = linked(&u.id);
                json!({ "id": u.id, "name": u.name, "disabled": u.disabled, "admin": u.admin,
                        "user_id": l.map(|l| l.i("user_id")), "username": l.map(|l| l.s("username")) })
            })
            .collect()
    } else {
        let px = plex(&st)?;
        let machine = plex_machine(&st, &px).await?;
        px.shares(&machine)
            .await?
            .into_iter()
            .map(|s| {
                let l = linked(&s.id).or_else(|| linked(&s.user_id));
                json!({ "id": s.id, "name": if s.username.is_empty() { s.email.clone() } else { s.username.clone() }, "email": s.email,
                        "plex_user_id": s.user_id, "disabled": false, "admin": false,
                        "user_id": l.map(|l| l.i("user_id")), "username": l.map(|l| l.s("username")) })
            })
            .collect()
    };
    Ok(Json(json!(list)))
}

/// Asks Jellyfin and Plex (those that are set up) to scan their libraries; the errors in words. The downloads
/// track calls this after an import.
pub async fn scan(st: &AppState, only: Option<&str>) -> Vec<String> {
    let mut errors = Vec::new();
    if only.is_none_or(|s| s == "jellyfin") {
        if let Some(j) = Jellyfin::from_settings(st) {
            if let Err(e) = j.scan().await {
                errors.push(e.detail);
            }
        }
    }
    if only.is_none_or(|s| s == "plex") {
        if let Some(px) = Plex::from_settings(st) {
            if let Err(e) = px.scan().await {
                errors.push(e.detail);
            }
        }
    }
    errors
}

/// Scans in the background after an import, and reads again what the servers have a minute later.
pub fn scan_after_import(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move {
        for e in scan(&st, None).await {
            tracing::warn!("A library scan after an import failed: {e}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        refresh(&st).await;
    });
}

/// `POST /api/media-servers/scan` body.
#[derive(Deserialize, Default)]
pub struct ScanBody {
    /// `jellyfin` or `plex`; both when missing
    pub server: Option<String>,
}

/// `POST /api/media-servers/scan`.
async fn scan_now(State(st): State<AppState>, me: Me, body: Option<Json<ScanBody>>) -> Result<Json<Value>> {
    me.admin()?;
    let only = body.and_then(|b| b.0.server);
    if let Some(s) = &only {
        known(s)?;
    }
    let errors = scan(&st, only.as_deref()).await;
    Ok(Json(json!({ "ok": errors.is_empty(), "errors": errors })))
}

/// One thing a server has: its id there, `movie` or `tv`, and its TMDB id.
type Held = (String, &'static str, i64);

/// Reads again what each set-up server has and keeps it in `media_available`; per server, the count or the
/// error.
pub async fn refresh(st: &AppState) -> Value {
    let mut out = json!({});
    for server in SERVERS {
        let items: Option<Result<Vec<Held>>> = match server {
            "jellyfin" => match Jellyfin::from_settings(st) {
                None => None,
                Some(j) => Some(j.items().await.map(|v| v.into_iter().map(|i| (i.id, i.kind, i.tmdb_id)).collect())),
            },
            _ => match Plex::from_settings(st) {
                None => None,
                Some(px) => Some(px.items().await.map(|v| v.into_iter().map(|i| (i.id, i.kind, i.tmdb_id)).collect())),
            },
        };
        let res = match items {
            None => {
                let _ = st.db.execute("DELETE FROM media_available WHERE server=?", p![server]);
                json!({ "configured": false })
            }
            Some(Err(e)) => {
                let _ = st.db.execute(
                    "INSERT INTO media_state (server, error) VALUES (?,?) ON CONFLICT(server) DO UPDATE SET error=excluded.error",
                    p![server, e.detail.clone()],
                );
                json!({ "configured": true, "error": e.detail })
            }
            Some(Ok(items)) => {
                let n = items.len();
                let save = || -> Result<()> {
                    st.db.execute("DELETE FROM media_available WHERE server=?", p![server])?;
                    for (id, kind, tmdb) in &items {
                        st.db.execute(
                            "INSERT OR REPLACE INTO media_available (server, kind, tmdb_id, item_id) VALUES (?,?,?,?)",
                            p![server, *kind, *tmdb, id.clone()],
                        )?;
                    }
                    st.db.execute(
                        "INSERT INTO media_state (server, refreshed_at, error) VALUES (?,?,'') ON CONFLICT(server) DO UPDATE SET refreshed_at=excluded.refreshed_at, error=''",
                        p![server, crate::auth::now()],
                    )?;
                    Ok(())
                };
                match save() {
                    Ok(()) => json!({ "configured": true, "items": n }),
                    Err(e) => json!({ "configured": true, "error": e.detail }),
                }
            }
        };
        out[server] = res;
    }
    out
}

/// `POST /api/media-servers/refresh`.
async fn refresh_now(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    Ok(Json(refresh(&st).await))
}

/// The servers that have the movie (`kind` `movie`) or series (`tv`) with TMDB id `tmdb_id`.
pub fn available_on(db: &SqliteDb, kind: &str, tmdb_id: i64) -> Result<Vec<String>> {
    Ok(db
        .query("SELECT server FROM media_available WHERE kind=? AND tmdb_id=? ORDER BY server", p![kind, tmdb_id])?
        .iter()
        .map(|r| r.s("server"))
        .collect())
}

/// `GET /api/media-servers/available` query.
#[derive(Deserialize)]
pub struct AvailableQuery {
    /// `movie` or `tv`
    pub kind: String,
    /// TMDB ids, separated by commas
    pub tmdb_ids: String,
}

/// `GET /api/media-servers/available`.
async fn available(State(st): State<AppState>, _me: Me, Query(q): Query<AvailableQuery>) -> Result<Json<Value>> {
    if q.kind != "movie" && q.kind != "tv" {
        return Err(err(400, "The kind must be movie or tv"));
    }
    let mut out = BTreeMap::new();
    for id in q.tmdb_ids.split(',').filter_map(|t| t.trim().parse::<i64>().ok()).take(500) {
        out.insert(id.to_string(), available_on(&st.db, &q.kind, id)?);
    }
    Ok(Json(json!(out)))
}
