//! Moving in from jfa-go, once: its users and its open invites become Marquee accounts and invites, with a
//! dry run first. jfa-go is asked through its own API with an admin's name and password.
//!
//! ```text
//! POST /api/migrate/jfa-go   {url, username, password, apply?} → the report: what was (or would be) added,
//!                            skipped and why, and the problems (admins; a dry run unless `apply` is true)
//! ```
//!
//! | jfa-go | Marquee |
//! | --- | --- |
//! | a user | the account linked to the same Jellyfin user, else the account with the same name, else a new account; linked to its Jellyfin user with the libraries it has now |
//! | its mail address | the account's, when the account has none |
//! | its end date | the account's end date |
//! | turned off | the account turned off |
//! | a Jellyfin administrator | left alone (link it by hand in Media access if wanted) |
//! | an open invite | an invite with the same code, uses left, end date, account length and the libraries of its profile's user |
//!
//! New accounts get a random password: people sign in with Jellyfin (which sets their Marquee password) or
//! ask for a reset mail. Nothing is changed on Jellyfin or in jfa-go.

use super::account_sync::Libraries;
use super::jellyfin::{Jellyfin, User as JfUser};
use crate::roles::Me;
use crate::AppState;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt};
use serde::Deserialize;
use serde_json::Value;

/// The route of this module.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/migrate/jfa-go", post(migrate))
}

/// `POST /api/migrate/jfa-go` body.
#[derive(Deserialize)]
pub struct Source {
    /// jfa-go's address, with its URL base if it has one (`http://jfa-go:8056`)
    pub url: String,
    /// a jfa-go admin's name
    pub username: String,
    /// that admin's password
    pub password: String,
    /// make the changes (default: a dry run that only says what would happen)
    #[serde(default)]
    pub apply: bool,
}

/// One line of the report.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// what it is about (`User sam`, `Invite Family`)
    pub title: String,
    /// `add`, `skip` or `problem`
    pub action: &'static str,
    /// what happens, or why not, in words
    pub detail: String,
}

/// What a run did, or would do.
#[derive(Debug, Default)]
pub struct Plan {
    /// every line, in order
    pub items: Vec<Item>,
    /// notes for the whole run
    pub notes: Vec<String>,
}

impl Plan {
    /// Adds a line.
    fn push(&mut self, title: impl Into<String>, action: &'static str, detail: impl Into<String>) {
        self.items.push(Item { title: title.into(), action, detail: detail.into() });
    }

    /// The report as the Move in page reads it: core's [`crate::migrate::Report`], like every other source.
    pub fn report(&self, applied: bool) -> crate::migrate::Report {
        let count = |a: &str| self.items.iter().filter(|i| i.action == a).count();
        crate::migrate::Report {
            source: "jfa-go".into(),
            applied,
            added: count("add"),
            skipped: count("skip"),
            problems: count("problem"),
            files: 0,
            notes: self.notes.clone(),
            items: self
                .items
                .iter()
                .map(|i| crate::migrate::Item { title: i.title.clone(), action: i.action.into(), detail: i.detail.clone(), files: 0 })
                .collect(),
        }
    }
}

/// A jfa-go client, signed in.
struct Jfa {
    /// the address, without a trailing `/`
    base: String,
    /// the session token
    token: String,
    /// the shared HTTP client
    http: reqwest::Client,
}

impl Jfa {
    /// Signs in to jfa-go at `url` with an admin's name and password.
    async fn sign_in(st: &AppState, url: &str, username: &str, password: &str) -> Result<Jfa> {
        let base = url.trim().trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") {
            return Err(err(400, "jfa-go's address must start with http:// or https://"));
        }
        let r = st
            .http
            .get(format!("{base}/token/login"))
            .basic_auth(username.trim(), Some(password))
            .send()
            .await
            .map_err(|e| err(502, format!("jfa-go at {base} can't be reached: {}", super::jellyfin::plain(&e))))?;
        if matches!(r.status().as_u16(), 401 | 403) {
            return Err(err(400, "jfa-go turned the name or password down. Use a jfa-go admin's sign-in."));
        }
        if !r.status().is_success() {
            return Err(err(
                502,
                format!("jfa-go answered {} to the sign-in. Check the address, with its URL base if it has one.", r.status()),
            ));
        }
        let v: Value = r.json().await.map_err(|_| err(502, "jfa-go's answer to the sign-in can't be read. Is that jfa-go's address?"))?;
        let token = v["token"].as_str().unwrap_or("").to_string();
        if token.is_empty() {
            return Err(err(502, "jfa-go didn't hand out a session. Is that jfa-go's address?"));
        }
        Ok(Jfa { base, token, http: st.http.clone() })
    }

    /// `GET <path>` as JSON.
    async fn get(&self, path: &str) -> Result<Value> {
        let r = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| err(502, format!("jfa-go can't be reached: {}", super::jellyfin::plain(&e))))?;
        if !r.status().is_success() {
            return Err(err(502, format!("jfa-go answered {} to {path}", r.status())));
        }
        r.json().await.map_err(|_| err(502, format!("jfa-go's answer to {path} can't be read")))
    }
}

/// `POST /api/migrate/jfa-go`.
async fn migrate(State(st): State<AppState>, me: Me, Json(b): Json<Source>) -> Result<Json<crate::migrate::Report>> {
    me.admin()?;
    let plan = run(&st, &b).await?;
    Ok(Json(plan.report(b.apply)))
}

/// A string field, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A number field, 0 when missing.
fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

/// How the links table keeps a Jellyfin user's libraries.
fn stored(u: &JfUser) -> String {
    match &u.libraries {
        None => "*".into(),
        Some(l) => Libraries::Some(l.clone()).stored(),
    }
}

/// Reads jfa-go and Jellyfin, and brings the users and open invites over (only with `apply`).
pub async fn run(st: &AppState, src: &Source) -> Result<Plan> {
    let jf = super::jellyfin(st)
        .map_err(|_| err(400, "Set up Jellyfin in Settings → Media servers first: jfa-go's users are Jellyfin users."))?;
    let jfa = Jfa::sign_in(st, &src.url, &src.username, &src.password).await?;
    let jf_users = jf.users().await?;
    let mut plan = Plan::default();
    users(st, &jfa, &jf_users, src.apply, &mut plan).await?;
    invites(st, &jfa, &jf, &jf_users, src.apply, &mut plan).await?;
    plan.notes.push("New accounts get a random password. People sign in with their Jellyfin name and password (that becomes their password here too), or ask for a reset mail.".into());
    plan.notes.push(
        "jfa-go profiles aren't a thing here: each invite carries its own libraries. Nothing was changed on Jellyfin or in jfa-go.".into(),
    );
    if !src.apply {
        plan.notes.push("This was a dry run: nothing was changed. Run it again with Apply to bring everything over.".into());
    }
    Ok(plan)
}

/// The users part of [`run`].
async fn users(st: &AppState, jfa: &Jfa, jf_users: &[JfUser], apply: bool, plan: &mut Plan) -> Result<()> {
    let list = jfa.get("/users").await?;
    let now = crate::auth::now();
    for u in list["users"].as_array().into_iter().flatten() {
        let (id, name) = (s(u, "id"), s(u, "name"));
        let title = format!("User {name}");
        let Some(ju) = jf_users.iter().find(|j| j.id == id) else {
            plan.push(title, "problem", "Jellyfin has no user with this id any more, so there is nothing to link.");
            continue;
        };
        if ju.admin || u["admin"].as_bool() == Some(true) {
            plan.push(title, "skip", "A Jellyfin administrator: left alone. Link it by hand in Media access if you want.");
            continue;
        }
        let email = s(u, "email");
        let expiry = n(u, "expiry") as f64;
        let disabled = u["disabled"].as_bool() == Some(true);
        let linked = st
            .db
            .one("SELECT user_id FROM media_accounts WHERE server='jellyfin' AND remote_id=?", p![id.clone()])?
            .map(|r| r.i("user_id"));
        let by_name = st.db.one("SELECT id FROM users WHERE username=? COLLATE NOCASE", p![name.clone()])?.map(|r| r.i("id"));
        let (account, how) = match (linked, by_name) {
            (Some(a), _) => (Some(a), "already linked"),
            (None, Some(a)) => {
                let other =
                    st.db.one("SELECT remote_id FROM media_accounts WHERE user_id=? AND server='jellyfin' AND remote_id!=''", p![a])?;
                if other.is_some() {
                    plan.push(
                        title,
                        "problem",
                        format!("The account {name} is already linked to another Jellyfin user. Link this one by hand in Media access."),
                    );
                    continue;
                }
                (Some(a), "linked to the account with the same name")
            }
            (None, None) => (None, "a new account"),
        };
        let mut changes: Vec<String> = Vec::new();
        let current = match account {
            Some(a) => st.db.one("SELECT * FROM users WHERE id=?", p![a])?,
            None => None,
        };
        if !email.is_empty() && current.as_ref().is_none_or(|c| c.s("email").trim().is_empty()) {
            changes.push(format!("mail address {email}"));
        }
        if expiry > 0.0 && current.as_ref().is_none_or(|c| (c.f("expires_at") - expiry).abs() > 1.0) {
            changes.push(format!(
                "ends {}",
                chrono::DateTime::from_timestamp(expiry as i64, 0).map(|d| d.format("%-d %B %Y").to_string()).unwrap_or_default()
            ));
            if expiry <= now {
                changes.push("already past its end date, so it will be turned off".into());
            }
        }
        if disabled && current.as_ref().is_none_or(|c| !c.b("disabled")) {
            changes.push("turned off".into());
        }
        if linked.is_none() {
            changes.push(format!("Jellyfin user {} linked", ju.name));
        }
        if changes.is_empty() {
            plan.push(title, "skip", "Already here as it is in jfa-go.");
            continue;
        }
        let detail = format!("{how}: {}", changes.join(", "));
        if !apply {
            plan.push(title, "add", detail);
            continue;
        }
        let res: Result<i64> = async {
            let a = match account {
                Some(a) => a,
                None => {
                    let uname = super::external_login::free_name(st, &name)?;
                    crate::users::create_account(st, &uname, &crate::auth::random_token(24), "", crate::users::default_role(st), Default::default())?
                }
            };
            if !email.is_empty() {
                st.db.execute("UPDATE users SET email=? WHERE id=? AND (email IS NULL OR TRIM(email)='')", p![email.clone(), a])?;
            }
            if expiry > 0.0 {
                super::expiry::set_expiry(&st.db, a, Some(expiry))?;
            }
            if disabled {
                st.db.execute("UPDATE users SET disabled=1 WHERE id=?", p![a])?;
                st.db.execute("DELETE FROM sessions WHERE user_id=?", p![a])?;
            }
            if linked.is_none() {
                st.db.execute("DELETE FROM media_accounts WHERE user_id=? AND server='jellyfin'", p![a])?;
                st.db.execute(
                    "INSERT INTO media_accounts (user_id, server, remote_id, remote_name, libraries, active, password_pending, synced_at) VALUES (?,'jellyfin',?,?,?,?,1,?)",
                    p![a, id.clone(), ju.name.clone(), stored(ju), (!ju.disabled) as i64, now],
                )?;
            }
            Ok(a)
        }
        .await;
        match res {
            Ok(_) => plan.push(title, "add", detail),
            Err(e) => plan.push(title, "problem", e.detail),
        }
    }
    Ok(())
}

/// The invites part of [`run`]: every invite that can still be used.
async fn invites(st: &AppState, jfa: &Jfa, jf: &Jellyfin, jf_users: &[JfUser], apply: bool, plan: &mut Plan) -> Result<()> {
    let list = jfa.get("/invites").await?;
    let profiles = jfa.get("/profiles").await.unwrap_or(Value::Null);
    let all_libraries = jf.libraries().await?;
    let now = crate::auth::now();
    for i in list["invites"].as_array().into_iter().flatten() {
        let code: String = s(i, "code").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        let label = s(i, "label");
        let title = format!("Invite {}", if label.is_empty() { code.clone() } else { label.clone() });
        let valid_till = n(i, "valid_till") as f64;
        let no_limit = i["no_limit"].as_bool() == Some(true);
        let left = n(i, "remaining_uses");
        if code.is_empty() {
            plan.push(title, "problem", "jfa-go gave no code for it.");
            continue;
        }
        if valid_till > 0.0 && valid_till <= now {
            plan.push(title, "skip", "It has already ended.");
            continue;
        }
        if !no_limit && left <= 0 {
            plan.push(title, "skip", "It has no uses left.");
            continue;
        }
        if st.db.one("SELECT code FROM invites WHERE code=?", p![code.clone()])?.is_some() {
            plan.push(title, "skip", "Already here.");
            continue;
        }
        // the profile's libraries: those of the Jellyfin user it was made from, or every library
        let profile = s(i, "profile");
        let pr = &profiles["profiles"][&profile];
        let libraries = if profile.is_empty() || pr.is_null() || s(pr, "libraries").eq_ignore_ascii_case("all") {
            Some("*".to_string())
        } else {
            let from = s(pr, "fromUser");
            jf_users.iter().find(|u| !from.is_empty() && u.name.eq_ignore_ascii_case(&from)).map(stored)
        };
        let Some(libraries) = libraries else {
            plan.push(title, "problem", format!("Its profile {profile} has chosen libraries, but the Jellyfin user it was made from is gone. Make this invite again in Invites."));
            continue;
        };
        let lib_words = if libraries == "*" {
            "every library".to_string()
        } else {
            let ids = Libraries::parse(&libraries).unwrap_or_default();
            all_libraries.iter().filter(|l| ids.contains(&l.id)).map(|l| l.name.clone()).collect::<Vec<_>>().join(", ")
        };
        let account_days = if i["user_expiry"].as_bool() == Some(true) {
            let mins = n(i, "user_months") * 30 * 1440 + n(i, "user_days") * 1440 + n(i, "user_hours") * 60 + n(i, "user_minutes");
            ((mins + 1439) / 1440).clamp(1, 3650)
        } else {
            0
        };
        let max_uses = if no_limit { 0 } else { left };
        let detail = format!(
            "{}, {}, Jellyfin with {lib_words}{}",
            if max_uses == 0 { "no use limit".to_string() } else { format!("{max_uses} uses left") },
            if valid_till > 0.0 {
                format!(
                    "ends {}",
                    chrono::DateTime::from_timestamp(valid_till as i64, 0).map(|d| d.format("%-d %B %Y").to_string()).unwrap_or_default()
                )
            } else {
                "never ends".into()
            },
            if account_days > 0 { format!(", accounts last {account_days} days") } else { String::new() }
        );
        if apply {
            st.db.execute(
                "INSERT INTO invites (code, label, max_uses, expires_at, account_days, role, jellyfin, jellyfin_libraries, sent_to, created_by, created_at)
                 VALUES (?,?,?,?,?,?,1,?,?,'jfa-go',?)",
                p![code, label, max_uses, valid_till.max(0.0), account_days, crate::users::default_role(st).as_str(), libraries, s(i, "send_to"), now],
            )?;
        }
        plan.push(title, "add", detail);
    }
    if profiles["profiles"].as_object().is_some_and(|p| !p.is_empty()) && list["invites"].as_array().is_none_or(|a| a.is_empty()) {
        plan.notes.push("jfa-go has profiles but no open invites, so nothing of the profiles came over.".into());
    }
    Ok(())
}
