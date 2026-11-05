//! Accounts, shared by requests, movies and TV: one list of people, each with a [`Role`], request quotas and
//! auto-approve.
//!
//! ```text
//! GET    /api/roles        → [{id, label, description}] every role a person may give (signed in)
//! GET    /api/users        → [user] every account (admins)
//! POST   /api/users        {username, password, email?, role?, quotas…} → the new account (admins)
//! GET    /api/users/{id}   → one account (admins, or the account itself)
//! PATCH  /api/users/{id}   {email?, password?, current_password?, role?, disabled?, quotas…} → the account
//!                          (admins change anything but the owner's role; an account itself only its email and
//!                          password, with its current password)
//! DELETE /api/users/{id}   → {ok}: the account and its sessions removed (admins; never the owner or yourself)
//! ```
//!
//! Quotas: `movie_quota_limit` movies in `movie_quota_days` days, `tv_quota_limit` seasons in `tv_quota_days`
//! days; a limit of 0 means no limit. `auto_approve_movies` and `auto_approve_tv` skip the approval step.

use crate::auth::{create_user, hash_password, validate_credentials, verify_password};
use crate::roles::{Me, Role};
use crate::AppState;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The request columns every account has, with their SQL definitions (added to older databases).
const COLUMNS: [(&str, &str); 6] = [
    ("movie_quota_limit", "INTEGER NOT NULL DEFAULT 0"),
    ("movie_quota_days", "INTEGER NOT NULL DEFAULT 7"),
    ("tv_quota_limit", "INTEGER NOT NULL DEFAULT 0"),
    ("tv_quota_days", "INTEGER NOT NULL DEFAULT 7"),
    ("auto_approve_movies", "INTEGER NOT NULL DEFAULT 0"),
    ("auto_approve_tv", "INTEGER NOT NULL DEFAULT 0"),
];

/// Adds the request columns to `users` where they are missing, and turns the template's `user` role into
/// `requester`.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    let e = |e: kit_server_core::Error| anyhow::anyhow!("The accounts table can't be updated: {}", e.detail);
    let have: Vec<String> = db.query("PRAGMA table_info(users)", p![]).map_err(e)?.iter().map(|r| r.s("name")).collect();
    for (name, def) in COLUMNS {
        if !have.iter().any(|h| h == name) {
            db.execute_batch(&format!("ALTER TABLE users ADD COLUMN {name} {def}")).map_err(e)?;
        }
    }
    db.execute("UPDATE users SET role='requester' WHERE role NOT IN ('owner','admin','requester','viewer')", p![]).map_err(e)?;
    Ok(())
}

/// The account routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/roles", get(roles))
        .route("/api/users", get(list).post(create))
        .route("/api/users/{id}", get(one).patch(update).delete(remove))
}

/// An account as an admin sees it: no password hash, the role's words, booleans as booleans.
pub fn public(u: &Row) -> Value {
    let role = Role::parse(&u.s("role"));
    json!({
        "id": u.i("id"),
        "username": u.s("username"),
        "email": u.s("email"),
        "role": role.as_str(),
        "role_label": role.label(),
        "is_admin": role.is_admin(),
        "can_request": role.can_request(),
        "disabled": u.b("disabled"),
        "created_at": u.f("created_at"),
        "movie_quota_limit": u.i("movie_quota_limit"),
        "movie_quota_days": u.i("movie_quota_days"),
        "tv_quota_limit": u.i("tv_quota_limit"),
        "tv_quota_days": u.i("tv_quota_days"),
        "auto_approve_movies": u.b("auto_approve_movies"),
        "auto_approve_tv": u.b("auto_approve_tv"),
    })
}

/// The account `id`, 404 when there is none.
fn find(db: &SqliteDb, id: i64) -> Result<Row> {
    db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| err(404, "There is no such account"))
}

/// `GET /api/roles`: the roles a person may give, with their words.
async fn roles(_me: Me) -> Json<Value> {
    let list: Vec<Value> = Role::ALL
        .iter()
        .filter(|r| **r != Role::Owner)
        .map(|r| json!({ "id": r.as_str(), "label": r.label(), "description": r.description() }))
        .collect();
    Json(json!(list))
}

/// `GET /api/users`: every account, the owner first.
async fn list(State(st): State<AppState>, me: Me) -> Result<Json<Value>> {
    me.admin()?;
    let rows = st.db.query("SELECT * FROM users ORDER BY role='owner' DESC, username COLLATE NOCASE", p![])?;
    Ok(Json(json!(rows.iter().map(public).collect::<Vec<_>>())))
}

/// The quota and auto-approve fields of a create or update body.
#[derive(Deserialize, Default)]
pub struct Limits {
    /// movies per period (0: no limit)
    pub movie_quota_limit: Option<i64>,
    /// the movie period in days
    pub movie_quota_days: Option<i64>,
    /// seasons per period (0: no limit)
    pub tv_quota_limit: Option<i64>,
    /// the TV period in days
    pub tv_quota_days: Option<i64>,
    /// movie requests skip approval
    pub auto_approve_movies: Option<bool>,
    /// TV requests skip approval
    pub auto_approve_tv: Option<bool>,
}

impl Limits {
    /// The `(column, value)` pairs this body sets; 400 for a limit below 0 or a period outside 1–365 days.
    fn columns(&self) -> Result<Vec<(&'static str, Value)>> {
        let mut out = Vec::new();
        for (name, v) in [("movie_quota_limit", self.movie_quota_limit), ("tv_quota_limit", self.tv_quota_limit)] {
            if let Some(v) = v {
                if v < 0 {
                    return Err(err(400, "A request limit can't be below 0 (0 means no limit)"));
                }
                out.push((name, json!(v)));
            }
        }
        for (name, v) in [("movie_quota_days", self.movie_quota_days), ("tv_quota_days", self.tv_quota_days)] {
            if let Some(v) = v {
                if !(1..=365).contains(&v) {
                    return Err(err(400, "A request limit's period must be 1 to 365 days"));
                }
                out.push((name, json!(v)));
            }
        }
        for (name, v) in [("auto_approve_movies", self.auto_approve_movies), ("auto_approve_tv", self.auto_approve_tv)] {
            if let Some(v) = v {
                out.push((name, json!(v as i64)));
            }
        }
        Ok(out)
    }
}

/// `POST /api/users` body.
#[derive(Deserialize)]
pub struct NewUser {
    /// the sign-in name
    pub username: String,
    /// the password (8 characters or more)
    pub password: String,
    /// an email address for notifications
    #[serde(default)]
    pub email: String,
    /// `admin`, `requester` (the default) or `viewer`
    pub role: Option<String>,
    /// quotas and auto-approve
    #[serde(flatten)]
    pub limits: Limits,
}

/// Sets `cols` on the account `id`.
fn set_columns(db: &SqliteDb, id: i64, cols: Vec<(&'static str, Value)>) -> Result<()> {
    for (name, v) in cols {
        // the names come from this file's fixed lists, never from the request
        db.execute(&format!("UPDATE users SET {name}=? WHERE id=?"), vec![v, json!(id)])?;
    }
    Ok(())
}

/// `POST /api/users`: a new account; the role and limits not given come from the "New accounts" settings.
async fn create(State(st): State<AppState>, me: Me, Json(b): Json<NewUser>) -> Result<Json<Value>> {
    me.admin()?;
    let role = match b.role.as_deref() {
        None | Some("") => Role::assignable(&st.settings.str("default_role")).filter(|r| !r.is_admin()).unwrap_or(Role::Requester),
        Some(r) => Role::assignable(r).ok_or_else(|| err(400, "The role must be admin, requester or viewer"))?,
    };
    let mut limits = b.limits;
    limits.movie_quota_limit = limits.movie_quota_limit.or(Some(st.settings.int("default_movie_quota")));
    limits.tv_quota_limit = limits.tv_quota_limit.or(Some(st.settings.int("default_tv_quota")));
    let cols = limits.columns()?;
    let id = create_user(&st.db, b.username.trim(), &b.password, role.as_str())?;
    set_columns(&st.db, id, cols)?;
    if !b.email.trim().is_empty() {
        st.db.execute("UPDATE users SET email=? WHERE id=?", p![b.email.trim(), id])?;
    }
    tracing::info!("{} made the account '{}' ({})", me.account.username, b.username.trim(), role.as_str());
    Ok(Json(public(&find(&st.db, id)?)))
}

/// `GET /api/users/{id}`: one account, for an admin or the account itself.
async fn one(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    if me.account.id != id {
        me.admin()?;
    }
    Ok(Json(public(&find(&st.db, id)?)))
}

/// `PATCH /api/users/{id}` body: every field is optional.
#[derive(Deserialize)]
pub struct Change {
    /// a new email address
    pub email: Option<String>,
    /// a new password
    pub password: Option<String>,
    /// the current password, when an account changes its own password
    pub current_password: Option<String>,
    /// a new role (admins; never the owner's)
    pub role: Option<String>,
    /// turn sign-in off or on (admins; never the owner or yourself)
    pub disabled: Option<bool>,
    /// quotas and auto-approve (admins)
    #[serde(flatten)]
    pub limits: Limits,
}

/// `PATCH /api/users/{id}`: change an account.
async fn update(State(st): State<AppState>, me: Me, Path(id): Path<i64>, Json(b): Json<Change>) -> Result<Json<Value>> {
    let user = find(&st.db, id)?;
    let own = me.account.id == id;
    let is_owner = Role::parse(&user.s("role")) == Role::Owner;
    let admin_only = b.role.is_some() || b.disabled.is_some() || !b.limits.columns()?.is_empty();
    if !own || admin_only {
        me.admin()?;
    }
    if let Some(r) = b.role.as_deref() {
        let role = Role::assignable(r).ok_or_else(|| err(400, "The role must be admin, requester or viewer"))?;
        if is_owner {
            return Err(err(400, "The owner's role can't be changed"));
        }
        if own && !role.is_admin() {
            return Err(err(400, "You can't take away your own admin rights; ask another admin"));
        }
        st.db.execute("UPDATE users SET role=? WHERE id=?", p![role.as_str(), id])?;
    }
    if let Some(d) = b.disabled {
        if is_owner || own {
            return Err(err(400, "The owner and your own account can't be turned off"));
        }
        st.db.execute("UPDATE users SET disabled=? WHERE id=?", p![d as i64, id])?;
        if d {
            st.db.execute("DELETE FROM sessions WHERE user_id=?", p![id])?;
        }
    }
    if let Some(pw) = b.password.as_deref() {
        if own {
            let current = b.current_password.clone().unwrap_or_default();
            let hash = user.s("password_hash");
            let ok = tokio::task::spawn_blocking(move || verify_password(&current, &hash)).await.unwrap_or(false);
            if !ok {
                return Err(err(400, "Your current password is wrong"));
            }
        }
        validate_credentials(&user.s("username"), pw)?;
        let pw = pw.to_string();
        let hash = tokio::task::spawn_blocking(move || hash_password(&pw)).await.unwrap_or_default();
        st.db.execute("UPDATE users SET password_hash=? WHERE id=?", p![hash, id])?;
    }
    if let Some(e) = b.email.as_deref() {
        st.db.execute("UPDATE users SET email=? WHERE id=?", p![e.trim(), id])?;
    }
    set_columns(&st.db, id, b.limits.columns()?)?;
    Ok(Json(public(&find(&st.db, id)?)))
}

/// `DELETE /api/users/{id}`: remove an account and its sessions.
async fn remove(State(st): State<AppState>, me: Me, Path(id): Path<i64>) -> Result<Json<Value>> {
    me.admin()?;
    let user = find(&st.db, id)?;
    if Role::parse(&user.s("role")) == Role::Owner {
        return Err(err(400, "The owner's account can't be removed"));
    }
    if me.account.id == id {
        return Err(err(400, "You can't remove your own account; ask another admin"));
    }
    st.db.execute("DELETE FROM sessions WHERE user_id=?", p![id])?;
    st.db.execute("DELETE FROM users WHERE id=?", p![id])?;
    tracing::info!("{} removed the account '{}'", me.account.username, user.s("username"));
    Ok(Json(json!({ "ok": true })))
}
