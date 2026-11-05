//! Requests and discovery (the requests track). docs/TRACKS.md says who owns this file.
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
