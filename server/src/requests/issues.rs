//! Seerr's issues, ported from Seerr 3.5.1's `routes/issue.ts`, `routes/issueComment.ts` and `entity/Issue.ts`:
//! somebody reports a problem with a title (video, audio, subtitles, other), people talk about it in comments, and
//! it is resolved or opened again.
//!
//! ```text
//! GET    /api/v1/issue                       issues, newest first (MANAGE, VIEW or CREATE_ISSUES; your own unless
//!                                            you may see everybody's)
//! POST   /api/v1/issue                       report one, with its first comment (MANAGE or CREATE_ISSUES)
//! GET    /api/v1/issue/count                 how many of each type, open and closed (anybody signed in)
//! GET    /api/v1/issue/{id}                  one issue with its comments
//! POST   /api/v1/issue/{id}/comment          add a comment
//! POST   /api/v1/issue/{id}/{open|resolved}  open it again or resolve it
//! DELETE /api/v1/issue/{id}                  remove it
//! GET    /api/v1/issueComment/{id}           one comment
//! PUT    /api/v1/issueComment/{id}           change your own comment
//! DELETE /api/v1/issueComment/{id}           remove a comment
//! ```

use super::seerr::{has_any, media_json, perm, shown_user, Answer, Fail, Seerr};
use super::{media, stamp};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// Seerr's issue types (`IssueType`).
pub mod kind {
    /// the picture
    pub const VIDEO: i64 = 1;
    /// the sound
    pub const AUDIO: i64 = 2;
    /// the subtitles
    pub const SUBTITLES: i64 = 3;
    /// anything else
    pub const OTHER: i64 = 4;
}

/// Seerr's issue states (`IssueStatus`).
pub mod state {
    /// still to look at
    pub const OPEN: i64 = 1;
    /// dealt with
    pub const RESOLVED: i64 = 2;
}

/// The issues and comments tables.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_issues (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            issue_type INTEGER NOT NULL,
            status INTEGER NOT NULL DEFAULT 1,
            problem_season INTEGER NOT NULL DEFAULT 0,
            problem_episode INTEGER NOT NULL DEFAULT 0,
            media_id INTEGER NOT NULL,
            created_by INTEGER NOT NULL,
            modified_by INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS request_issue_comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            issue_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            message TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The issues tables can't be made: {}", e.detail))
}

/// The routes.
pub fn router() -> Router<AppState> {
    let v1 = Router::new()
        .route("/issue", get(list).post(report))
        .route("/issue/count", get(count))
        .route("/issue/{id}", get(one).delete(remove))
        .route("/issue/{id}/comment", post(comment))
        .route("/issue/{id}/{status}", post(set_status))
        .route("/issueComment/{id}", get(one_comment).put(change_comment).delete(remove_comment));
    Router::new().nest("/api/v1", v1)
}

/// The answer when an issue isn't there.
fn no_issue() -> Fail {
    Fail(err(404, "Issue not found."))
}

/// The answer when a comment isn't there.
fn no_comment() -> Fail {
    Fail(err(404, "Issue comment not found."))
}

/// Account `id` as the viewer may see it, or `null`.
fn person(db: &SqliteDb, id: Option<i64>, full: bool) -> kit_server_core::Result<Value> {
    match id {
        Some(id) => match db.one("SELECT * FROM users WHERE id=?", p![id])? {
            Some(u) => shown_user(db, &u, full),
            None => Ok(Value::Null),
        },
        None => Ok(Value::Null),
    }
}

/// A comment in Seerr's shape.
fn comment_json(db: &SqliteDb, c: &Row, full: bool) -> kit_server_core::Result<Value> {
    Ok(json!({
        "id": c.i("id"),
        "user": person(db, Some(c.i("user_id")), full)?,
        "message": c.s("message"),
        "createdAt": c.s("created_at"),
        "updatedAt": c.s("updated_at"),
    }))
}

/// An issue in Seerr's shape, with its title, the people and its comments.
fn issue_json(db: &SqliteDb, r: &Row, full: bool) -> kit_server_core::Result<Value> {
    let m = match media(db, r.i("media_id"))? {
        Some(m) => media_json(db, &m, false)?,
        None => Value::Null,
    };
    let comments = db.query("SELECT * FROM request_issue_comments WHERE issue_id=? ORDER BY id", p![r.i("id")])?;
    Ok(json!({
        "id": r.i("id"),
        "issueType": r.i("issue_type"),
        "status": r.i("status"),
        "problemSeason": r.i("problem_season"),
        "problemEpisode": r.i("problem_episode"),
        "media": m,
        "createdBy": person(db, Some(r.i("created_by")), full)?,
        "modifiedBy": person(db, r.get("modified_by").and_then(Value::as_i64), full)?,
        "comments": comments.iter().map(|c| comment_json(db, c, full)).collect::<kit_server_core::Result<Vec<_>>>()?,
        "createdAt": r.s("created_at"),
        "updatedAt": r.s("updated_at"),
    }))
}

/// Issue `id`, or 404.
fn find(db: &SqliteDb, id: i64) -> std::result::Result<Row, Fail> {
    db.one("SELECT * FROM request_issues WHERE id=?", p![id])?.ok_or_else(no_issue)
}

/// Whether the viewer sees full account details (email and settings).
fn full(s: &Seerr) -> bool {
    has_any(&s.0.row, &[perm::MANAGE_USERS])
}

/// What `GET /issue` takes.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    /// how many per page (10)
    pub take: Option<usize>,
    /// how many to skip
    pub skip: Option<usize>,
    /// only the issues of this account
    pub created_by: Option<i64>,
    /// `added` (default) or `modified`
    pub sort: Option<String>,
    /// `all` (default), `open` or `resolved`
    pub filter: Option<String>,
}

/// `GET /issue`.
async fn list(State(st): State<AppState>, s: Seerr, Query(q): Query<ListQuery>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::VIEW_ISSUES, perm::CREATE_ISSUES])?;
    let statuses = match q.filter.as_deref() {
        Some("open") => vec![state::OPEN],
        Some("resolved") => vec![state::RESOLVED],
        _ => vec![state::OPEN, state::RESOLVED],
    };
    let who = if !s.0.has(&[perm::MANAGE_ISSUES, perm::VIEW_ISSUES]) {
        if q.created_by.is_some_and(|c| c != s.0.id()) {
            return Err(Fail(err(403, "You do not have permission to view issues reported by other users")));
        }
        Some(s.0.id())
    } else {
        q.created_by
    };
    let order = if q.sort.as_deref() == Some("modified") { "updated_at" } else { "created_at" };
    let rows = st.db.query(&format!("SELECT * FROM request_issues ORDER BY {order} DESC, id DESC"), p![])?;
    let picked: Vec<&Row> =
        rows.iter().filter(|r| statuses.contains(&r.i("status")) && who.is_none_or(|w| r.i("created_by") == w)).collect();
    let take = q.take.filter(|t| *t > 0).unwrap_or(10).min(1000);
    let skip = q.skip.unwrap_or(0);
    let results =
        picked.iter().skip(skip).take(take).map(|r| issue_json(&st.db, r, full(&s))).collect::<kit_server_core::Result<Vec<_>>>()?;
    Ok(Json(json!({
        "pageInfo": { "pages": picked.len().div_ceil(take), "pageSize": take, "results": picked.len(), "page": skip.div_ceil(take) + 1 },
        "results": results,
    }))
    .into_response())
}

/// What `POST /issue` takes (Seerr's `IssueRequestBody`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// the title's media id (`mediaInfo.id`)
    pub media_id: i64,
    /// [`kind`]
    pub issue_type: i64,
    /// the first comment
    #[serde(default)]
    pub message: String,
    /// the season, for a series (0: all or not a series)
    #[serde(default)]
    pub problem_season: i64,
    /// the episode (0: all)
    #[serde(default)]
    pub problem_episode: i64,
    /// report it for somebody else (MANAGE_ISSUES)
    pub user_id: Option<i64>,
}

/// `POST /issue`: 201 with the new issue.
async fn report(State(st): State<AppState>, s: Seerr, Json(b): Json<Report>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::CREATE_ISSUES])?;
    if !(kind::VIDEO..=kind::OTHER).contains(&b.issue_type) {
        return Err(Fail(err(400, "Say what kind of problem it is: 1 video, 2 audio, 3 subtitles or 4 other.")));
    }
    if media(&st.db, b.media_id)?.is_none() {
        return Err(Fail(err(404, "Media does not exist.")));
    }
    let mut by = s.0.id();
    if let Some(u) = b.user_id.filter(|u| *u != s.0.id()) {
        if !s.0.has(&[perm::MANAGE_ISSUES]) {
            return Err(Fail(err(403, "You do not have permission to create an issue on behalf of another user.")));
        }
        if st.db.one("SELECT id FROM users WHERE id=?", p![u])?.is_none() {
            return Err(Fail(err(404, "Issue user not found")));
        }
        by = u;
    }
    let now = stamp();
    let id = st.db.execute(
        "INSERT INTO request_issues (issue_type, status, problem_season, problem_episode, media_id, created_by, created_at, updated_at)
            VALUES (?,?,?,?,?,?,?,?)",
        p![b.issue_type, state::OPEN, b.problem_season, b.problem_episode, b.media_id, by, now, now],
    )?;
    st.db.execute(
        "INSERT INTO request_issue_comments (issue_id, user_id, message, created_at, updated_at) VALUES (?,?,?,?,?)",
        p![id, by, b.message, now, now],
    )?;
    let r = find(&st.db, id)?;
    Ok((StatusCode::CREATED, Json(issue_json(&st.db, &r, full(&s))?)).into_response())
}

/// `GET /issue/count`.
async fn count(State(st): State<AppState>, _s: Seerr) -> Answer {
    let n = |sql: &str, v: i64| -> kit_server_core::Result<i64> {
        Ok(st.db.one(&format!("SELECT COUNT(*) AS n FROM request_issues WHERE {sql}=?"), p![v])?.map(|r| r.i("n")).unwrap_or(0))
    };
    let total = st.db.one("SELECT COUNT(*) AS n FROM request_issues", p![])?.map(|r| r.i("n")).unwrap_or(0);
    Ok(Json(json!({
        "total": total,
        "video": n("issue_type", kind::VIDEO)?,
        "audio": n("issue_type", kind::AUDIO)?,
        "subtitles": n("issue_type", kind::SUBTITLES)?,
        "others": n("issue_type", kind::OTHER)?,
        "open": n("status", state::OPEN)?,
        "closed": n("status", state::RESOLVED)?,
    }))
    .into_response())
}

/// `GET /issue/{id}`: your own, or anybody's with MANAGE or VIEW_ISSUES.
async fn one(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::VIEW_ISSUES, perm::CREATE_ISSUES])?;
    let r = find(&st.db, id)?;
    if r.i("created_by") != s.0.id() && !s.0.has(&[perm::MANAGE_ISSUES, perm::VIEW_ISSUES]) {
        return Err(Fail(err(403, "You do not have permission to view this issue.")));
    }
    Ok(Json(issue_json(&st.db, &r, full(&s))?).into_response())
}

/// A comment's text.
#[derive(Deserialize)]
pub struct Message {
    /// what was said
    #[serde(default)]
    pub message: String,
}

/// `POST /issue/{id}/comment`: on your own issue, or any with MANAGE_ISSUES.
async fn comment(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<Message>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::CREATE_ISSUES])?;
    let r = find(&st.db, id)?;
    if r.i("created_by") != s.0.id() && !s.0.has(&[perm::MANAGE_ISSUES]) {
        return Err(Fail(err(403, "You do not have permission to comment on this issue.")));
    }
    let now = stamp();
    st.db.execute(
        "INSERT INTO request_issue_comments (issue_id, user_id, message, created_at, updated_at) VALUES (?,?,?,?,?)",
        p![id, s.0.id(), b.message, now, now],
    )?;
    st.db.execute("UPDATE request_issues SET updated_at=? WHERE id=?", p![now, id])?;
    Ok(Json(issue_json(&st.db, &find(&st.db, id)?, full(&s))?).into_response())
}

/// `POST /issue/{id}/{open|resolved}`: your own issue, or any with MANAGE_ISSUES.
async fn set_status(State(st): State<AppState>, s: Seerr, Path((id, status)): Path<(i64, String)>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::CREATE_ISSUES])?;
    let r = find(&st.db, id)?;
    if !s.0.has(&[perm::MANAGE_ISSUES]) && r.i("created_by") != s.0.id() {
        return Err(Fail(err(401, "You do not have permission to modify this issue.")));
    }
    let to = match status.as_str() {
        "resolved" => state::RESOLVED,
        "open" => state::OPEN,
        _ => return Err(Fail(err(400, "You must provide a valid status"))),
    };
    st.db.execute("UPDATE request_issues SET status=?, modified_by=?, updated_at=? WHERE id=?", p![to, s.0.id(), stamp(), id])?;
    Ok(Json(issue_json(&st.db, &find(&st.db, id)?, full(&s))?).into_response())
}

/// `DELETE /issue/{id}`: with MANAGE_ISSUES, or your own while nobody has answered it.
async fn remove(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::CREATE_ISSUES])?;
    let r = find(&st.db, id)?;
    let comments = st.db.one("SELECT COUNT(*) AS n FROM request_issue_comments WHERE issue_id=?", p![id])?.map(|c| c.i("n")).unwrap_or(0);
    if !s.0.has(&[perm::MANAGE_ISSUES]) && (r.i("created_by") != s.0.id() || comments > 1) {
        return Err(Fail(err(401, "You do not have permission to delete this issue.")));
    }
    st.db.execute("DELETE FROM request_issue_comments WHERE issue_id=?", p![id])?;
    st.db.execute("DELETE FROM request_issues WHERE id=?", p![id])?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Comment `id`, or 404.
fn find_comment(db: &SqliteDb, id: i64) -> std::result::Result<Row, Fail> {
    db.one("SELECT * FROM request_issue_comments WHERE id=?", p![id])?.ok_or_else(no_comment)
}

/// `GET /issueComment/{id}`: your own, or any with MANAGE or VIEW_ISSUES.
async fn one_comment(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::VIEW_ISSUES, perm::CREATE_ISSUES])?;
    let c = find_comment(&st.db, id)?;
    if !s.0.has(&[perm::MANAGE_ISSUES, perm::VIEW_ISSUES]) && c.i("user_id") != s.0.id() {
        return Err(Fail(err(403, "You do not have permission to view this comment.")));
    }
    Ok(Json(comment_json(&st.db, &c, full(&s))?).into_response())
}

/// `PUT /issueComment/{id}`: only your own.
async fn change_comment(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>, Json(b): Json<Message>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::CREATE_ISSUES])?;
    let c = find_comment(&st.db, id)?;
    if c.i("user_id") != s.0.id() {
        return Err(Fail(err(403, "You can only edit your own comments.")));
    }
    st.db.execute("UPDATE request_issue_comments SET message=?, updated_at=? WHERE id=?", p![b.message, stamp(), id])?;
    Ok(Json(comment_json(&st.db, &find_comment(&st.db, id)?, full(&s))?).into_response())
}

/// `DELETE /issueComment/{id}`: your own, or any with MANAGE_ISSUES.
async fn remove_comment(State(st): State<AppState>, s: Seerr, Path(id): Path<i64>) -> Answer {
    s.needs(&[perm::MANAGE_ISSUES, perm::CREATE_ISSUES])?;
    let c = find_comment(&st.db, id)?;
    if !s.0.has(&[perm::MANAGE_ISSUES]) && c.i("user_id") != s.0.id() {
        return Err(Fail(err(403, "You do not have permission to delete this comment.")));
    }
    st.db.execute("DELETE FROM request_issue_comments WHERE id=?", p![id])?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
