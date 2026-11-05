//! Getting a newer snapshot of TRaSH's data files from GitHub: one call for the newest commit, one for its file
//! list, then only the files whose git blob id changed since the snapshot Marquee has.

use super::snapshot::{wanted, File, Snapshot};
use kit_server_core::{err, Result};
use serde_json::Value;
use std::future::Future;

/// The repository the data files come from.
pub const REPO: &str = "TRaSH-Guides/Guides";

/// The newest commit, its date (RFC 3339) and every file at it: (path in the repository, git blob id).
pub type Tree = (String, String, Vec<(String, String)>);

/// Where a snapshot's files come from (GitHub, or a stand-in in tests).
pub trait Source: Send + Sync {
    /// The newest commit and its date (RFC 3339), and every file at it: (path in the repository, git blob id).
    fn tree(&self) -> impl Future<Output = Result<Tree>> + Send;
    /// The text of the file at `path` at `commit`.
    fn file(&self, commit: &str, path: &str) -> impl Future<Output = Result<String>> + Send;
}

/// TRaSH Guides on GitHub: the API for the commit and file list, raw.githubusercontent.com for the files.
pub struct GitHub {
    /// the HTTP client (the server's)
    pub http: reqwest::Client,
    /// the branch to follow
    pub branch: String,
}

/// A GET that must answer 200, as text; the error says what failed in words.
async fn get(http: &reqwest::Client, url: &str) -> Result<String> {
    let r = http
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| err(502, format!("GitHub can't be reached: {e}")))?;
    let status = r.status();
    let text = r.text().await.map_err(|e| err(502, format!("GitHub's answer can't be read: {e}")))?;
    if !status.is_success() {
        let why = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or_default();
        return Err(err(502, format!("GitHub answered {status} for {url}. {why}").trim().to_string()));
    }
    Ok(text)
}

impl Source for GitHub {
    async fn tree(&self) -> Result<Tree> {
        let c: Value =
            serde_json::from_str(&get(&self.http, &format!("https://api.github.com/repos/{REPO}/commits/{}", self.branch)).await?)
                .map_err(|e| err(502, format!("GitHub's commit can't be read: {e}")))?;
        let commit = c["sha"].as_str().unwrap_or("").to_string();
        let date = c["commit"]["committer"]["date"].as_str().unwrap_or("").to_string();
        if commit.is_empty() {
            return Err(err(502, "GitHub didn't say which commit is the newest"));
        }
        let t: Value =
            serde_json::from_str(&get(&self.http, &format!("https://api.github.com/repos/{REPO}/git/trees/{commit}?recursive=1")).await?)
                .map_err(|e| err(502, format!("GitHub's file list can't be read: {e}")))?;
        if t["truncated"] == true {
            tracing::warn!("GitHub cut TRaSH's file list short; some files may be missing");
        }
        let files = t["tree"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["type"] == "blob")
            .filter_map(|e| Some((e["path"].as_str()?.to_string(), e["sha"].as_str()?.to_string())))
            .collect();
        Ok((commit, date, files))
    }

    async fn file(&self, commit: &str, path: &str) -> Result<String> {
        get(&self.http, &format!("https://raw.githubusercontent.com/{REPO}/{commit}/{path}")).await
    }
}

/// The snapshot at `src`'s newest commit, reusing `have`'s files that didn't change; `None` when `have` is
/// already that commit.
pub async fn fetch<S: Source>(src: &S, have: &Snapshot) -> Result<Option<Snapshot>> {
    let (commit, date, tree) = src.tree().await?;
    if commit == have.commit {
        return Ok(None);
    }
    let mut snap = Snapshot { commit: commit.clone(), date, ..Snapshot::default() };
    for (path, sha) in tree {
        let Some(key) = wanted(&path) else { continue };
        if let Some(f) = have.files.get(&key).filter(|f| f.sha == sha) {
            snap.files.insert(key, f.clone());
            continue;
        }
        let text = src.file(&commit, &path).await?;
        match serde_json::from_str::<Value>(&text) {
            Ok(body) => {
                snap.files.insert(key, File { sha, body });
            }
            // a broken file upstream: keep the last good one
            Err(e) => {
                tracing::warn!("TRaSH's {path} can't be read ({e}); the last good copy is kept");
                if let Some(f) = have.files.get(&key) {
                    snap.files.insert(key, f.clone());
                }
            }
        }
    }
    if snap.files.is_empty() {
        return Err(err(502, "TRaSH Guides' newest commit has none of the files Marquee uses"));
    }
    Ok(Some(snap))
}
