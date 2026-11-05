//! Port of Sonarr's and Radarr's `Localization` (`LocalizationService`, `LocalizationController`): the UI strings
//! the forked web apps load before they draw anything.
//!
//! Only English is served for now (the upstream default, `UILanguage` = English); the strings are upstream's
//! `Localization/Core/en.json` at the commits in `UPSTREAM.lock`, kept in `data/<app>/localization/`.
//!
//! ```text
//! GET /sonarr/api/v3/localization           → {"strings": {...}}  (Sonarr's shape)
//! GET /radarr/api/v3/localization           → {"Strings": {...}}  (Radarr serialises without camelCase)
//! GET /<app>/api/v3/localization/language   → {"identifier": "en"}
//! ```

use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use std::sync::LazyLock;

/// Sonarr's English strings.
const SONARR_EN: &str = include_str!("../data/sonarr/localization/en.json");
/// Radarr's English strings.
const RADARR_EN: &str = include_str!("../data/radarr/localization/en.json");

/// Sonarr's answer, built once: the file is wrapped as it is, so the keys keep upstream's order.
static SONARR_BODY: LazyLock<String> = LazyLock::new(|| format!("{{\"strings\":{}}}", SONARR_EN.trim()));
/// Radarr's answer.
static RADARR_BODY: LazyLock<String> = LazyLock::new(|| format!("{{\"Strings\":{}}}", RADARR_EN.trim()));

/// Which app's strings to serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    /// Sonarr
    Sonarr,
    /// Radarr
    Radarr,
}

/// The localization routes for `app`, to nest under its `/api/v3`.
pub fn routes<S: Clone + Send + Sync + 'static>(app: App) -> Router<S> {
    Router::new()
        .route(
            "/localization",
            get(move || async move {
                let body = match app {
                    App::Sonarr => SONARR_BODY.as_str(),
                    App::Radarr => RADARR_BODY.as_str(),
                };
                ([(header::CONTENT_TYPE, "application/json; charset=utf-8")], body).into_response()
            }),
        )
        // `GetLanguageIdentifier`: English has no country code, so the identifier is "en"
        .route("/localization/language", get(|| async { Json(json!({ "identifier": "en" })) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_valid_json_with_upstream_keys() {
        let s: serde_json::Value = serde_json::from_str(&SONARR_BODY).unwrap();
        assert_eq!(s["strings"]["Series"], "Series");
        let r: serde_json::Value = serde_json::from_str(&RADARR_BODY).unwrap();
        assert_eq!(r["Strings"]["Movies"], "Movies");
    }
}
