//! Plex and Jellyfin, and the accounts, invites, sign-up and password reset that replace jfa-go (the media servers track). Submodules (jellyfin, plex, account_sync, invites, recovery, expiry, external_login) go in this folder. docs/TRACKS.md says who owns this folder.
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
