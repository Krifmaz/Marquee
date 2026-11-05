//! Plex and Jellyfin, invites, sign-up and password reset (the media servers track, docs/TRACKS.md). The shell
//! asks [`page`] for every route it doesn't know and shows [`nav`]'s links; signed out, it asks
//! [`public_page`] first.
//!
//! | Where | What |
//! | --- | --- |
//! | `#/media` | Media access (admins): whether Jellyfin and Plex answer, scans, each account's servers, libraries and end date, linking existing Jellyfin users and Plex shares |
//! | `#/invites` | Invites (admins): invite links, new invite, send by mail, turn off, remove |
//! | `#/invite/<code>` | the public sign-up page of an invite |
//! | `#/reset/<token>` | the public "pick a new password" page of a reset mail |
//! | `#/forgot` | the public "send me a reset link" page |
//! | `#/jellyfin` | the public "sign in with Jellyfin" page |

mod access;
mod invites;
mod signup;

use leptos::prelude::*;
use serde_json::Value;

/// Under the sign-in form (core's pages.rs draws it): Plex, Jellyfin and "Forgot your password?".
pub use signup::SignInOptions;

/// This track's page for `path` (`#/<path>?<query>`), `None` when the path isn't one of its own.
pub fn page(path: &str, _query: &str, admin: bool) -> Option<AnyView> {
    match path {
        "/media" if admin => Some(view! { <access::MediaAccess/> }.into_any()),
        "/invites" if admin => Some(view! { <invites::Invites/> }.into_any()),
        _ => None,
    }
}

/// This track's sidebar links: (href, label).
pub fn nav(admin: bool) -> Vec<(&'static str, &'static str)> {
    if admin {
        vec![("/media", "Media access"), ("/invites", "Invites")]
    } else {
        vec![]
    }
}

/// What a signed-out address shows: an invite, a reset link, the forgot-password page or Jellyfin sign-in.
#[derive(Clone, Debug, PartialEq)]
pub enum Public {
    /// `#/invite/<code>`
    Invite(String),
    /// `#/reset/<token>`
    Reset(String),
    /// `#/forgot`
    Forgot,
    /// `#/jellyfin`
    Jellyfin,
}

impl Public {
    /// The public page of a `location.hash`, `None` for the sign-in page.
    pub fn parse(hash: &str) -> Option<Public> {
        let h = hash.trim_start_matches('#');
        let h = h.split('?').next().unwrap_or("");
        let clean = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>();
        if let Some(c) = h.strip_prefix("/invite/") {
            Some(Public::Invite(clean(c))).filter(|p| !matches!(p, Public::Invite(c) if c.is_empty()))
        } else if let Some(t) = h.strip_prefix("/reset/") {
            Some(Public::Reset(clean(t))).filter(|p| !matches!(p, Public::Reset(t) if t.is_empty()))
        } else if h == "/forgot" {
            Some(Public::Forgot)
        } else if h == "/jellyfin" {
            Some(Public::Jellyfin)
        } else {
            None
        }
    }
}

/// A page shown to people who aren't signed in (an invite or a password reset link), `None` for the sign-in page.
pub fn public_page(hash: &str) -> Option<AnyView> {
    Some(match Public::parse(hash)? {
        Public::Invite(code) => view! { <signup::SignUp code=code/> }.into_any(),
        Public::Reset(token) => view! { <signup::Reset token=token/> }.into_any(),
        Public::Forgot => view! { <signup::Forgot/> }.into_any(),
        Public::Jellyfin => view! { <signup::JellyfinSignIn/> }.into_any(),
    })
}

/// A string field of a JSON object, "" when missing.
pub(crate) fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A date in words from seconds since 1970: `5 Nov 2026` (UTC).
pub fn day_words(secs: f64) -> String {
    let days = (secs / 86400.0).floor() as i64;
    // Howard Hinnant's civil-from-days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {} {y}", months[(m - 1) as usize])
}

/// When something ends, in words: "Never", or "Ends 5 Nov 2026" / "Ended 5 Nov 2026".
pub fn ends_words(at: Option<f64>, now: f64) -> String {
    match at {
        None => "Never ends".into(),
        Some(t) if t <= now => format!("Ended {}", day_words(t)),
        Some(t) => format!("Ends {}", day_words(t)),
    }
}

/// Seconds since 1970 now, from the browser.
pub fn now_secs() -> f64 {
    kit_leptos::rt::now_ms() as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{day_words, ends_words, nav, Public};

    #[test]
    fn public_pages_come_from_the_hash() {
        assert_eq!(Public::parse("#/invite/Ab-9_x"), Some(Public::Invite("Ab-9_x".into())));
        assert_eq!(Public::parse("#/invite/a<b>"), Some(Public::Invite("ab".into())));
        assert_eq!(Public::parse("#/invite/"), None);
        assert_eq!(Public::parse("#/reset/tok"), Some(Public::Reset("tok".into())));
        assert_eq!(Public::parse("#/forgot"), Some(Public::Forgot));
        assert_eq!(Public::parse("#/jellyfin"), Some(Public::Jellyfin));
        assert_eq!(Public::parse("#/"), None);
        assert_eq!(Public::parse(""), None);
    }

    #[test]
    fn dates_read_plainly() {
        assert_eq!(day_words(0.0), "1 Jan 1970");
        assert_eq!(day_words(1_793_836_800.0), "5 Nov 2026");
        assert_eq!(day_words(951_782_400.0), "29 Feb 2000");
        assert_eq!(ends_words(None, 0.0), "Never ends");
        assert_eq!(ends_words(Some(1_793_836_800.0), 0.0), "Ends 5 Nov 2026");
        assert_eq!(ends_words(Some(1_793_836_800.0), 2e9), "Ended 5 Nov 2026");
    }

    #[test]
    fn only_admins_get_the_links() {
        assert!(nav(false).is_empty());
        assert_eq!(nav(true), vec![("/media", "Media access"), ("/invites", "Invites")]);
    }
}
