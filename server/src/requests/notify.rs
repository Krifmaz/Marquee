//! The news about requests: a new request waits for approval, a request was approved, declined or didn't work,
//! and what was asked for is there. It goes to the places set in Settings → Request news: a Discord channel (a
//! webhook) and ntfy (a topic address, with an access token when the topic needs one). Web push needs the kit's
//! service worker (see docs/DECISIONS.md).
//!
//! Each account also gets its own news by mail, as in Seerr 3.5.1's `lib/notifications` (the email agent): the
//! person who asked hears when their request is approved, declined or there; people who manage requests hear
//! about new, automatically approved and failed requests. Each account picks which news it gets on its
//! notification settings page (`notificationTypes.email`, Seerr's bits in [`kind`]); without a choice it gets all.
//!
//! Sending happens in the background: a request never waits for, or fails because of, a notification. A failure
//! is written to the log in plain words.

use super::seerr::{perm, permissions};
use super::{Media, Request};
use crate::AppState;
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde_json::{json, Value};

/// Seerr's notification kinds (`Notification` in `lib/notifications/index.ts`), as bits.
pub mod kind {
    /// a new request waits for approval
    pub const MEDIA_PENDING: i64 = 2;
    /// a request was approved
    pub const MEDIA_APPROVED: i64 = 4;
    /// what was asked for is there
    pub const MEDIA_AVAILABLE: i64 = 8;
    /// approving a request didn't work
    pub const MEDIA_FAILED: i64 = 16;
    /// a test
    pub const TEST_NOTIFICATION: i64 = 32;
    /// a request was declined
    pub const MEDIA_DECLINED: i64 = 64;
    /// a request was approved automatically
    pub const MEDIA_AUTO_APPROVED: i64 = 128;
    /// an issue was reported
    pub const ISSUE_CREATED: i64 = 256;
    /// an issue has a new comment
    pub const ISSUE_COMMENT: i64 = 512;
    /// an issue was resolved
    pub const ISSUE_RESOLVED: i64 = 1024;
    /// an issue was opened again
    pub const ISSUE_REOPENED: i64 = 2048;
    /// a request was made automatically (watchlist)
    pub const MEDIA_AUTO_REQUESTED: i64 = 4096;
    /// every kind together (Seerr's `ALL_NOTIFICATIONS`)
    pub const ALL: i64 = 8190;
}

/// Seerr's `hasNotificationType`: whether the kinds `kinds` are among the chosen bits `chosen` (a test always is;
/// no kinds at all always are).
pub fn has_type(kinds: i64, chosen: i64) -> bool {
    kinds == 0 || (chosen | kind::TEST_NOTIFICATION) & kinds != 0
}

/// Seerr's `getAdminPermission`: the permission that gets the admin news of kind `k`.
pub fn admin_permission(k: i64) -> i64 {
    match k {
        kind::MEDIA_PENDING
        | kind::MEDIA_APPROVED
        | kind::MEDIA_AVAILABLE
        | kind::MEDIA_FAILED
        | kind::MEDIA_DECLINED
        | kind::MEDIA_AUTO_APPROVED => perm::MANAGE_REQUESTS,
        kind::ISSUE_CREATED | kind::ISSUE_COMMENT | kind::ISSUE_RESOLVED | kind::ISSUE_REOPENED => perm::MANAGE_ISSUES,
        _ => perm::ADMIN,
    }
}

/// What happened to a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// a new request waits for an admin
    Pending,
    /// an admin approved it
    Approved,
    /// it was approved automatically when it was made
    AutoApproved,
    /// an admin declined it
    Declined,
    /// approving it didn't work
    Failed,
    /// what it asked for is there
    Available,
    /// a test from Settings
    Test,
}

/// One notification: a title, a sentence, and the poster's address ("" for none).
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// the first line
    pub title: String,
    /// the sentence under it
    pub body: String,
    /// the poster's full address
    pub image: String,
}

/// The words for `event` on request `r` for the title `m`, asked for by `asker`.
pub fn note(event: Event, r: &Request, m: &Media, asker: &str) -> Note {
    let name = if m.year > 0 { format!("{} ({})", m.title, m.year) } else { m.title.clone() };
    let what = if r.kind == "tv" && !r.seasons.is_empty() {
        let list: Vec<String> = r.seasons.iter().map(|s| s.to_string()).collect();
        let s = if r.seasons.len() == 1 { "season" } else { "seasons" };
        format!("{name}, {s} {}", list.join(", "))
    } else {
        name.clone()
    };
    let (title, body) = match event {
        Event::Pending => ("New request".to_string(), format!("{asker} asked for {what}. It waits for an admin to approve it.")),
        Event::Approved => ("Request approved".to_string(), format!("{what} was approved for {asker}. Marquee is looking for it now.")),
        Event::AutoApproved => (
            "Request approved automatically".to_string(),
            format!("{asker} asked for {what}, and it was approved automatically. Marquee is looking for it now."),
        ),
        Event::Declined => ("Request declined".to_string(), format!("The request for {what} from {asker} was declined.")),
        Event::Failed => (
            "Request didn't work".to_string(),
            format!("{what} for {asker} couldn't be added: {}. Open Requests and press Retry.", r.message.trim_end_matches('.')),
        ),
        Event::Available => ("Now available".to_string(), format!("{what} is ready to watch, {asker}.")),
        Event::Test => ("Test from Marquee".to_string(), "Request notifications reach this place.".to_string()),
    };
    Note { title, body, image: crate::movies::poster_url(&m.poster) }
}

impl Event {
    /// Seerr's kind for this event.
    pub fn kind(self) -> i64 {
        match self {
            Event::Pending => kind::MEDIA_PENDING,
            Event::Approved => kind::MEDIA_APPROVED,
            Event::AutoApproved => kind::MEDIA_AUTO_APPROVED,
            Event::Declined => kind::MEDIA_DECLINED,
            Event::Failed => kind::MEDIA_FAILED,
            Event::Available => kind::MEDIA_AVAILABLE,
            Event::Test => kind::TEST_NOTIFICATION,
        }
    }

    /// Whether the people who manage requests hear about it; otherwise the person who asked does (Seerr's
    /// `notifyAdmin` in `MediaRequest.sendNotification`).
    pub fn for_admins(self) -> bool {
        matches!(self, Event::Pending | Event::AutoApproved | Event::Failed)
    }
}

/// Sends the news about `r` in the background, to every place that is set up and to each account that gets it.
pub fn spawn(st: &AppState, event: Event, r: &Request, m: &Media, asker: &str) {
    let n = note(event, r, m, asker);
    let mails = match recipients(&st.db, event, r) {
        Ok(people) if crate::mail::enabled(st) => people.iter().filter_map(|u| mail_for(st, u, &n, r, m)).collect(),
        Ok(_) => vec![],
        Err(e) => {
            tracing::warn!("The accounts to tell about a request can't be read: {}", e.detail);
            vec![]
        }
    };
    let st = st.clone();
    tokio::spawn(async move {
        for (place, outcome) in send(&st, &n).await {
            if let Err(e) = outcome {
                tracing::warn!("The request notification to {place} didn't go out: {}", e.detail);
            }
        }
        for mail in mails {
            let to = mail.to.clone();
            if let Err(e) = crate::mail::send(&st, mail).await {
                tracing::warn!("The request notification mail to {to} didn't go out: {e}");
            }
        }
    });
}

/// The account's chosen mail news (`notificationTypes.email`); all of them when it never chose.
fn mail_kinds(db: &SqliteDb, user_id: i64) -> Result<i64> {
    let row = db.one("SELECT data FROM request_user_notify WHERE user_id=?", p![user_id])?;
    let data: Value = row.map(|r| serde_json::from_str(&r.s("data")).unwrap_or(Value::Null)).unwrap_or(Value::Null);
    Ok(data["notificationTypes"]["email"].as_i64().unwrap_or(kind::ALL))
}

/// The accounts that get mail about `event` on request `r`: the person who asked, or the people who manage
/// requests (not the one who asked when it was approved automatically), each only when they chose that news.
/// Turned-off accounts get nothing.
pub fn recipients(db: &SqliteDb, event: Event, r: &Request) -> Result<Vec<Row>> {
    let k = event.kind();
    if event == Event::Test {
        return Ok(vec![]);
    }
    let users = db.query("SELECT * FROM users WHERE disabled=0 ORDER BY id", p![])?;
    let mut out = vec![];
    for u in users {
        let id = u.i("id");
        let wanted = if event.for_admins() {
            let p = permissions(&u);
            let allowed = p & perm::ADMIN != 0 || p & admin_permission(k) != 0;
            let own = event == Event::AutoApproved && id == r.modified_by.unwrap_or(r.requested_by);
            allowed && !own
        } else {
            id == r.requested_by
        };
        if wanted && has_type(k, mail_kinds(db, id)?) {
            out.push(u);
        }
    }
    Ok(out)
}

/// The mail for account `u` about note `n`, or `None` when the account has no mail address.
fn mail_for(st: &AppState, u: &Row, n: &Note, r: &Request, m: &Media) -> Option<crate::mail::Mail> {
    let to = u.s("email");
    if !to.contains('@') {
        return None;
    }
    let name = if m.year > 0 { format!("{} ({})", m.title, m.year) } else { m.title.clone() };
    let mut text = format!("Hello {},\n\n{}\n", u.s("username"), n.body);
    let url = crate::mail::public_url(st);
    if !url.is_empty() {
        let page = if r.kind == "tv" { "tv" } else { "movie" };
        text.push_str(&format!("\nOpen it: {url}/requests/{page}/{}\n", m.tmdb_id));
    }
    Some(crate::mail::Mail { to, subject: format!("{} – {}", n.title, name), text })
}

/// Sends `n` to every place that is set up; each place's name and how it went.
pub async fn send(st: &AppState, n: &Note) -> Vec<(&'static str, Result<()>)> {
    let mut out = Vec::new();
    let discord = st.settings.str("request_discord_webhook");
    if !discord.trim().is_empty() {
        out.push(("Discord", post(st, discord.trim(), discord_body(&st.app_name(), n), &[]).await));
    }
    let ntfy = st.settings.str("request_ntfy_url");
    if !ntfy.trim().is_empty() {
        let token = st.settings.str("request_ntfy_token");
        let mut headers = vec![("Title", n.title.clone()), ("Tags", "movie_camera".to_string())];
        if !n.image.is_empty() {
            headers.push(("Attach", n.image.clone()));
        }
        if !token.trim().is_empty() {
            headers.push(("Authorization", format!("Bearer {}", token.trim())));
        }
        out.push(("ntfy", post_text(st, ntfy.trim(), &n.body, &headers).await));
    }
    out
}

/// The body of a Discord webhook message for `n`.
pub fn discord_body(app: &str, n: &Note) -> Value {
    let mut embed = json!({ "title": n.title, "description": n.body, "color": 0x5865F2 });
    if !n.image.is_empty() {
        embed["thumbnail"] = json!({ "url": n.image });
    }
    json!({ "username": app, "embeds": [embed] })
}

/// POSTs JSON to `url`; a plain error when the place doesn't take it.
async fn post(st: &AppState, url: &str, body: Value, headers: &[(&str, String)]) -> Result<()> {
    let mut req = st.http.post(url).json(&body);
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    answer(req.send().await)
}

/// POSTs plain text to `url` (ntfy).
async fn post_text(st: &AppState, url: &str, body: &str, headers: &[(&str, String)]) -> Result<()> {
    let mut req = st.http.post(url).body(body.to_string());
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    answer(req.send().await)
}

/// `Ok` for a 2xx answer, else what went wrong in words.
fn answer(r: reqwest::Result<reqwest::Response>) -> Result<()> {
    let r = r.map_err(|e| err(502, format!("it can't be reached ({e})")))?;
    match r.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err(err(502, "it refused the address or token; check them in Settings → Request news")),
        404 => Err(err(502, "the address doesn't exist; check it in Settings → Request news")),
        s => Err(err(502, format!("it answered with an error ({s})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_say_what_happened_in_plain_words() {
        let m = Media { title: "Severance".into(), year: 2022, kind: "tv".into(), ..Media::default() };
        let r = Request { kind: "tv".into(), seasons: vec![1, 2], ..Request::default() };
        let n = note(Event::Pending, &r, &m, "sam");
        assert_eq!(n.title, "New request");
        assert_eq!(n.body, "sam asked for Severance (2022), seasons 1, 2. It waits for an admin to approve it.");
        let failed = Request { message: "Add a TV folder in Settings → Folders first".into(), ..r.clone() };
        assert!(note(Event::Failed, &failed, &m, "sam").body.contains("couldn't be added: Add a TV folder"));
        let b = discord_body("Marquee", &n);
        assert_eq!(b["embeds"][0]["title"], "New request");
        assert!(b["embeds"][0].get("thumbnail").is_none());
    }

    #[test]
    fn kinds_follow_seerrs_has_notification_type() {
        assert!(has_type(kind::MEDIA_APPROVED, kind::ALL));
        assert!(!has_type(kind::MEDIA_APPROVED, 0), "nothing chosen");
        assert!(has_type(kind::TEST_NOTIFICATION, 0), "a test always goes");
        assert!(has_type(0, 0));
        assert!(has_type(kind::MEDIA_DECLINED, kind::MEDIA_DECLINED | kind::MEDIA_PENDING));
        assert_eq!(admin_permission(kind::MEDIA_FAILED), perm::MANAGE_REQUESTS);
        assert_eq!(admin_permission(kind::ISSUE_COMMENT), perm::MANAGE_ISSUES);
        assert_eq!(admin_permission(kind::MEDIA_AUTO_REQUESTED), perm::ADMIN);
        assert!(Event::AutoApproved.for_admins() && !Event::Available.for_admins());
    }
}
