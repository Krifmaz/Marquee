//! Tags, shared by movies and TV: short labels (`kids`, `4k`) that group items. Labels are kept lower case,
//! as Radarr and Sonarr keep them.

use kit_server_core::{err, p, Db, Result, RowExt, SqliteDb};
use serde_json::{json, Value};

/// The tag table.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS tags (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT UNIQUE NOT NULL);")
        .map_err(|e| anyhow::anyhow!("The tag table can't be made: {}", e.detail))
}

/// A label as it is kept: trimmed, lower case; 400 when empty.
fn clean(label: &str) -> Result<String> {
    let l = label.trim().to_lowercase();
    if l.is_empty() {
        return Err(err(400, "A tag needs a label"));
    }
    Ok(l)
}

/// Every tag as `{label, id}`.
pub fn all(db: &SqliteDb) -> Result<Vec<Value>> {
    Ok(db.query("SELECT * FROM tags ORDER BY label", p![])?.iter().map(|r| json!({ "label": r.s("label"), "id": r.i("id") })).collect())
}

/// The tag `id`.
pub fn find(db: &SqliteDb, id: i64) -> Result<Option<Value>> {
    Ok(db.one("SELECT * FROM tags WHERE id=?", p![id])?.map(|r| json!({ "label": r.s("label"), "id": r.i("id") })))
}

/// Adds a tag, or gives the one with that label.
pub fn create(db: &SqliteDb, label: &str) -> Result<Value> {
    let l = clean(label)?;
    if let Some(r) = db.one("SELECT * FROM tags WHERE label=?", p![l])? {
        return Ok(json!({ "label": r.s("label"), "id": r.i("id") }));
    }
    let id = db.execute("INSERT INTO tags (label) VALUES (?)", p![l])?;
    Ok(json!({ "label": l, "id": id }))
}

/// Renames the tag `id`.
pub fn rename(db: &SqliteDb, id: i64, label: &str) -> Result<Value> {
    let l = clean(label)?;
    if find(db, id)?.is_none() {
        return Err(err(404, "There is no such tag"));
    }
    db.execute("UPDATE tags SET label=? WHERE id=?", p![l, id])?;
    Ok(json!({ "label": l, "id": id }))
}

/// Removes the tag `id` (items that had it keep their other tags).
pub fn delete(db: &SqliteDb, id: i64) -> Result<()> {
    if find(db, id)?.is_none() {
        return Err(err(404, "There is no such tag"));
    }
    db.execute("DELETE FROM tags WHERE id=?", p![id])?;
    Ok(())
}
