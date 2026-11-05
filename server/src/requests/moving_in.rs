//! Moving in from Seerr (Overseerr, Jellyseerr): every request with its status, seasons, 4K flag, dates and the
//! person who asked, and each person's request limits and auto-approve. Seerr's people are matched to Marquee
//! accounts by username, email, or their Plex or Jellyfin name or id (the media servers import makes the accounts
//! first). A request whose title is already in the library comes in as available.
//!
//! ```text
//! POST /api/migrate/seerr   {url, api_key, apply?} → what would happen (apply: false, the default) or what did
//! ```
//!
//! Nothing is written on a dry run. Running it again skips what already moved in, so it is safe to repeat.

use super::{availability, media_by_tmdb, status, upsert_media, Media, TitleFacts};
use crate::roles::{Me, Role};
use crate::AppState;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The route.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/migrate/seerr", post(run))
}

/// `POST /api/migrate/seerr` body.
#[derive(Deserialize)]
pub struct Source {
    /// Seerr's address, for example `http://seerr:5055`
    pub url: String,
    /// Seerr → Settings → General → API key
    pub api_key: String,
    /// write it (`false`, the default, only says what would happen)
    #[serde(default)]
    pub apply: bool,
}

/// One line of what moving in did or would do.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// what it is about: a person or a title
    pub title: String,
    /// `add`, `skip` or `problem`
    pub action: &'static str,
    /// what happens, in words
    pub detail: String,
}

/// `POST /api/migrate/seerr` (admins): moves in Seerr's people's limits and its requests.
async fn run(State(st): State<AppState>, me: Me, Json(b): Json<Source>) -> Result<Json<Value>> {
    me.admin()?;
    let base = api_base(&b.url)?;
    let users = fetch_all(&st, &base, &b.api_key, "/user").await?;
    let requests = fetch_all(&st, &base, &b.api_key, "/request?filter=all&sort=added&sortDirection=asc").await?;
    let lines = move_in(&st, &users, &requests, b.apply).await?;
    if b.apply {
        tracing::info!("{} moved in requests from Seerr", me.account.username);
    }
    Ok(Json(report(b.apply, &lines)))
}

/// The answer: how many lines of each kind, and every line.
pub fn report(applied: bool, lines: &[Line]) -> Value {
    let n = |a: &str| lines.iter().filter(|l| l.action == a).count();
    json!({
        "source": "seerr",
        "applied": applied,
        "added": n("add"),
        "skipped": n("skip"),
        "problems": n("problem"),
        "items": lines.iter().map(|l| json!({ "title": l.title, "action": l.action, "detail": l.detail, "files": [] })).collect::<Vec<_>>(),
    })
}

/// Seerr's API address from what a person typed: `http://seerr:5055`, with or without `/api/v1` and a trailing `/`.
pub fn api_base(url: &str) -> Result<String> {
    let u = url.trim().trim_end_matches('/');
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err(err(400, "Seerr's address starts with http:// or https://, for example http://seerr:5055"));
    }
    let u = u.strip_suffix("/api/v1").unwrap_or(u);
    Ok(format!("{u}/api/v1"))
}

/// Every item of a paged Seerr list at `path`.
async fn fetch_all(st: &AppState, base: &str, key: &str, path: &str) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    let sep = if path.contains('?') { '&' } else { '?' };
    loop {
        let url = format!("{base}{path}{sep}take=100&skip={}", out.len());
        let r = st
            .http
            .get(&url)
            .header("X-Api-Key", key.trim())
            .send()
            .await
            .map_err(|e| err(502, format!("Seerr can't be reached at {base}: {e}")))?;
        let v: Value = match r.status().as_u16() {
            200 => r.json().await.map_err(|e| err(502, format!("Seerr sent an answer Marquee can't read: {e}")))?,
            401 | 403 => return Err(err(400, "Seerr refused the API key. Copy it from Seerr → Settings → General.")),
            s => return Err(err(502, format!("Seerr answered with an error ({s})"))),
        };
        let page = v["results"].as_array().cloned().unwrap_or_default();
        let total = v["pageInfo"]["results"].as_u64().unwrap_or(0) as usize;
        let empty = page.is_empty();
        out.extend(page);
        if empty || out.len() >= total || out.len() >= 100_000 {
            return Ok(out);
        }
    }
}

/// The Marquee account Seerr's user `u` is: the same username, email, or Plex/Jellyfin name or id.
pub fn match_account(db: &SqliteDb, u: &Value) -> Result<Option<Row>> {
    let text = |k: &str| u[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    for name in [text("username"), text("plexUsername"), text("jellyfinUsername"), text("displayName")].into_iter().flatten() {
        if let Some(r) = db.one("SELECT * FROM users WHERE username=? COLLATE NOCASE", p![name])? {
            return Ok(Some(r));
        }
    }
    if let Some(mail) = text("email") {
        if let Some(r) = db.one("SELECT * FROM users WHERE email=? COLLATE NOCASE AND email<>''", p![mail])? {
            return Ok(Some(r));
        }
    }
    let remote = [
        ("plex", u["plexId"].as_i64().map(|i| i.to_string()), text("plexUsername")),
        ("jellyfin", text("jellyfinUserId"), text("jellyfinUsername")),
    ];
    for (server, id, name) in remote {
        for (col, v) in [("remote_id", id), ("remote_name", name)] {
            let Some(v) = v else { continue };
            // the column names come from the fixed list above
            let sql =
                format!("SELECT u.* FROM users u JOIN media_accounts m ON m.user_id=u.id WHERE m.server=? AND m.{col}=? COLLATE NOCASE");
            if let Some(r) = db.one(&sql, p![server, v])? {
                return Ok(Some(r));
            }
        }
    }
    Ok(None)
}

/// The words for a Seerr user: its display name, else username, else email.
fn who(u: &Value) -> String {
    ["displayName", "username", "plexUsername", "jellyfinUsername", "email"]
        .iter()
        .filter_map(|k| u[k].as_str().filter(|s| !s.is_empty()))
        .next()
        .unwrap_or("someone")
        .to_string()
}

/// The limit and auto-approve columns Seerr's user `u` gives its Marquee account (empty for an admin: Marquee's
/// admins have no limits). A limit Seerr leaves to its global default is left as it is.
pub fn limits(u: &Value) -> Vec<(&'static str, i64)> {
    let perms = u["permissions"].as_i64().unwrap_or(0);
    if perms & super::seerr::perm::ADMIN != 0 {
        return vec![];
    }
    let mut out = Vec::new();
    for (col, k) in [
        ("movie_quota_limit", "movieQuotaLimit"),
        ("movie_quota_days", "movieQuotaDays"),
        ("tv_quota_limit", "tvQuotaLimit"),
        ("tv_quota_days", "tvQuotaDays"),
    ] {
        if let Some(v) = u[k].as_i64() {
            out.push((col, if col.ends_with("days") { v.clamp(1, 365) } else { v.max(0) }));
        }
    }
    let (_, movies, tv) = super::seerr::role_of(perms);
    out.push(("auto_approve_movies", movies as i64));
    out.push(("auto_approve_tv", tv as i64));
    out
}

/// The facts about a title: the library's, else TMDB's, else a placeholder the next refresh fills in.
async fn facts_or_placeholder(st: &AppState, kind: &str, tmdb_id: i64) -> (TitleFacts, bool) {
    match super::facts(st, kind, tmdb_id).await {
        Ok(f) => (f, true),
        Err(_) => {
            let what = if kind == "movie" { "Movie" } else { "Show" };
            (TitleFacts { title: format!("{what} {tmdb_id} on TMDB"), ..TitleFacts::default() }, false)
        }
    }
}

/// Whether Seerr's request `r` (movie or TV) already moved in: the same account asked for the same title and
/// seasons at the same time.
fn already_in(db: &SqliteDb, media_id: i64, user: i64, seasons: &[i64], created: &str) -> Result<bool> {
    let rows = db.query("SELECT * FROM requests WHERE media_id=? AND requested_by=? AND created_at=?", p![media_id, user, created])?;
    Ok(rows.iter().map(super::Request::from_row).any(|r| r.seasons == seasons))
}

/// Moves in `users`' limits and `requests` (Seerr's answers); writes only with `apply`. What happened, line by line.
pub async fn move_in(st: &AppState, users: &[Value], requests: &[Value], apply: bool) -> Result<Vec<Line>> {
    let mut lines = Vec::new();
    for u in users {
        let name = who(u);
        match match_account(&st.db, u)? {
            None => lines.push(Line {
                title: name,
                action: "problem",
                detail: "No Marquee account matches this person (username, email, Plex or Jellyfin). Their requests are skipped. Make the account, or move in from Jellyfin or jfa-go first, then run this again.".into(),
            }),
            Some(acc) => {
                let cols = limits(u);
                if cols.is_empty() || Role::parse(&acc.s("role")).is_admin() {
                    lines.push(Line { title: name, action: "skip", detail: format!("Matches {}, an admin: admins have no limits.", acc.s("username")) });
                    continue;
                }
                if apply {
                    for (col, v) in &cols {
                        // the column names come from the fixed list in `limits`
                        st.db.execute(&format!("UPDATE users SET {col}=? WHERE id=?"), p![v, acc.i("id")])?;
                    }
                }
                let get = |c: &str| cols.iter().find(|(k, _)| *k == c).map(|(_, v)| *v);
                let lim = |l: Option<i64>, d: Option<i64>, what: &str| match l {
                    Some(n) if n > 0 => format!("{n} {what} every {} days", d.unwrap_or(7)),
                    Some(_) => format!("no limit on {what}"),
                    None => format!("{what} limit unchanged"),
                };
                let auto = match (get("auto_approve_movies") == Some(1), get("auto_approve_tv") == Some(1)) {
                    (true, true) => "auto-approved",
                    (true, false) => "movies auto-approved",
                    (false, true) => "TV auto-approved",
                    _ => "needs approval",
                };
                lines.push(Line {
                    title: name,
                    action: "add",
                    detail: format!(
                        "Matches {}: {}, {}, {auto}.",
                        acc.s("username"),
                        lim(get("movie_quota_limit"), get("movie_quota_days"), "movies"),
                        lim(get("tv_quota_limit"), get("tv_quota_days"), "seasons")
                    ),
                });
            }
        }
    }
    // a dry run remembers what it would add, so a request twice in Seerr's list isn't counted twice
    let mut planned: Vec<(String, i64, i64, Vec<i64>, String)> = Vec::new();
    for r in requests {
        let kind = r["type"].as_str().or(r["media"]["mediaType"].as_str()).unwrap_or("").to_string();
        let tmdb_id = r["media"]["tmdbId"].as_i64().unwrap_or(0);
        if (kind != "movie" && kind != "tv") || tmdb_id <= 0 {
            lines.push(Line {
                title: format!("Request {}", r["id"]),
                action: "problem",
                detail: "Seerr doesn't say which title this request is for; it is skipped.".into(),
            });
            continue;
        }
        let asker = &r["requestedBy"];
        let Some(acc) = match_account(&st.db, asker)? else {
            lines.push(Line {
                title: format!("{} {tmdb_id}", if kind == "movie" { "Movie" } else { "Show" }),
                action: "problem",
                detail: format!("Asked for by {}, who has no Marquee account; skipped.", who(asker)),
            });
            continue;
        };
        let decider = match r["modifiedBy"].is_object() {
            true => match_account(&st.db, &r["modifiedBy"])?.map(|a| a.i("id")),
            false => None,
        };
        let (f, known) = facts_or_placeholder(st, &kind, tmdb_id).await;
        let mut seasons: Vec<i64> = r["seasons"].as_array().into_iter().flatten().filter_map(|s| s["seasonNumber"].as_i64()).collect();
        seasons.sort_unstable();
        seasons.dedup();
        let created = r["createdAt"].as_str().unwrap_or("").to_string();
        let updated = r["updatedAt"].as_str().filter(|s| !s.is_empty()).unwrap_or(&created).to_string();
        let title = if f.year > 0 { format!("{} ({})", f.title, f.year) } else { f.title.clone() };
        let user = acc.i("id");
        let key = (kind.clone(), tmdb_id, user, seasons.clone(), created.clone());
        let existing = media_by_tmdb(&st.db, &kind, tmdb_id)?;
        let dup = planned.contains(&key)
            || match &existing {
                Some(m) => already_in(&st.db, m.id, user, &seasons, &created)?,
                None => false,
            };
        if dup {
            lines.push(Line { title, action: "skip", detail: format!("{}'s request already moved in.", acc.s("username")) });
            continue;
        }
        planned.push(key);
        // where it stands: Seerr's status, unless the library already has everything it asked for
        let probe = existing.clone().unwrap_or(Media { kind: kind.clone(), tmdb_id, ..Media::default() });
        let a = availability(&st.db, &probe)?;
        let fake = super::Request { kind: kind.clone(), seasons: seasons.clone(), ..super::Request::default() };
        let there = super::fulfilled(&fake, &a);
        let in_library = if kind == "movie" {
            crate::movies::by_tmdb(&st.db, tmdb_id)?.is_some()
        } else {
            crate::series::by_tmdb(&st.db, tmdb_id)?.is_some()
        };
        let seerr_status = r["status"].as_i64().unwrap_or(status::PENDING);
        let (st_new, message, words) = if there {
            (status::COMPLETED, String::new(), "available: it's in the library".to_string())
        } else {
            match seerr_status {
                status::PENDING => (status::PENDING, String::new(), "waiting for approval".to_string()),
                status::DECLINED => (status::DECLINED, String::new(), "declined".to_string()),
                status::APPROVED | status::COMPLETED if in_library => (status::APPROVED, String::new(), "approved".to_string()),
                _ => (
                    status::FAILED,
                    "Moved in from Seerr but not in the library yet. Press Retry to add it and look for it.".to_string(),
                    "not in the library yet: Retry adds it".to_string(),
                ),
            }
        };
        let what = if seasons.is_empty() {
            String::new()
        } else {
            format!(
                " (season{} {})",
                if seasons.len() == 1 { "" } else { "s" },
                seasons.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(", ")
            )
        };
        let mut detail = format!("{} asked for it{what}; {words}.", acc.s("username"));
        if !known {
            detail.push_str(" TMDB couldn't be asked, so the title fills in later.");
        }
        lines.push(Line { title, action: "add", detail });
        if !apply {
            continue;
        }
        let m = match existing {
            Some(m) if !known => m,
            _ => upsert_media(&st.db, &kind, tmdb_id, &f)?,
        };
        let tags: Vec<i64> = r["tags"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect();
        st.db.execute(
            "INSERT INTO requests (media_id, kind, status, is4k, seasons, requested_by, modified_by, profile_id, root_folder,
                language_profile_id, tags, is_auto, message, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            p![
                m.id,
                kind,
                st_new,
                r["is4k"].as_bool().unwrap_or(false) as i64,
                serde_json::to_string(&seasons).unwrap_or_else(|_| "[]".into()),
                user,
                decider,
                r["profileId"].as_i64(),
                r["rootFolder"].as_str().filter(|s| !s.is_empty()),
                r["languageProfileId"].as_i64(),
                serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()),
                r["isAutoRequest"].as_bool().unwrap_or(false) as i64,
                message,
                created,
                updated
            ],
        )?;
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_limits_read_like_seerrs() {
        assert_eq!(api_base("http://seerr:5055/").unwrap(), "http://seerr:5055/api/v1");
        assert_eq!(api_base("https://req.example.com/api/v1").unwrap(), "https://req.example.com/api/v1");
        assert!(api_base("seerr:5055").is_err());
        let sam = json!({ "permissions": 32 | 256, "movieQuotaLimit": 5, "movieQuotaDays": 7, "tvQuotaLimit": null });
        assert_eq!(
            limits(&sam),
            vec![("movie_quota_limit", 5), ("movie_quota_days", 7), ("auto_approve_movies", 1), ("auto_approve_tv", 0)]
        );
        assert!(limits(&json!({ "permissions": 2 })).is_empty(), "admins have no limits");
        assert_eq!(who(&json!({ "displayName": "", "username": "sam" })), "sam");
    }
}
