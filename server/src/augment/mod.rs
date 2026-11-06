//! Data augmentation: what Sonarr learns about series from outside TheTVDB — the scene's other names and
//! seasons ([`scene`], from Sonarr's services and [`xem`]) and which shows are daily ([`daily`]). A port of
//! Sonarr's `DataAugmentation` at the commit in `UPSTREAM.lock`.

pub mod daily;
pub mod scene;
pub mod xem;

use crate::AppState;

/// How often the scene mappings are fetched again (Sonarr's `UpdateSceneMapping` task: every 3 hours).
pub const UPDATE_EVERY: std::time::Duration = std::time::Duration::from_secs(3 * 3600);

/// Makes the data augmentation tables.
pub fn migrate(db: &kit_server_core::SqliteDb) -> anyhow::Result<()> {
    scene::migrate(db)
}

/// Fetches the scene mappings again (Sonarr's `UpdateSceneMappingCommand`), in the background.
pub fn spawn_update(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move { st.scenes.update(&st.http).await });
}

/// A series was added: fetches the scene mappings when they weren't fetched since the server started (Sonarr's
/// `SeriesAddedEvent` and `SeriesImportedEvent` handlers).
pub fn series_added(st: &AppState) {
    if !st.scenes.updated_after_startup() {
        spawn_update(st);
    }
}

/// Runs in the background: fetches the scene mappings soon after start when they are older than 3 hours (or
/// were never fetched), then every 3 hours.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let stale = match scene::last_fetched(&st.db).and_then(|f| chrono::DateTime::parse_from_rfc3339(&f).ok()) {
            Some(at) => chrono::Utc::now().signed_duration_since(at).to_std().map(|d| d >= UPDATE_EVERY).unwrap_or(false),
            None => true,
        };
        if stale {
            st.scenes.update(&st.http).await;
        }
        loop {
            tokio::time::sleep(UPDATE_EVERY).await;
            st.scenes.update(&st.http).await;
        }
    });
}
