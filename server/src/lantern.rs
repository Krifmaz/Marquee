//! Talking to Lantern, the one place Marquee searches and downloads through (docs/LANTERN_HUB.md has the contract).
//!
//! Searching uses Lantern's Prowlarr API (`GET /api/v1/search`). Grabbing, progress and finished files use Lantern's
//! download hub (`/api/v1/hub/...`): Marquee registers itself, sends a release to grab, and Lantern calls
//! `POST /api/lantern/events` back when a download moves on, with the finished files' paths.

use crate::AppState;
use kit_server_core::{err, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One release Lantern found.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Release {
    /// Lantern's stable id for it
    pub guid: String,
    /// the release's name
    pub title: String,
    /// bytes
    pub size: i64,
    /// `torrent` or `usenet`
    pub protocol: String,
    /// the indexer's name
    pub indexer: String,
    /// the indexer's id in Lantern
    pub indexer_id: i64,
    /// seeders (torrents)
    pub seeders: Option<i64>,
    /// leechers (torrents)
    pub leechers: Option<i64>,
    /// the link Lantern serves the file at
    pub download_url: Option<String>,
    /// a magnet link (torrents)
    pub magnet_url: Option<String>,
    /// the release's page on the site
    pub info_url: Option<String>,
    /// when it was published, RFC 3339
    pub publish_date: Option<String>,
    /// age in days
    pub age: i64,
}

/// One download Lantern's hub keeps for Marquee.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Grab {
    /// Lantern's id for the download
    pub id: String,
    /// what Marquee sent as `ref` (`movie:<id>` or `episodes:<id>,<id>`)
    #[serde(rename = "ref")]
    pub reference: String,
    /// the release's name
    pub title: String,
    /// `queued`, `downloading`, `paused`, `completed`, `failed` or `removed`
    pub status: String,
    /// from 0 to 1
    pub progress: f64,
    /// bytes
    pub size: i64,
    /// bytes still to come
    pub left: i64,
    /// seconds still to go, when known
    pub eta: Option<i64>,
    /// what went wrong, in words, when it failed
    pub error: Option<String>,
    /// the finished files, as Marquee sees them (Lantern applies its path mappings), on the same disk as the media
    /// folders so they can be hardlinked
    pub files: Vec<String>,
    /// `torrent` or `usenet`
    pub protocol: String,
}

/// What Marquee asks Lantern to search for.
#[derive(Clone, Debug, Default)]
pub struct Query {
    /// `movie`, `tvsearch` or `search`
    pub kind: &'static str,
    /// title words
    pub terms: String,
    /// IMDb id (`tt…`)
    pub imdb: String,
    /// TMDB id
    pub tmdb: i64,
    /// TheTVDB id
    pub tvdb: i64,
    /// season
    pub season: Option<i64>,
    /// episode in the season
    pub episode: Option<i64>,
    /// Newznab categories (2000 movies, 5000 TV, 5070 anime)
    pub categories: Vec<u32>,
}

/// Lantern as the settings say to reach it.
pub struct Lantern {
    /// the address, without a trailing slash
    base: String,
    /// Lantern's API key
    key: String,
    /// the shared HTTP client
    http: reqwest::Client,
}

/// The name Marquee registers under.
pub const APP_NAME: &str = "Marquee";

impl Lantern {
    /// Lantern from Settings → Connections; 400 with what to do when it is not set up.
    pub fn from(st: &AppState) -> Result<Lantern> {
        let base = st.settings.str("lantern_url").trim().trim_end_matches('/').to_string();
        let key = st.settings.str("lantern_api_key").trim().to_string();
        if base.is_empty() || key.is_empty() {
            return Err(err(400, "Add Lantern's address and API key in Settings → Connections first"));
        }
        Ok(Lantern { base, key, http: st.http.clone() })
    }

    /// Whether Lantern is set up.
    pub fn configured(st: &AppState) -> bool {
        Lantern::from(st).is_ok()
    }

    /// One call to Lantern; its JSON answer, or a plain error that says what happened.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let r = req.header("X-Api-Key", &self.key).timeout(std::time::Duration::from_secs(90)).send().await.map_err(|e| {
            tracing::warn!("Lantern can't be reached: {e}");
            err(502, format!("Lantern can't be reached at {}. Check the address in Settings → Connections.", self.base))
        })?;
        let status = r.status();
        let body: Value = r.json().await.unwrap_or(Value::Null);
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(err(502, "Lantern turned down the API key. Copy it again from Lantern → Settings."));
        }
        if !status.is_success() {
            let why = body["detail"].as_str().or(body["message"].as_str()).unwrap_or("no reason given").to_string();
            return Err(err(502, format!("Lantern answered {}: {why}", status.as_u16())));
        }
        Ok(body)
    }

    /// Searches every indexer in Lantern.
    pub async fn search(&self, q: &Query) -> Result<Vec<Release>> {
        let mut params: Vec<(&str, String)> = vec![("type", q.kind.to_string()), ("limit", "100".into())];
        if !q.terms.is_empty() {
            params.push(("query", q.terms.clone()));
        }
        if !q.imdb.is_empty() {
            params.push(("imdbid", q.imdb.clone()));
        }
        if q.tmdb > 0 {
            params.push(("tmdbid", q.tmdb.to_string()));
        }
        if q.tvdb > 0 {
            params.push(("tvdbid", q.tvdb.to_string()));
        }
        if let Some(s) = q.season {
            params.push(("season", s.to_string()));
        }
        if let Some(e) = q.episode {
            params.push(("ep", e.to_string()));
        }
        for c in &q.categories {
            params.push(("categories", c.to_string()));
        }
        let v = self.send(self.http.get(format!("{}/api/v1/search", self.base)).query(&params)).await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Registers Marquee with Lantern's hub (safe to repeat): its kinds of media and where Lantern calls back.
    pub async fn register(&self, callback: &str, callback_key: &str) -> Result<Value> {
        let body = json!({ "name": APP_NAME, "kinds": ["movie", "tv", "anime"], "callback": callback, "callbackKey": callback_key });
        self.send(self.http.post(format!("{}/api/v1/hub/apps", self.base)).json(&body)).await
    }

    /// Asks Lantern to download `release` for `kind` (`movie`, `tv` or `anime`); Lantern's download.
    pub async fn grab(&self, kind: &str, reference: &str, release: &Release) -> Result<Grab> {
        let body = json!({ "app": APP_NAME, "kind": kind, "ref": reference, "release": release });
        let v = self.send(self.http.post(format!("{}/api/v1/hub/grabs", self.base)).json(&body)).await?;
        serde_json::from_value(v).map_err(|_| err(502, "Lantern's answer to a grab can't be read"))
    }

    /// Every download Lantern keeps for Marquee.
    pub async fn grabs(&self) -> Result<Vec<Grab>> {
        let v = self.send(self.http.get(format!("{}/api/v1/hub/grabs", self.base)).query(&[("app", APP_NAME)])).await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Tells Lantern a download's files are in the library, so its seeding and clean-up rules can run.
    pub async fn imported(&self, id: &str) -> Result<()> {
        self.send(self.http.post(format!("{}/api/v1/hub/grabs/{id}/imported", self.base))).await.map(|_| ())
    }

    /// Stops a download in Lantern; `remove_data` also deletes what was downloaded so far.
    pub async fn remove(&self, id: &str, remove_data: bool) -> Result<()> {
        let url = format!("{}/api/v1/hub/grabs/{id}", self.base);
        self.send(self.http.delete(url).query(&[("removeData", remove_data)])).await.map(|_| ())
    }
}
