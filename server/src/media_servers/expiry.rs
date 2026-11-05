//! Accounts that end on a date (an invite can give each new account a number of days; an admin can set or
//! clear the date). A few days before (`signup_expiry_reminder_days`) the person gets a reminder mail; on the
//! date the account is turned off everywhere, or removed everywhere (`signup_expiry_action`). The owner's
//! account never ends.
//!
//! ```text
//! PUT /api/users/{id}/expiry   {expires_at: <seconds since 1970> | null} or {days: n} → {expires_at} (admins)
//! ```

use crate::roles::{Me, Role};
use crate::AppState;
use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// Adds the expiry columns to `users` where they are missing: `expires_at` (0: never) and `expiry_reminded`
/// (when the reminder went out, 0: not yet).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    let e = |e: kit_server_core::Error| anyhow::anyhow!("The accounts table can't be updated: {}", e.detail);
    let have: Vec<String> = db.query("PRAGMA table_info(users)", p![]).map_err(e)?.iter().map(|r| r.s("name")).collect();
    for (name, def) in [("expires_at", "REAL NOT NULL DEFAULT 0"), ("expiry_reminded", "REAL NOT NULL DEFAULT 0")] {
        if !have.iter().any(|h| h == name) {
            db.execute_batch(&format!("ALTER TABLE users ADD COLUMN {name} {def}")).map_err(e)?;
        }
    }
    Ok(())
}

/// The expiry route.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/users/{id}/expiry", put(set))
}

/// When account `u` ends, seconds since 1970; `null` for never.
pub fn expires_at(u: &Row) -> Value {
    let t = u.f("expires_at");
    if t > 0.0 {
        json!(t)
    } else {
        Value::Null
    }
}

/// Sets when account `id` ends (`None`: never) and forgets an earlier reminder.
pub fn set_expiry(db: &SqliteDb, id: i64, at: Option<f64>) -> Result<()> {
    db.execute("UPDATE users SET expires_at=?, expiry_reminded=0 WHERE id=?", p![at.unwrap_or(0.0), id]).map(|_| ())
}

/// `PUT /api/users/{id}/expiry` body.
#[derive(Deserialize)]
pub struct ExpiryBody {
    /// when it ends, seconds since 1970; `null` for never
    #[serde(default)]
    pub expires_at: Option<f64>,
    /// or: this many days from now
    pub days: Option<i64>,
}

/// `PUT /api/users/{id}/expiry`.
async fn set(State(st): State<AppState>, me: Me, Path(id): Path<i64>, Json(b): Json<ExpiryBody>) -> Result<Json<Value>> {
    me.admin()?;
    let u = st.db.one("SELECT * FROM users WHERE id=?", p![id])?.ok_or_else(|| err(404, "There is no such account"))?;
    if Role::parse(&u.s("role")) == Role::Owner {
        return Err(err(400, "The owner's account never ends"));
    }
    let at = match b.days {
        Some(d) if !(1..=3650).contains(&d) => return Err(err(400, "The number of days must be 1 to 3650")),
        Some(d) => Some(crate::auth::now() + d as f64 * 86400.0),
        None => b.expires_at.filter(|t| *t > 0.0),
    };
    set_expiry(&st.db, id, at)?;
    if at.is_some_and(|t| t > crate::auth::now()) && u.b("disabled") && u.f("expires_at") > 0.0 && u.f("expires_at") <= crate::auth::now() {
        // a later date for an account that had ended: on again
        st.db.execute("UPDATE users SET disabled=0 WHERE id=?", p![id])?;
        super::account_sync::sync(&st, id, None, false).await;
    }
    Ok(Json(json!({ "expires_at": at })))
}

/// A date in words for a mail: `5 November 2026`.
fn day(t: f64) -> String {
    chrono::DateTime::from_timestamp(t as i64, 0).map(|d| d.format("%-d %B %Y").to_string()).unwrap_or_default()
}

/// Sends the reminders that are due and ends the accounts whose date has passed (hourly).
pub async fn check(st: &AppState) {
    if let Err(e) = check_now(st).await {
        tracing::warn!("The account expiry check failed: {}", e.detail);
    }
}

/// [`check`], with the error.
pub async fn check_now(st: &AppState) -> Result<()> {
    let now = crate::auth::now();
    let app = st.app_name();
    let link = crate::mail::public_url(st);
    let days = st.settings.int("signup_expiry_reminder_days").max(0) as f64;
    if days > 0.0 {
        let due = st.db.query(
            "SELECT * FROM users WHERE expires_at>? AND expires_at<=? AND expiry_reminded=0 AND disabled=0 AND role!='owner'",
            p![now, now + days * 86400.0],
        )?;
        for u in due {
            st.db.execute("UPDATE users SET expiry_reminded=? WHERE id=?", p![now, u.i("id")])?;
            if u.s("email").trim().is_empty() || !crate::mail::enabled(st) {
                continue;
            }
            let text = format!(
                "Hello {},\n\nYour {app} account ends on {}. After that you can't sign in to {app}, Jellyfin or Plex any more.\n\nIf you'd like to keep it, ask the person who invited you.\n{}",
                u.s("username"),
                day(u.f("expires_at")),
                if link.is_empty() { String::new() } else { format!("\n{link}\n") }
            );
            crate::mail::send_later(st, crate::mail::Mail { to: u.s("email"), subject: format!("Your {app} account ends soon"), text });
        }
    }
    let ended = st.db.query("SELECT * FROM users WHERE expires_at>0 AND expires_at<=? AND disabled=0 AND role!='owner'", p![now])?;
    let remove = st.settings.str("signup_expiry_action") == "delete";
    for u in ended {
        let id = u.i("id");
        if remove {
            for e in super::account_sync::remove_everywhere(st, id).await {
                tracing::warn!("Removing the ended account '{}': {e}", u.s("username"));
            }
            st.db.execute("DELETE FROM sessions WHERE user_id=?", p![id])?;
            st.db.execute("DELETE FROM users WHERE id=?", p![id])?;
            tracing::info!("The account '{}' ended and was removed everywhere", u.s("username"));
        } else {
            st.db.execute("UPDATE users SET disabled=1 WHERE id=?", p![id])?;
            st.db.execute("DELETE FROM sessions WHERE user_id=?", p![id])?;
            super::account_sync::sync(st, id, None, false).await;
            tracing::info!("The account '{}' ended and was turned off everywhere", u.s("username"));
        }
    }
    Ok(())
}
