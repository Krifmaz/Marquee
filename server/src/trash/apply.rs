//! Putting TRaSH's profiles, custom formats, sizes and naming into Marquee, and keeping them up to date without
//! ever overwriting a person's own edits.
//!
//! Every item Marquee made from TRaSH is remembered in `trash_managed` with two fingerprints: `applied` (the item
//! as Marquee last wrote it) and `upstream` (the newest TRaSH version seen). When TRaSH changes an item:
//!
//! - the item is as Marquee wrote it → the new version is written and the change listed as `updated`;
//! - someone edited it since → nothing is written; the change waits (`waiting`) until an admin takes it
//!   (`taken`) or keeps their own (`kept`);
//! - someone deleted it → it stays deleted.
//!
//! Each change lists what it changes in plain words (`lines`), so the TRaSH page can show what changed.

use super::convert::{self, app_of};
use super::formats::{self, format_kind};
use super::snapshot::Snapshot;
use super::Choices;
use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Radarr's stock quality list, to tell stock sizes from a person's own.
const STOCK_MOVIE_SIZES: &str = include_str!("../../data/radarr/qualitydefinition.json");

/// Sonarr's stock quality list.
const STOCK_TV_SIZES: &str = include_str!("../../data/sonarr/qualitydefinition.json");

/// The kinds of item Marquee takes from TRaSH.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// a custom format (`custom_formats`)
    Format,
    /// a quality profile (`quality_profiles`)
    Profile,
    /// a kind's quality sizes (`quality_definitions`)
    Sizes,
    /// a kind's file and folder naming (`trash_naming`)
    Naming,
}

impl Item {
    /// The name kept in the tables.
    pub fn as_str(self) -> &'static str {
        match self {
            Item::Format => "format",
            Item::Profile => "profile",
            Item::Sizes => "sizes",
            Item::Naming => "naming",
        }
    }

    /// The item for a stored name.
    pub fn parse(s: &str) -> Option<Item> {
        match s {
            "format" => Some(Item::Format),
            "profile" => Some(Item::Profile),
            "sizes" => Some(Item::Sizes),
            "naming" => Some(Item::Naming),
            _ => None,
        }
    }
}

/// A short fingerprint of a JSON value that doesn't depend on the order of its keys.
pub fn fingerprint(v: &Value) -> String {
    /// The value as text with every object's keys sorted.
    fn canon(v: &Value, out: &mut String) {
        match v {
            Value::Object(o) => {
                let mut keys: Vec<&String> = o.keys().collect();
                keys.sort();
                out.push('{');
                for k in keys {
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push(':');
                    canon(&o[k], out);
                    out.push(',');
                }
                out.push('}');
            }
            Value::Array(a) => {
                out.push('[');
                for x in a {
                    canon(x, out);
                    out.push(',');
                }
                out.push(']');
            }
            // 1 and 1.0 are the same size
            Value::Number(n) => out.push_str(&n.as_f64().map(|f| f.to_string()).unwrap_or_else(|| n.to_string())),
            other => out.push_str(&other.to_string()),
        }
    }
    let mut s = String::new();
    canon(v, &mut s);
    hex::encode(&Sha256::digest(s.as_bytes())[..12])
}

/// A body without its `id`.
fn no_id(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.remove("id");
    }
    v
}

/// The quality names of a kind's list.
fn quality_names(db: &SqliteDb, kind: &str) -> Result<Vec<String>> {
    Ok(crate::quality::definitions(db, kind)?.iter().filter_map(|d| d["quality"]["name"].as_str().map(String::from)).collect())
}

/// A kind's sizes now, for the qualities TRaSH lists in `desired`, in the same shape.
fn current_sizes(db: &SqliteDb, kind: &str, desired: &Value) -> Result<Value> {
    let defs = crate::quality::definitions(db, kind)?;
    Ok(sizes_of(&defs, desired))
}

/// `defs`' sizes for the qualities in `desired`.
fn sizes_of(defs: &[Value], desired: &Value) -> Value {
    let rows: Vec<Value> = desired
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|want| {
            let name = want["quality"].as_str()?;
            let d = defs.iter().find(|d| d["quality"]["name"].as_str() == Some(name))?;
            Some(json!({ "quality": name, "min": d["minSize"], "preferred": d["preferredSize"], "max": d["maxSize"] }))
        })
        .collect();
    json!(rows)
}

/// The item as it is now in Marquee (no id), `None` when it is gone.
pub fn read(db: &SqliteDb, item: Item, kind: &str, local: i64, desired: &Value) -> Result<Option<Value>> {
    Ok(match item {
        Item::Format => formats::one(db, local)?.map(no_id),
        Item::Profile => crate::quality::profile(db, local)?.map(no_id),
        Item::Sizes => Some(current_sizes(db, kind, desired)?),
        Item::Naming => super::stored_naming(db, kind)?,
    })
}

/// Writes `body` as the item (`local`: the one to replace, `None` to add); the item's id (0 for sizes and naming).
pub fn write(db: &SqliteDb, item: Item, kind: &str, local: Option<i64>, body: &Value) -> Result<i64> {
    match item {
        Item::Format => formats::save(db, kind, local, body),
        Item::Profile => match local {
            Some(id) => crate::quality::update_profile(db, id, body.clone()).map(|_| id),
            None => crate::quality::create_profile(db, kind, body.clone()).map(|v| v["id"].as_i64().unwrap_or(0)),
        },
        Item::Sizes => {
            for d in crate::quality::definitions(db, kind)? {
                let name = d["quality"]["name"].as_str().unwrap_or("");
                let Some(row) = body.as_array().into_iter().flatten().find(|r| r["quality"].as_str() == Some(name)) else { continue };
                let mut nd = d.clone();
                nd["minSize"] = row["min"].clone();
                nd["preferredSize"] = row["preferred"].clone();
                nd["maxSize"] = row["max"].clone();
                let id = d["id"].as_i64().unwrap_or(0);
                crate::quality::update_definition(db, id, no_id(nd))?;
            }
            Ok(0)
        }
        Item::Naming => {
            db.execute(
                "INSERT INTO trash_naming (kind, body) VALUES (?,?) ON CONFLICT(kind) DO UPDATE SET body=excluded.body",
                p![kind, no_id(body.clone()).to_string()],
            )?;
            Ok(0)
        }
    }
}

/// The id of an item of `kind` named `name` that Marquee doesn't manage yet (same name, made by hand or by an
/// import), `None` when there is none.
fn by_name(db: &SqliteDb, item: Item, kind: &str, name: &str) -> Result<Option<i64>> {
    Ok(match item {
        Item::Format => formats::id_by_name(db, kind, name)?,
        Item::Profile => db.one("SELECT id FROM quality_profiles WHERE kind=? AND name=?", p![kind, name])?.map(|r| r.i("id")),
        Item::Sizes | Item::Naming => None,
    })
}

/// The stock state of sizes or naming (what Marquee had before TRaSH), to tell it from a person's own edits.
fn stock(item: Item, kind: &str, desired: &Value) -> Option<Value> {
    match item {
        Item::Sizes => {
            let defs: Vec<Value> = serde_json::from_str(if kind == "movie" { STOCK_MOVIE_SIZES } else { STOCK_TV_SIZES }).ok()?;
            Some(sizes_of(&defs, desired))
        }
        Item::Naming => None,
        _ => None,
    }
}

/// What a change does, in plain words (`before` is `None` for something new).
pub fn describe(item: Item, before: Option<&Value>, after: &Value) -> Vec<String> {
    let Some(before) = before else { return vec!["New from TRaSH Guides".into()] };
    let mut out = Vec::new();
    match item {
        Item::Format => {
            if before["specifications"] != after["specifications"] {
                out.push("Its matching rules changed".into());
            }
            if before["includeCustomFormatWhenRenaming"] != after["includeCustomFormatWhenRenaming"] {
                out.push("Whether its name goes into file names changed".into());
            }
        }
        Item::Profile => {
            let scores = |v: &Value| -> Vec<(String, i64)> {
                v["formatItems"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|f| (f["name"].as_str().unwrap_or("").to_string(), f["score"].as_i64().unwrap_or(0)))
                    .collect()
            };
            let (old, new) = (scores(before), scores(after));
            for (name, s) in &new {
                match old.iter().find(|(n, _)| n == name) {
                    None => out.push(format!("Adds {name} ({s})")),
                    Some((_, o)) if o != s => out.push(format!("{name}: {o} → {s}")),
                    _ => {}
                }
            }
            for (name, _) in old.iter().filter(|(n, _)| !new.iter().any(|(m, _)| m == n)) {
                out.push(format!("Drops {name}"));
            }
            let allowed = |v: &Value| -> Vec<String> {
                let mut names = Vec::new();
                for i in v["items"].as_array().into_iter().flatten().filter(|i| i["allowed"] == true) {
                    match i["items"].as_array().filter(|s| !s.is_empty()) {
                        Some(subs) => names.extend(subs.iter().filter_map(|s| s["quality"]["name"].as_str().map(String::from))),
                        None => names.extend(i["quality"]["name"].as_str().map(String::from)),
                    }
                }
                names
            };
            let (a, b) = (allowed(before), allowed(after));
            for q in b.iter().filter(|q| !a.contains(q)) {
                out.push(format!("Allows {q}"));
            }
            for q in a.iter().filter(|q| !b.contains(q)) {
                out.push(format!("No longer allows {q}"));
            }
            if before["cutoff"] != after["cutoff"] {
                out.push("Upgrades until a different quality".into());
            }
            for (key, label) in [
                ("minFormatScore", "Minimum score"),
                ("cutoffFormatScore", "Upgrade-until score"),
                ("minUpgradeFormatScore", "Smallest score step"),
            ] {
                if before[key] != after[key] {
                    out.push(format!("{label}: {} → {}", before[key], after[key]));
                }
            }
            if before["upgradeAllowed"] != after["upgradeAllowed"] {
                out.push(if after["upgradeAllowed"] == true { "Upgrades are on" } else { "Upgrades are off" }.into());
            }
            if before["language"] != after["language"] {
                out.push(format!("Language: {}", after["language"]["name"].as_str().unwrap_or("?")));
            }
        }
        Item::Sizes => {
            let num = |v: &Value| v.as_f64().map(|f| format!("{f}")).unwrap_or_else(|| "no limit".into());
            for row in after.as_array().into_iter().flatten() {
                let q = row["quality"].as_str().unwrap_or("");
                let old = before.as_array().into_iter().flatten().find(|r| r["quality"].as_str() == Some(q));
                let same = old.is_some_and(|o| fingerprint(o) == fingerprint(row));
                if !same {
                    let was = old.map(|o| format!("{}–{}", num(&o["min"]), num(&o["max"]))).unwrap_or_else(|| "—".into());
                    out.push(format!("{q}: {was} → {}–{} MB a minute", num(&row["min"]), num(&row["max"])));
                }
            }
        }
        Item::Naming => {
            if let Some(o) = after.as_object() {
                for (k, v) in o {
                    if before[k] != *v {
                        out.push(match v {
                            Value::String(s) => format!("{k}: {s}"),
                            other => format!("{k}: {other}"),
                        });
                    }
                }
            }
        }
    }
    if out.is_empty() {
        out.push("Small changes".into());
    }
    out
}

/// Records a change in the list the TRaSH page shows; a new waiting change replaces an older one of the same item.
#[allow(clippy::too_many_arguments)]
fn log(
    db: &SqliteDb,
    commit: &str,
    item: Item,
    kind: &str,
    trash_id: &str,
    local: i64,
    name: &str,
    status: &str,
    lines: &[String],
    proposed: Option<&Value>,
) -> Result<()> {
    if status == "waiting" {
        db.execute(
            "UPDATE trash_changes SET status='replaced' WHERE status='waiting' AND kind=? AND item=? AND trash_id=?",
            p![kind, item.as_str(), trash_id],
        )?;
    }
    db.execute(
        "INSERT INTO trash_changes (at, commit_id, kind, item, trash_id, local_id, name, status, lines, proposed) VALUES (?,?,?,?,?,?,?,?,?,?)",
        p![crate::movies::now_iso(), commit, kind, item.as_str(), trash_id, local, name, status, json!(lines).to_string(), proposed.map(|v| v.to_string())],
    )?;
    Ok(())
}

/// Puts TRaSH's version (`desired`) of one item into Marquee, following the rules at the top; the item's id in
/// Marquee when it exists.
#[allow(clippy::too_many_arguments)]
fn reconcile(db: &SqliteDb, commit: &str, item: Item, kind: &str, trash_id: &str, name: &str, desired: &Value) -> Result<Option<i64>> {
    let up = fingerprint(desired);
    let managed = db.one("SELECT * FROM trash_managed WHERE kind=? AND item=? AND trash_id=?", p![kind, item.as_str(), trash_id])?;
    let Some(m) = managed else {
        let existing = match item {
            Item::Sizes | Item::Naming => Some(0),
            _ => by_name(db, item, kind, name)?,
        };
        let remember = |local: i64, applied: &str| {
            db.execute(
                "INSERT INTO trash_managed (kind, item, trash_id, local_id, applied, upstream) VALUES (?,?,?,?,?,?)",
                p![kind, item.as_str(), trash_id, local, applied, up],
            )
        };
        let Some(local) = existing else {
            let local = write(db, item, kind, None, desired)?;
            remember(local, &up)?;
            log(db, commit, item, kind, trash_id, local, name, "added", &describe(item, None, desired), None)?;
            return Ok(Some(local));
        };
        let now = read(db, item, kind, local, desired)?;
        let unchanged_stock = match (&now, stock(item, kind, desired)) {
            (None, _) => true,
            (Some(n), Some(s)) => fingerprint(n) == fingerprint(&s),
            _ => false,
        };
        if now.as_ref().is_some_and(|n| fingerprint(n) == up) {
            remember(local, &up)?;
        } else if unchanged_stock {
            write(db, item, kind, Some(local).filter(|l| *l > 0), desired)?;
            remember(local, &up)?;
            log(db, commit, item, kind, trash_id, local, name, "added", &describe(item, now.as_ref(), desired), None)?;
        } else {
            // something of the same name that isn't Marquee's: ask before replacing it
            remember(local, "")?;
            log(db, commit, item, kind, trash_id, local, name, "waiting", &describe(item, now.as_ref(), desired), Some(desired))?;
        }
        return Ok(Some(local));
    };
    let local = m.i("local_id");
    let now = read(db, item, kind, local, desired)?;
    if m.s("upstream") == up {
        return Ok(now.map(|_| local));
    }
    db.execute("UPDATE trash_managed SET upstream=? WHERE kind=? AND item=? AND trash_id=?", p![up, kind, item.as_str(), trash_id])?;
    let Some(now) = now else { return Ok(None) };
    let lines = describe(item, Some(&now), desired);
    if fingerprint(&now) == m.s("applied") {
        write(db, item, kind, Some(local).filter(|l| *l > 0), desired)?;
        db.execute("UPDATE trash_managed SET applied=? WHERE kind=? AND item=? AND trash_id=?", p![up, kind, item.as_str(), trash_id])?;
        log(db, commit, item, kind, trash_id, local, name, "updated", &lines, None)?;
    } else {
        log(db, commit, item, kind, trash_id, local, name, "waiting", &lines, Some(desired))?;
    }
    Ok(Some(local))
}

/// The TRaSH profiles the choices pick: (media kind, profile file, whether it is the kind's default).
pub fn picks(c: &Choices) -> Vec<(&'static str, String, bool)> {
    let mut out = Vec::new();
    for (kind, pick) in [("movie", &c.movie), ("tv", &c.tv)] {
        let app = app_of(kind);
        let file = |res: &str| -> String {
            let hd = res == "1080p";
            let name = match (kind, pick.source.as_str()) {
                ("movie", "bluray") => if hd { "hd-bluray-web" } else { "uhd-bluray-web" }.to_string(),
                (_, "remux") => format!("remux-web-{res}"),
                _ => format!("web-{res}"),
            };
            format!("{app}/quality-profiles/{name}.json")
        };
        match pick.resolution.as_str() {
            "2160p" => out.push((kind, file("2160p"), true)),
            "both" => {
                out.push((kind, file("1080p"), true));
                out.push((kind, file("2160p"), false));
            }
            _ => out.push((kind, file("1080p"), true)),
        }
    }
    if c.anime {
        out.push(("anime", "sonarr/quality-profiles/anime-remux-1080p.json".to_string(), true));
        out.push(("movie", "radarr/quality-profiles/anime-remux-1080p.json".to_string(), false));
    }
    out
}

/// Puts everything the choices pick from `snap` into Marquee (see the rules at the top); how many changes it made
/// and how many wait for an admin.
pub fn apply(db: &SqliteDb, snap: &Snapshot, c: &Choices) -> Result<(usize, usize)> {
    let before = db.one("SELECT COALESCE(MAX(id), 0) n FROM trash_changes", p![])?.map(|r| r.i("n")).unwrap_or(0);
    let commit = snap.commit.as_str();
    let picks = picks(c);
    let profiles: Vec<(&str, &serde_json::Value, bool)> = picks
        .iter()
        .filter_map(|(kind, file, default)| match snap.file(file) {
            Some(p) => Some((*kind, p, *default)),
            None => {
                tracing::warn!("TRaSH Guides has no {file} any more; it is left out");
                None
            }
        })
        .collect();
    // custom formats first: profiles point at them
    let mut format_ids: Vec<(String, String, i64)> = Vec::new(); // (format kind, trash id, id in Marquee)
    for fk in ["movie", "tv"] {
        let app = app_of(fk);
        let mut wanted: Vec<String> = Vec::new();
        for (_, p, _) in profiles.iter().filter(|(k, _, _)| format_kind(k) == fk) {
            for id in convert::profile_formats(snap, app, p) {
                if !wanted.contains(&id) {
                    wanted.push(id);
                }
            }
        }
        for id in wanted {
            let Some(cf) = snap.format(app, &id) else { continue };
            let body = formats::from_trash(cf);
            let name = cf["name"].as_str().unwrap_or("");
            if let Some(local) = reconcile(db, commit, Item::Format, fk, &id, name, &body)? {
                format_ids.push((fk.to_string(), id, local));
            }
        }
        if let Some(file) = snap.folder(app, "naming").map(|(_, v)| v).next() {
            let body = convert::naming(fk, file, &c.server);
            reconcile(db, commit, Item::Naming, fk, "naming", if fk == "movie" { "Movie naming" } else { "Series naming" }, &body)?;
        }
    }
    let kinds: &[&str] = if c.anime { &["movie", "tv", "anime"] } else { &["movie", "tv"] };
    for kind in kinds {
        let Some(file) = snap.file(convert::size_file(kind)) else { continue };
        let body = convert::sizes(file, &quality_names(db, kind)?);
        let id = file["trash_id"].as_str().unwrap_or("sizes");
        reconcile(db, commit, Item::Sizes, kind, id, &format!("Quality sizes ({})", super::kind_label(kind)), &body)?;
    }
    let movie_langs: Vec<Value> = serde_json::from_str(include_str!("../../data/radarr/language.json")).unwrap_or_default();
    for (kind, p, default) in profiles {
        let app = app_of(kind);
        let fk = format_kind(kind);
        let set = p["trash_score_set"].as_str();
        let scored: Vec<(i64, String, i64)> = convert::profile_formats(snap, app, p)
            .iter()
            .filter_map(|id| {
                let local = format_ids.iter().find(|(k, t, _)| k == fk && t == id)?.2;
                let cf = snap.format(app, id)?;
                Some((local, cf["name"].as_str().unwrap_or("").to_string(), convert::score(cf, set)))
            })
            .collect();
        let qualities = crate::quality::definitions(db, kind)?;
        let body = convert::profile(p, &qualities, &scored, (kind == "movie").then_some(movie_langs.as_slice()));
        let tid = p["trash_id"].as_str().unwrap_or("");
        let local = reconcile(db, commit, Item::Profile, kind, tid, p["name"].as_str().unwrap_or(""), &body)?;
        if let (true, Some(local)) = (default, local) {
            super::set_meta(db, &format!("default:{kind}"), &local.to_string())?;
        }
    }
    let after: Vec<String> = db
        .query("SELECT status FROM trash_changes WHERE id>?", p![before])?
        .iter()
        .map(|r| r.s("status"))
        .filter(|s| s != "replaced")
        .collect();
    Ok((after.iter().filter(|s| *s != "waiting").count(), after.iter().filter(|s| *s == "waiting").count()))
}

/// Takes TRaSH's version of a waiting change (`take`) or keeps the person's own (`!take`).
pub fn decide(db: &SqliteDb, id: i64, take: bool) -> Result<Value> {
    let Some(r) = db.one("SELECT * FROM trash_changes WHERE id=?", p![id])? else {
        return Err(err(404, "There is no such change"));
    };
    if r.s("status") != "waiting" {
        return Err(err(409, "This change was already decided"));
    }
    let item = Item::parse(&r.s("item")).ok_or_else(|| err(500, "This change is of an unknown kind"))?;
    let kind = r.s("kind");
    if take {
        let body: Value = serde_json::from_str(&r.s("proposed")).map_err(|_| err(500, "This change can't be read"))?;
        let local = r.i("local_id");
        let exists = read(db, item, &kind, local, &body)?.is_some();
        let local = write(db, item, &kind, exists.then_some(local).filter(|l| *l > 0), &body)?;
        db.execute(
            "UPDATE trash_managed SET applied=?, local_id=? WHERE kind=? AND item=? AND trash_id=?",
            p![fingerprint(&body), local, kind, r.s("item"), r.s("trash_id")],
        )?;
    }
    db.execute("UPDATE trash_changes SET status=?, proposed=NULL WHERE id=?", p![if take { "taken" } else { "kept" }, id])?;
    Ok(json!({ "id": id, "status": if take { "taken" } else { "kept" } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_ignore_key_order_and_number_spelling() {
        assert_eq!(fingerprint(&json!({"a": 1, "b": [1, 2]})), fingerprint(&json!({"b": [1, 2], "a": 1.0})));
        assert_ne!(fingerprint(&json!({"a": 1})), fingerprint(&json!({"a": 2})));
    }

    #[test]
    fn choices_pick_trash_profiles() {
        let c = Choices::default();
        let files: Vec<String> = picks(&c).into_iter().map(|(k, f, d)| format!("{k} {f} {d}")).collect();
        assert_eq!(
            files,
            [
                "movie radarr/quality-profiles/hd-bluray-web.json true",
                "tv sonarr/quality-profiles/web-1080p.json true",
                "anime sonarr/quality-profiles/anime-remux-1080p.json true",
                "movie radarr/quality-profiles/anime-remux-1080p.json false",
            ]
        );
        let mut c = Choices::default();
        c.movie.resolution = "both".into();
        c.movie.source = "remux".into();
        c.tv.resolution = "2160p".into();
        c.anime = false;
        let files: Vec<String> = picks(&c).into_iter().map(|(_, f, d)| format!("{f} {d}")).collect();
        assert_eq!(
            files,
            [
                "radarr/quality-profiles/remux-web-1080p.json true",
                "radarr/quality-profiles/remux-web-2160p.json false",
                "sonarr/quality-profiles/web-2160p.json true"
            ]
        );
    }

    #[test]
    fn changes_are_described_in_words() {
        let a = json!({"formatItems": [{"name": "WEB Tier 01", "score": 1700}, {"name": "Old", "score": 5}], "items": [], "cutoff": 1});
        let b = json!({"formatItems": [{"name": "WEB Tier 01", "score": 1800}, {"name": "HDR", "score": 500}], "items": [], "cutoff": 2});
        let lines = describe(Item::Profile, Some(&a), &b);
        assert_eq!(lines, ["WEB Tier 01: 1700 → 1800", "Adds HDR (500)", "Drops Old", "Upgrades until a different quality"]);
        let s = describe(
            Item::Sizes,
            Some(&json!([{"quality": "HDTV-720p", "min": 10, "max": 1000}])),
            &json!([{"quality": "HDTV-720p", "min": 12, "max": 1000}]),
        );
        assert_eq!(s, ["HDTV-720p: 10–1000 → 12–1000 MB a minute"]);
    }
}
