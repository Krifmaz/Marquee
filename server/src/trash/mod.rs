//! TRaSH Guides profiles, custom formats and their sync (the TRaSH track). docs/TRACKS.md says who owns this folder.
//!
//! The track fills this module in. The server already calls [`migrate`] at start-up and merges [`router`], so
//! the track doesn't need to edit the shared files to add tables or routes.

use crate::AppState;
use axum::Router;
use kit_server_core::SqliteDb;

/// Makes or updates this track's tables (called once at start-up, after the core tables).
pub fn migrate(_db: &SqliteDb) -> anyhow::Result<()> {
    Ok(())
}

/// This track's routes (merged into the server's router).
pub fn router() -> Router<AppState> {
    Router::new()
}

/// Starts this track's background work, if it has any (called once when the server starts).
pub fn spawn(_st: AppState) {}

/// The custom formats the Radarr (`movie`) or Sonarr (`tv`) API lists at `customformat`, in their shape.
pub fn custom_formats(_st: &AppState, _kind: &str) -> Vec<serde_json::Value> {
    vec![]
}

/// The naming the Radarr (`movie`) or Sonarr (`tv`) API answers at `config/naming`, `None` for Marquee's own.
pub fn naming(_st: &AppState, _kind: &str) -> Option<serde_json::Value> {
    None
}

/// The profile a new item of `kind` gets, `None` to use the built-in rule (HD-1080p).
pub fn default_profile(_db: &SqliteDb, _kind: &str) -> Option<i64> {
    None
}
