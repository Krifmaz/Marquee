//! Plex and Jellyfin, invites, sign-up and password reset (the media servers track). Its pages go in web/src/media/. docs/TRACKS.md says who owns this file.
//!
//! The shell already asks [`page`] for every route it doesn't know and shows [`nav`]'s links, so the track
//! doesn't need to edit main.rs to add pages.

use leptos::prelude::*;

/// This track's page for `path` (`#/<path>?<query>`), `None` when the path isn't one of its own.
pub fn page(_path: &str, _query: &str, _admin: bool) -> Option<AnyView> {
    None
}

/// This track's sidebar links: (href, label), for example `("/requests", "Requests")`.
pub fn nav(_admin: bool) -> Vec<(&'static str, &'static str)> {
    vec![]
}

/// A page shown to people who aren't signed in (an invite or a password reset link), `None` for the sign-in page.
pub fn public_page(_hash: &str) -> Option<AnyView> {
    None
}
