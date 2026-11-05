//! What the Radarr- and Sonarr-compatible APIs share: the API key check, paged answers, validation errors in
//! their shape, commands, and the indexers Prowlarr pushes.
//!
//! Each API lives under its own prefix (`/radarr`, `/sonarr`), because both apps answer `/api/v3/queue`,
//! `/api/v3/calendar` and others with different things. A tool connects with the base URL
//! `http://marquee:8080/radarr` (or `/sonarr`) and Marquee's API key.

use crate::AppState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kit_server_core::{p, Db, Result, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The tables for commands and pushed indexers.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS arr_commands (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app TEXT NOT NULL,
            name TEXT NOT NULL,
            body TEXT NOT NULL,
            status TEXT NOT NULL,
            queued TEXT NOT NULL,
            ended TEXT
        );
        CREATE TABLE IF NOT EXISTS arr_indexers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app TEXT NOT NULL,
            name TEXT NOT NULL,
            body TEXT NOT NULL
        );",
    )
    .map_err(|e| anyhow::anyhow!("The API tables can't be made: {}", e.detail))
}

/// Whether `a` and `b` are equal, in time that does not depend on where they differ.
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The API key a request carries: the `X-Api-Key` header, else the `apikey` query parameter.
pub fn request_key(req: &Request) -> Option<String> {
    if let Some(k) = req.headers().get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(k.trim().to_string());
    }
    req.uri()
        .query()?
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case("apikey"))
        .map(|(_, v)| v.to_string())
}

/// Middleware: lets a request through only with Marquee's API key (401 with no body otherwise, as Radarr and
/// Sonarr answer). `/ping` needs no key.
pub async fn require_key(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if req.uri().path().ends_with("/ping") {
        return next.run(req).await;
    }
    let want = st.settings.str("api_key");
    match request_key(&req) {
        Some(k) if !want.is_empty() && ct_eq(&k, &want) => next.run(req).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// `GET /api`: the API versions.
pub async fn api_versions() -> Json<Value> {
    Json(json!({ "current": "v3", "deprecated": [] }))
}

/// `GET /ping`.
pub async fn ping() -> Json<Value> {
    Json(json!({ "status": "OK" }))
}

/// An answer with a status: `201 Created`, `202 Accepted`.
pub fn with_status(status: u16, body: Value) -> Response {
    (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(body)).into_response()
}

/// A failed check in the apps' shape: a list of `{propertyName, errorMessage, …}` with 400.
pub fn validation(property: &str, message: &str, value: Value, code: &str) -> Response {
    let body = json!([{
        "propertyName": property,
        "errorMessage": message,
        "attemptedValue": value,
        "severity": "error",
        "errorCode": code,
        "formattedMessageArguments": [],
        "formattedMessagePlaceholderValues": { "propertyName": spaced(property), "propertyValue": value },
    }]);
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}

/// `TmdbId` → `Tmdb Id`, how the apps name a property in words.
fn spaced(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Not found, in the apps' shape.
pub fn not_found() -> Response {
    let body = json!({ "type": "https://tools.ietf.org/html/rfc9110#section-15.5.5", "title": "Not Found", "status": 404 });
    (StatusCode::NOT_FOUND, Json(body)).into_response()
}

/// A paged list query: `page`, `pageSize`, `sortKey`, `sortDirection`.
#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Paging {
    /// the page, from 1
    pub page: Option<i64>,
    /// rows per page
    pub page_size: Option<i64>,
    /// the column to sort by
    pub sort_key: Option<String>,
    /// `ascending` or `descending`
    pub sort_direction: Option<String>,
}

/// One page of `records` (all of them, sorted already) in the apps' paged shape.
pub fn paged(q: &Paging, default_key: &str, default_dir: &str, records: Vec<Value>) -> Value {
    let page = q.page.unwrap_or(1).max(1);
    let size = q.page_size.unwrap_or(10).clamp(1, 10_000);
    let total = records.len() as i64;
    let rows: Vec<Value> = records.into_iter().skip(((page - 1) * size) as usize).take(size as usize).collect();
    json!({
        "page": page,
        "pageSize": size,
        // the apps answer with the column they really sorted by, whatever name was asked for
        "sortKey": default_key,
        "sortDirection": q.sort_direction.clone().unwrap_or_else(|| default_dir.to_string()),
        "totalRecords": total,
        "records": rows,
    })
}

/// A command as the apps show it.
fn command_view(r: &Row) -> Value {
    let mut body: Value = serde_json::from_str(&r.s("body")).unwrap_or_else(|_| json!({}));
    let name = r.s("name");
    if let Some(o) = body.as_object_mut() {
        o.insert("name".into(), json!(name));
        o.entry("trigger").or_insert(json!("manual"));
        o.entry("sendUpdatesToClient").or_insert(json!(true));
        o.entry("isExclusive").or_insert(json!(false));
        o.entry("isTypeExclusive").or_insert(json!(false));
        o.entry("isLongRunning").or_insert(json!(false));
        o.entry("requiresDiskAccess").or_insert(json!(false));
        o.entry("suppressMessages").or_insert(json!(false));
        o.entry("updateScheduledTask").or_insert(json!(false));
        o.entry("completionMessage").or_insert(json!("Completed"));
    }
    let done = r.s("status") == "completed";
    let mut v = json!({
        "name": name,
        "commandName": spaced(&name),
        "body": body,
        "priority": "normal",
        "status": r.s("status"),
        "result": if done { "successful" } else { "unknown" },
        "queued": r.s("queued"),
        "started": r.s("queued"),
        "trigger": "manual",
        "stateChangeTime": if r.s("ended").is_empty() { r.s("queued") } else { r.s("ended") },
        "sendUpdatesToClient": true,
        "updateScheduledTask": false,
        "id": r.i("id"),
    });
    if done {
        v["ended"] = json!(r.s("ended"));
        v["message"] = json!("Completed");
    }
    v
}

/// Records the command `name` of `app` as done now and gives it back in the apps' shape.
pub fn record_command(db: &SqliteDb, app: &str, name: &str, body: &Value) -> Result<Value> {
    let now = crate::movies::now_iso();
    let id = db.execute(
        "INSERT INTO arr_commands (app, name, body, status, queued, ended) VALUES (?,?,?,'completed',?,?)",
        p![app, name, body.to_string(), now, now],
    )?;
    // keep the last 200 per app
    db.execute("DELETE FROM arr_commands WHERE app=? AND id <= ?", p![app, id - 200])?;
    let r = db.one("SELECT * FROM arr_commands WHERE id=?", p![id])?.unwrap_or_default();
    Ok(command_view(&r))
}

/// The recent commands of `app`.
pub fn commands(db: &SqliteDb, app: &str) -> Result<Vec<Value>> {
    Ok(db.query("SELECT * FROM arr_commands WHERE app=? ORDER BY id DESC LIMIT 50", p![app])?.iter().map(command_view).collect())
}

/// The command `id` of `app`.
pub fn command(db: &SqliteDb, app: &str, id: i64) -> Result<Option<Value>> {
    Ok(db.one("SELECT * FROM arr_commands WHERE app=? AND id=?", p![app, id])?.as_ref().map(command_view))
}

/// What a secret field shows: the apps never send a saved key back.
pub const MASK: &str = "********";

/// Whether a field of an indexer holds a secret (`privacy` apiKey or password, or a name that says so).
fn secret_field(f: &Value) -> bool {
    matches!(f["privacy"].as_str(), Some("apiKey" | "password")) || matches!(f["name"].as_str(), Some("apiKey" | "password" | "passkey"))
}

/// An indexer as a client gets it: secrets masked, with its id.
fn indexer_view(r: &Row) -> Value {
    let mut v: Value = serde_json::from_str(&r.s("body")).unwrap_or_else(|_| json!({}));
    if let Some(fields) = v["fields"].as_array_mut() {
        for f in fields {
            if secret_field(f) && f["value"].as_str().is_some_and(|s| !s.is_empty()) {
                f["value"] = json!(MASK);
            }
        }
    }
    v["id"] = json!(r.i("id"));
    v
}

/// The indexers pushed to `app` (by Prowlarr or a person).
pub fn indexers(db: &SqliteDb, app: &str) -> Result<Vec<Value>> {
    Ok(db.query("SELECT * FROM arr_indexers WHERE app=? ORDER BY id", p![app])?.iter().map(indexer_view).collect())
}

/// The indexer `id` of `app`.
pub fn indexer(db: &SqliteDb, app: &str, id: i64) -> Result<Option<Value>> {
    Ok(db.one("SELECT * FROM arr_indexers WHERE app=? AND id=?", p![app, id])?.as_ref().map(indexer_view))
}

/// Saves an indexer of `app` (`id` `None` adds one). A masked secret keeps the saved value. The body is kept
/// as it came, with `supportsRss`/`supportsSearch` and the protocol filled in.
pub fn save_indexer(db: &SqliteDb, app: &str, id: Option<i64>, mut body: Value) -> Result<std::result::Result<Value, Response>> {
    let name = body["name"].as_str().map(str::trim).unwrap_or("").to_string();
    if name.is_empty() {
        return Ok(Err(validation("Name", "'Name' must not be empty.", json!(""), "NotEmptyValidator")));
    }
    if body["implementation"].as_str().unwrap_or("").is_empty() {
        return Ok(Err(validation("Implementation", "'Implementation' must not be empty.", json!(""), "NotEmptyValidator")));
    }
    let taken = db.one("SELECT id FROM arr_indexers WHERE app=? AND name=? COLLATE NOCASE AND id != ?", p![app, name, id.unwrap_or(0)])?;
    if taken.is_some() {
        return Ok(Err(validation("Name", "Should be unique", json!(name), "UniqueNameValidator")));
    }
    let old: Option<Value> = match id {
        Some(i) => db
            .one("SELECT body FROM arr_indexers WHERE app=? AND id=?", p![app, i])?
            .map(|r| serde_json::from_str(&r.s("body")).unwrap_or_default()),
        None => None,
    };
    if id.is_some() && old.is_none() {
        return Ok(Err(not_found()));
    }
    if let (Some(fields), Some(old)) = (body["fields"].as_array_mut(), &old) {
        for f in fields.iter_mut() {
            if f["value"].as_str() == Some(MASK) {
                let saved = old["fields"].as_array().and_then(|a| a.iter().find(|o| o["name"] == f["name"])).map(|o| o["value"].clone());
                f["value"] = saved.unwrap_or(json!(""));
            }
        }
    }
    if let Some(o) = body.as_object_mut() {
        o.remove("id");
        o.entry("supportsRss").or_insert(json!(true));
        o.entry("supportsSearch").or_insert(json!(true));
        let usenet = o.get("implementation").and_then(Value::as_str) == Some("Newznab");
        o.entry("protocol").or_insert(json!(if usenet { "usenet" } else { "torrent" }));
        o.entry("priority").or_insert(json!(25));
        o.entry("downloadClientId").or_insert(json!(0));
        o.entry("tags").or_insert(json!([]));
    }
    let id = match id {
        Some(i) => {
            db.execute("UPDATE arr_indexers SET name=?, body=? WHERE id=?", p![name, body.to_string(), i])?;
            i
        }
        None => db.execute("INSERT INTO arr_indexers (app, name, body) VALUES (?,?,?)", p![app, name, body.to_string()])?,
    };
    Ok(Ok(indexer(db, app, id)?.unwrap_or_default()))
}

/// Removes the indexer `id` of `app`; false when there was none.
pub fn delete_indexer(db: &SqliteDb, app: &str, id: i64) -> Result<bool> {
    if indexer(db, app, id)?.is_none() {
        return Ok(false);
    }
    db.execute("DELETE FROM arr_indexers WHERE app=? AND id=?", p![app, id])?;
    Ok(true)
}

/// The disks behind `paths` as `{path, label, freeSpace, totalSpace}`, one per distinct path that exists.
pub fn disk_space(paths: &[String]) -> Vec<Value> {
    let mut seen = std::collections::BTreeSet::new();
    paths
        .iter()
        .filter(|p| seen.insert((*p).clone()))
        .filter_map(|p| {
            let path = std::path::Path::new(p);
            let free = fs2::available_space(path).ok()?;
            let total = fs2::total_space(path).ok()?;
            Some(json!({ "path": p, "label": "", "freeSpace": free, "totalSpace": total }))
        })
        .collect()
}

/// The `{label, id}` languages Radarr and Sonarr number (Radarr's list).
pub const LANGUAGES: &str = include_str!("../data/radarr/language.json");

/// ISO 639-1 codes → the apps' language ids.
const ISO_LANGUAGES: [(&str, i64); 56] = [
    ("en", 1),
    ("fr", 2),
    ("es", 3),
    ("de", 4),
    ("it", 5),
    ("da", 6),
    ("nl", 7),
    ("ja", 8),
    ("is", 9),
    ("zh", 10),
    ("ru", 11),
    ("pl", 12),
    ("vi", 13),
    ("sv", 14),
    ("no", 15),
    ("nb", 15),
    ("fi", 16),
    ("tr", 17),
    ("pt", 18),
    ("el", 20),
    ("ko", 21),
    ("hu", 22),
    ("he", 23),
    ("lt", 24),
    ("cs", 25),
    ("hi", 26),
    ("ro", 27),
    ("th", 28),
    ("bg", 29),
    ("ar", 31),
    ("uk", 32),
    ("fa", 33),
    ("bn", 34),
    ("sk", 35),
    ("lv", 36),
    ("ca", 38),
    ("hr", 39),
    ("sr", 40),
    ("bs", 41),
    ("et", 42),
    ("ta", 43),
    ("id", 44),
    ("te", 45),
    ("mk", 46),
    ("sl", 47),
    ("ml", 48),
    ("kn", 49),
    ("sq", 50),
    ("af", 51),
    ("mr", 52),
    ("tl", 53),
    ("ur", 54),
    ("rm", 55),
    ("mn", 56),
    ("ka", 57),
    ("cn", 10),
];

/// The apps' `{id, name}` of an ISO 639-1 language code; Unknown (0) when it isn't on their list.
pub fn language(iso: &str) -> Value {
    let id = ISO_LANGUAGES.iter().find(|(c, _)| *c == iso).map(|(_, i)| *i).unwrap_or(0);
    let list: Vec<Value> = serde_json::from_str(LANGUAGES).unwrap_or_default();
    let name = list.iter().find(|l| l["id"].as_i64() == Some(id)).and_then(|l| l["name"].as_str()).unwrap_or("Unknown");
    json!({ "id": id, "name": name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_languages_and_pages() {
        assert_eq!(spaced("TmdbId"), "Tmdb Id");
        assert_eq!(language("en"), json!({ "id": 1, "name": "English" }));
        assert_eq!(language("ja"), json!({ "id": 8, "name": "Japanese" }));
        assert_eq!(language("xx"), json!({ "id": 0, "name": "Unknown" }));
        let q = Paging { page: Some(2), page_size: Some(2), ..Paging::default() };
        let v = paged(&q, "title", "ascending", (1..=5).map(|i| json!(i)).collect());
        assert_eq!((v["totalRecords"].as_i64(), v["records"].clone()), (Some(5), json!([3, 4])));
    }
}
