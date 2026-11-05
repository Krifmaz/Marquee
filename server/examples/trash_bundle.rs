//! Makes the TRaSH Guides snapshot built into the server, `data/trash/bundle.json`, from a local clone of
//! <https://github.com/TRaSH-Guides/Guides>:
//!
//! ```text
//! git clone --depth 1 https://github.com/TRaSH-Guides/Guides.git /tmp/trash-guides
//! cd server && cargo run --example trash_bundle -- /tmp/trash-guides
//! ```
//!
//! The server's sync keeps the snapshot up to date after that; a new built-in one only makes the first start of
//! a fresh install closer to the newest guides.

use server::trash::snapshot::{wanted, File, Snapshot};
use std::process::Command;

/// Runs git in `repo` with `args` and returns what it printed.
fn git(repo: &str, args: &[&str]) -> String {
    let out = Command::new("git").env("TZ", "UTC").arg("-C").arg(repo).args(args).output().expect("git can't be run");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn main() {
    let repo = std::env::args().nth(1).expect("Give the path of a TRaSH Guides clone");
    let head = git(&repo, &["log", "-1", "--date=format-local:%Y-%m-%dT%H:%M:%SZ", "--format=%H %cd"]);
    let (commit, date) = head.trim().split_once(' ').expect("a commit and its date");
    let mut snap = Snapshot { commit: commit.into(), date: date.into(), ..Snapshot::default() };
    // "<mode> blob <sha>\t<path>"
    for line in git(&repo, &["ls-tree", "-r", "HEAD", "docs/json"]).lines() {
        let Some((meta, path)) = line.split_once('\t') else { continue };
        let Some(key) = wanted(path) else { continue };
        let sha = meta.split_whitespace().nth(2).unwrap_or_default().to_string();
        let text = std::fs::read_to_string(std::path::Path::new(&repo).join(path)).expect("a data file can't be read");
        match serde_json::from_str(&text) {
            Ok(body) => {
                snap.files.insert(key, File { sha, body });
            }
            Err(e) => eprintln!("Skipped {path}: {e}"),
        }
    }
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/data/trash/bundle.json");
    std::fs::write(out, serde_json::to_string(&snap).expect("JSON")).expect("bundle.json can't be written");
    println!("{} files from {} ({}) → {out}", snap.files.len(), snap.commit, snap.date);
}
