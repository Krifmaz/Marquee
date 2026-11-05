//! The news about requests: a new request waits for approval, a request was approved, declined or didn't work,
//! and what was asked for is there. It goes to the places set in Settings → Request news: a Discord channel (a
//! webhook) and ntfy (a topic address, with an access token when the topic needs one). Email goes through
//! `crate::mail` once the media servers track adds sending; web push needs the kit's service worker (see
//! docs/DECISIONS.md).
//!
//! Sending happens in the background: a request never waits for, or fails because of, a notification. A failure
//! is written to the log in plain words.

use super::{Media, Request};
use crate::AppState;
use kit_server_core::{err, Result};
use serde_json::{json, Value};

/// What happened to a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// a new request waits for an admin
    Pending,
    /// an admin approved it, or it was approved automatically
    Approved,
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

/// Sends the news about `r` in the background, to every place that is set up.
pub fn spawn(st: &AppState, event: Event, r: &Request, m: &Media, asker: &str) {
    let n = note(event, r, m, asker);
    let st = st.clone();
    tokio::spawn(async move {
        for (place, outcome) in send(&st, &n).await {
            if let Err(e) = outcome {
                tracing::warn!("The request notification to {place} didn't go out: {}", e.detail);
            }
        }
    });
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
}
