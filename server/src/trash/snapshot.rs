//! A snapshot of the TRaSH Guides data files Marquee uses: the custom formats, custom format groups, quality
//! profiles, quality sizes and naming of Radarr and Sonarr, as they are in the TRaSH Guides repository at one
//! commit. The files are used as they are (TRaSH's own JSON), keyed by their path under `docs/json/`, for example
//! `radarr/cf/<file>.json`.
//!
//! Marquee ships with one snapshot built in (`data/trash/bundle.json`, made by `cargo run --example
//! trash_bundle`), so the first start needs no network. The sync replaces the snapshot kept in the database
//! with a newer one from GitHub.

use kit_server_core::{p, Db, RowExt, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The snapshot built into the server.
const BUNDLED: &str = include_str!("../../data/trash/bundle.json");

/// The folders under `docs/json/<app>/` Marquee reads.
pub const FOLDERS: [&str; 5] = ["cf", "cf-groups", "quality-profiles", "quality-size", "naming"];

/// One data file: its git blob id (to tell a changed file from the same one) and its JSON.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct File {
    /// the git blob id GitHub lists for the file
    pub sha: String,
    /// the file's JSON
    pub body: Value,
}

/// The data files at one commit of the TRaSH Guides repository.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    /// the commit the files are from
    pub commit: String,
    /// when that commit was made, RFC 3339
    pub date: String,
    /// path under `docs/json/` → file
    pub files: BTreeMap<String, File>,
}

/// The key a repository path is kept under (`docs/json/radarr/cf/x.json` → `radarr/cf/x.json`), or `None` for
/// a file Marquee doesn't use.
pub fn wanted(path: &str) -> Option<String> {
    let rest = path.strip_prefix("docs/json/")?;
    let mut parts = rest.split('/');
    let (app, folder, file) = (parts.next()?, parts.next()?, parts.next()?);
    let ok = parts.next().is_none()
        && matches!(app, "radarr" | "sonarr")
        && FOLDERS.contains(&folder)
        && file.ends_with(".json")
        && !file.starts_with('.');
    ok.then(|| rest.to_string())
}

impl Snapshot {
    /// The snapshot built into the server.
    pub fn bundled() -> Snapshot {
        serde_json::from_str(BUNDLED).unwrap_or_default()
    }

    /// The snapshot kept in the database, or the built-in one when there is none yet (or the built-in one is newer).
    pub fn load(db: &SqliteDb) -> Snapshot {
        let kept = db
            .one("SELECT value FROM trash_meta WHERE key='snapshot'", p![])
            .ok()
            .flatten()
            .and_then(|r| serde_json::from_str::<Snapshot>(&r.s("value")).ok());
        let bundled = Snapshot::bundled();
        let when = |s: &Snapshot| chrono::DateTime::parse_from_rfc3339(&s.date).map(|d| d.timestamp()).unwrap_or(0);
        match kept {
            Some(k) if when(&k) >= when(&bundled) => k,
            _ => bundled,
        }
    }

    /// Keeps this snapshot in the database.
    pub fn save(&self, db: &SqliteDb) -> kit_server_core::Result<()> {
        let text = serde_json::to_string(self).unwrap_or_default();
        db.execute(
            "INSERT INTO trash_meta (key, value) VALUES ('snapshot', ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            p![text],
        )?;
        Ok(())
    }

    /// The file at `key` (`radarr/quality-profiles/web-1080p.json`).
    pub fn file(&self, key: &str) -> Option<&Value> {
        self.files.get(key).map(|f| &f.body)
    }

    /// Every file in `<app>/<folder>/`, with its key.
    pub fn folder<'a>(&'a self, app: &str, folder: &str) -> impl Iterator<Item = (&'a str, &'a Value)> + 'a {
        let prefix = format!("{app}/{folder}/");
        self.files.iter().filter(move |(k, _)| k.starts_with(&prefix)).map(|(k, f)| (k.as_str(), &f.body))
    }

    /// The custom format of `app` whose `trash_id` is `id`.
    pub fn format(&self, app: &str, id: &str) -> Option<&Value> {
        self.folder(app, "cf").map(|(_, v)| v).find(|v| v["trash_id"].as_str() == Some(id))
    }

    /// The quality profile of `app` whose `trash_id` is `id`.
    pub fn profile(&self, app: &str, id: &str) -> Option<&Value> {
        self.folder(app, "quality-profiles").map(|(_, v)| v).find(|v| v["trash_id"].as_str() == Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_data_files_marquee_reads_are_kept() {
        assert_eq!(wanted("docs/json/radarr/cf/x.json").as_deref(), Some("radarr/cf/x.json"));
        assert_eq!(wanted("docs/json/sonarr/quality-size/anime.json").as_deref(), Some("sonarr/quality-size/anime.json"));
        assert_eq!(wanted("docs/json/sonarr/dummy-files/a.json"), None);
        assert_eq!(wanted("docs/json/radarr/conflicts.json"), None);
        assert_eq!(wanted("docs/json/lidarr/cf/x.json"), None);
        assert_eq!(wanted("docs/Radarr/cf.md"), None);
    }

    #[test]
    fn the_built_in_snapshot_has_tv_movie_and_anime_data() {
        let s = Snapshot::bundled();
        assert_eq!(s.commit.len(), 40, "a commit id");
        for key in [
            "radarr/quality-profiles/hd-bluray-web.json",
            "radarr/quality-profiles/anime-remux-1080p.json",
            "sonarr/quality-profiles/web-1080p.json",
            "sonarr/quality-profiles/anime-remux-1080p.json",
            "radarr/quality-size/movie.json",
            "sonarr/quality-size/series.json",
            "sonarr/quality-size/anime.json",
            "radarr/naming/radarr-naming.json",
            "sonarr/naming/sonarr-naming.json",
        ] {
            assert!(s.file(key).is_some(), "{key} is missing from the built-in snapshot");
        }
        assert!(s.folder("sonarr", "cf").count() > 100);
    }
}
