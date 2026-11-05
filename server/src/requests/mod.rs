//! Requests and the Seerr-compatible API (the requests track). docs/TRACKS.md says who owns this folder.
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
