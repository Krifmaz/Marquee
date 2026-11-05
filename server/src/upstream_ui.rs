//! The forked web apps of Sonarr, Radarr and Seerr, built from the commits pinned in `UPSTREAM.lock` with
//! Marquee's patches (`ui/`), served next to Marquee's own web app:
//!
//! | Route | What |
//! | --- | --- |
//! | `/sonarr/…` | Sonarr's web app (admins), talking to `/sonarr/api/v3` |
//! | `/radarr/…` | Radarr's web app (admins), talking to `/radarr/api/v3` |
//! | `/requests/…` | Seerr's web app (everyone signed in), talking to `/api/v1` |
//!
//! The built files live in `<upstream-ui>/sonarr`, `…/radarr` and `…/seerr`, where `<upstream-ui>` is
//! `APP_UPSTREAM_UI_DIR` or the folder `upstream-ui` next to the web app's folder (`/app/upstream-ui` in the image).

use crate::roles::Role;
use crate::AppState;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::err;
use regex::Regex;
use serde_json::json;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use tower_http::services::ServeFile;

/// One of the two forked *arr web apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arr {
    /// Sonarr, at `/sonarr`.
    Sonarr,
    /// Radarr, at `/radarr`.
    Radarr,
}

impl Arr {
    /// The app's folder name and URL base without the slash: `sonarr` or `radarr`.
    pub fn slug(self) -> &'static str {
        match self {
            Arr::Sonarr => "sonarr",
            Arr::Radarr => "radarr",
        }
    }

    /// The name the app shows as its instance name.
    pub fn name(self) -> &'static str {
        match self {
            Arr::Sonarr => "Sonarr",
            Arr::Radarr => "Radarr",
        }
    }
}

/// The routes of the three web apps.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/sonarr", get(|| async { Redirect::permanent("/sonarr/") }))
        .route("/sonarr/", get(sonarr))
        .route("/sonarr/{*path}", get(sonarr))
        .route("/radarr", get(|| async { Redirect::permanent("/radarr/") }))
        .route("/radarr/", get(radarr))
        .route("/radarr/{*path}", get(radarr))
        .route("/requests", get(seerr))
        .route("/requests/", get(seerr))
        .route("/requests/{*path}", get(seerr))
}

/// The folder that holds the built web apps.
pub fn dir(st: &AppState) -> PathBuf {
    match std::env::var_os("APP_UPSTREAM_UI_DIR") {
        Some(d) => PathBuf::from(d),
        None => st.static_dir.with_file_name("upstream-ui"),
    }
}

/// `GET /sonarr/…`.
async fn sonarr(State(st): State<AppState>, req: Request) -> Response {
    arr_page(&st, Arr::Sonarr, req).await
}

/// `GET /radarr/…`.
async fn radarr(State(st): State<AppState>, req: Request) -> Response {
    arr_page(&st, Arr::Radarr, req).await
}

/// The part of a request's path after `/<base>/`, when every part of it is a plain name (no `..`).
fn rest_of(path: &str, base: &str) -> Option<String> {
    let rest = path.strip_prefix(base)?.trim_start_matches('/');
    let rest = percent_decode(rest)?;
    Path::new(&rest).components().all(|c| matches!(c, Component::Normal(_))).then_some(rest)
}

/// `%xx` escapes decoded; `None` when the result is not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).ok()?, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// The signed-in account's role, if anyone is signed in; the request comes back unchanged.
fn role_of(st: &AppState, req: Request) -> (Option<Role>, Request) {
    let (parts, body) = req.into_parts();
    let role = crate::auth::signed_in_with_role(st, &parts).ok().map(|(_, r)| r);
    (role, Request::from_parts(parts, body))
}

/// Where someone who may not see a page goes: Marquee's sign-in (it comes back here afterwards), or Marquee's
/// own app for a signed-in account that is not an admin.
fn send_away(signed_in: bool, uri: &axum::http::Uri) -> Response {
    if signed_in {
        Redirect::to("/").into_response()
    } else {
        let next = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        Redirect::to(&format!("/?next={}", urlencode(next))).into_response()
    }
}

/// A query value with everything but letters, digits and `-_.~/` escaped.
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// 404 with a plain sentence.
fn not_found(what: &str) -> Response {
    err(404, format!("{what} isn't here.")).into_response()
}

/// A file of a built web app, with its type, and the brotli or gzip copy when there is one and the browser takes
/// it.
async fn file(path: PathBuf, req: Request) -> Response {
    let (parts, _) = req.into_parts();
    let req = Request::from_parts(parts, Body::empty());
    match ServeFile::new(path).precompressed_br().precompressed_gzip().try_call(req).await {
        Ok(r) => r.map(Body::new),
        Err(e) => err(500, format!("The file can't be read: {e}")).into_response(),
    }
}

/// Sonarr's or Radarr's web app: its files, `initialize.json` (the API key, for admins only) and `index.html`
/// for every page.
async fn arr_page(st: &AppState, app: Arr, req: Request) -> Response {
    let base = format!("/{}", app.slug());
    let Some(rest) = rest_of(req.uri().path(), &base) else { return not_found("That page") };
    // the API and the live updates belong to the API routes; a path they don't know is not a page
    if rest.starts_with("api/") || rest.starts_with("signalr/") || rest.starts_with("feed/") {
        return not_found("That address");
    }
    let root = dir(st).join(app.slug());
    let (role, req) = role_of(st, req);
    if rest == "initialize.json" {
        return match role {
            Some(r) if r.is_admin() => initialize(st, app).into_response(),
            Some(_) => err(403, "Only an admin can open this.").into_response(),
            None => err(401, "Not authenticated").into_response(),
        };
    }
    let path = root.join(&rest);
    let is_page = rest.is_empty() || rest == "index.html" || !path.is_file();
    if !is_page {
        // the app's scripts, styles, fonts and pictures: nothing secret in them
        return file(path, req).await;
    }
    if !role.is_some_and(Role::is_admin) {
        return send_away(role.is_some(), req.uri());
    }
    let Ok(html) = std::fs::read_to_string(root.join("index.html")) else {
        return not_found(&format!("{}'s web app (this build of Marquee has none)", app.name()));
    };
    let mut r = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], arr_index(&html, &base)).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// `index.html` as Sonarr's own server sends it: every local script, style and picture under the URL base, and
/// `__URL_BASE__` filled in.
pub fn arr_index(html: &str, base: &str) -> String {
    static FILES: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)\b(href|src)="(/[^/"][^"]*?\.(?:css|js|ts|png|ico|ics|svg|json|xml))""#).unwrap());
    FILES.replace_all(html, |c: &regex::Captures| format!("{}=\"{base}{}\"", &c[1], &c[2])).replace("__URL_BASE__", base)
}

/// `initialize.json`: what the web app needs before its first request (the API root and Marquee's API key).
fn initialize(st: &AppState, app: Arr) -> impl IntoResponse {
    let base = format!("/{}", app.slug());
    let mut r = Json(json!({
        "apiRoot": format!("{base}/api/v3"),
        "apiKey": st.settings.str("api_key"),
        "release": format!("Marquee {}", crate::VERSION),
        "version": upstream_version(st, app),
        "instanceName": app.name(),
        "theme": "dark",
        "branch": "main",
        "analytics": false,
        "userHash": "",
        "urlBase": base,
        "isProduction": true,
    }))
    .into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// The upstream release the built web app is (`VERSION` next to it, written by the image build), else
/// Marquee's own version.
fn upstream_version(st: &AppState, app: Arr) -> String {
    std::fs::read_to_string(dir(st).join(app.slug()).join("VERSION"))
        .map(|v| v.trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| crate::VERSION.to_string())
}

/// `GET /requests/…`: Seerr's web app, a static build. A page's address finds its file the way Seerr's own
/// server does: the file itself, `<page>.html`, `<page>/index.html`, then a page with a `[name]` part (a movie,
/// a show, an account), else `404.html`. The app checks the sign-in itself and sends visitors to `/`.
async fn seerr(State(st): State<AppState>, req: Request) -> Response {
    let Some(rest) = rest_of(req.uri().path(), "/requests") else { return not_found("That page") };
    let root = dir(&st).join("seerr");
    if !root.join("index.html").is_file() {
        return not_found("Seerr's web app (this build of Marquee has none)");
    }
    match seerr_file(&root, &rest) {
        Some(path) => {
            let page = path.extension().is_some_and(|e| e == "html");
            let mut r = file(path, req).await;
            if page {
                r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            }
            r
        }
        None => {
            let mut r = file(root.join("404.html"), req).await;
            *r.status_mut() = StatusCode::NOT_FOUND;
            r
        }
    }
}

/// The file of Seerr's static build that answers `rest` (the path after `/requests/`).
pub fn seerr_file(root: &Path, rest: &str) -> Option<PathBuf> {
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return Some(root.join("index.html"));
    }
    let exact = root.join(rest);
    if exact.is_file() {
        return Some(exact);
    }
    let parts: Vec<&str> = rest.split('/').collect();
    page_in(root, &parts)
}

/// The page for the path parts `parts` under `dir`: a name beats a `[name]` part at every level.
fn page_in(dir: &Path, parts: &[&str]) -> Option<PathBuf> {
    let (first, more) = parts.split_first()?;
    if more.is_empty() {
        let mut names = vec![format!("{first}.html")];
        names.extend(dynamic_names(dir, ".html"));
        for n in names {
            if dir.join(&n).is_file() {
                return Some(dir.join(n));
            }
        }
        let mut dirs = vec![first.to_string()];
        dirs.extend(dynamic_names(dir, ""));
        return dirs.into_iter().map(|d| dir.join(d).join("index.html")).find(|p| p.is_file());
    }
    let mut dirs = vec![first.to_string()];
    dirs.extend(dynamic_names(dir, ""));
    dirs.into_iter().filter(|d| dir.join(d).is_dir()).find_map(|d| page_in(&dir.join(d), more))
}

/// The `[name]` entries of a folder that end in `suffix` (`.html` for pages, `""` for folders).
fn dynamic_names(dir: &Path, suffix: &str) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else { return vec![] };
    let mut names: Vec<String> = read
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with('[') && n.ends_with(&format!("]{suffix}")))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_puts_local_files_under_the_url_base() {
        let html = r#"<link href="/Content/styles.css" rel="stylesheet"><script src="/index-1.js" data-no-hash></script>
<a href="https://example.com/x.js"></a><script>window.Sonarr = { urlBase: '__URL_BASE__' };</script>"#;
        let out = arr_index(html, "/sonarr");
        assert!(out.contains(r#"href="/sonarr/Content/styles.css""#));
        assert!(out.contains(r#"src="/sonarr/index-1.js""#));
        assert!(out.contains(r#"href="https://example.com/x.js""#));
        assert!(out.contains("urlBase: '/sonarr'"));
    }

    #[test]
    fn rest_refuses_parent_folders() {
        assert_eq!(rest_of("/sonarr/Content/a.css", "/sonarr").as_deref(), Some("Content/a.css"));
        assert_eq!(rest_of("/sonarr/", "/sonarr").as_deref(), Some(""));
        assert_eq!(rest_of("/sonarr/../secret", "/sonarr"), None);
        assert_eq!(rest_of("/sonarr/%2e%2e/secret", "/sonarr"), None);
    }

    #[test]
    fn seerr_pages_find_their_files() {
        let root = std::env::temp_dir().join(format!("marquee-seerr-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for f in [
            "index.html",
            "404.html",
            "discover.html",
            "movie/[movieId].html",
            "movie/[movieId]/cast.html",
            "users/[userId]/settings/main.html",
            "settings/index.html",
            "_next/static/a.js",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, f).unwrap();
        }
        let got = |rest: &str| seerr_file(&root, rest).map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().into_owned());
        assert_eq!(got("").as_deref(), Some("index.html"));
        assert_eq!(got("discover").as_deref(), Some("discover.html"));
        assert_eq!(got("movie/603").as_deref(), Some("movie/[movieId].html"));
        assert_eq!(got("movie/603/cast").as_deref(), Some("movie/[movieId]/cast.html"));
        assert_eq!(got("users/4/settings/main").as_deref(), Some("users/[userId]/settings/main.html"));
        assert_eq!(got("settings/").as_deref(), Some("settings/index.html"));
        assert_eq!(got("_next/static/a.js").as_deref(), Some("_next/static/a.js"));
        assert_eq!(got("nothing/here"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
