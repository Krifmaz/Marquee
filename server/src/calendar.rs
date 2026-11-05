//! `GET /api/calendar?start&end`: movies and episodes in one list for the phone app. The Android track owns this
//! file (docs/TRACKS.md); the server already merges [`router`].

use crate::AppState;
use axum::Router;

/// The calendar routes.
pub fn router() -> Router<AppState> {
    Router::new()
}
