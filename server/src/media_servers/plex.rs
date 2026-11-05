//! Plex's API, the parts Marquee needs: the server's name and libraries, scanning, the TMDB ids of what it has
//! (all on the server at `plex_url` with the owner's `plex_token`), and on plex.tv: sharing libraries with a
//! person, changing and removing a share, and signing a person in with a PIN.
//!
//! plex.tv answers some of these in XML only; [`xml_elements`] reads the few attributes Marquee needs.

use crate::AppState;
use kit_server_core::{err, Result};
use serde_json::{json, Value};

/// plex.tv's address.
pub const PLEX_TV: &str = "https://plex.tv";

/// One of the server's libraries.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Library {
    /// the server's own key of the library (`1`, `2`…)
    pub id: String,
    /// its name
    pub name: String,
    /// `movie`, `show`, `artist`, `photo`
    pub kind: String,
}

/// A share of the server with one person (plex.tv's "shared server").
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Share {
    /// plex.tv's id of the share
    pub id: String,
    /// the person's plex.tv id ("" until they accept)
    pub user_id: String,
    /// their plex.tv name
    pub username: String,
    /// their mail address
    pub email: String,
}

/// A plex.tv account.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Account {
    /// plex.tv's id
    pub id: String,
    /// the plex.tv name
    pub username: String,
    /// the mail address
    pub email: String,
}

/// Something Plex has, with its TMDB id.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Plex's rating key
    pub id: String,
    /// `movie` or `tv`
    pub kind: &'static str,
    /// TMDB's id
    pub tmdb_id: i64,
}

/// A Plex client: the server at `plex_url` and plex.tv, with one token.
#[derive(Clone)]
pub struct Plex {
    /// the server's address, without a trailing `/` ("" for plex.tv only)
    base: String,
    /// plex.tv's address (the `plex_tv_url` setting, for tests), without a trailing `/`
    tv: String,
    /// the token
    token: String,
    /// this Marquee's client id for plex.tv
    client: String,
    /// the shared HTTP client
    http: reqwest::Client,
}

/// plex.tv's address from the settings.
fn tv_url(st: &AppState) -> String {
    let t = st.settings.str("plex_tv_url");
    if t.trim().is_empty() {
        PLEX_TV.to_string()
    } else {
        t.trim().trim_end_matches('/').to_string()
    }
}

/// This Marquee's client id for plex.tv: stable, made from the server's API key.
pub fn client_id(st: &AppState) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(format!("marquee-plex:{}", st.settings.str("api_key")).as_bytes());
    format!("marquee-{}", &hex::encode(h)[..16])
}

impl Plex {
    /// The client for the server in the settings; `None` when the address or token is missing.
    pub fn from_settings(st: &AppState) -> Option<Plex> {
        let base = st.settings.str("plex_url").trim().trim_end_matches('/').to_string();
        let token = st.settings.str("plex_token").trim().to_string();
        (!base.is_empty() && !token.is_empty()).then(|| Plex { base, tv: tv_url(st), token, client: client_id(st), http: st.http.clone() })
    }

    /// A plex.tv client with a person's `token` (or "" for the PIN sign-in).
    pub fn person(st: &AppState, token: &str) -> Plex {
        Plex { base: String::new(), tv: tv_url(st), token: token.to_string(), client: client_id(st), http: st.http.clone() }
    }

    /// One call to `url`; the answer's text, or the reason in words.
    async fn raw(&self, method: reqwest::Method, url: &str, body: Option<Value>) -> Result<String> {
        let mut r = self
            .http
            .request(method, url)
            .header("Accept", "application/json")
            .header("X-Plex-Product", "Marquee")
            .header("X-Plex-Version", crate::VERSION)
            .header("X-Plex-Client-Identifier", &self.client);
        if !self.token.is_empty() {
            r = r.header("X-Plex-Token", &self.token);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let where_ = if url.starts_with(&self.tv) { "plex.tv" } else { "Plex" };
        let resp = r.send().await.map_err(|e| err(502, format!("{where_} can't be reached: {}", super::jellyfin::plain(&e))))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 401 {
            return Err(err(502, format!("{where_} turned the Plex token down. Check the token in Settings → Media servers → Plex.")));
        }
        if !status.is_success() {
            let why = text.trim().chars().take(200).collect::<String>();
            return Err(err(502, format!("{where_} answered {status}{}", if why.is_empty() { String::new() } else { format!(": {why}") })));
        }
        Ok(text)
    }

    /// A call to the server; its JSON answer.
    async fn server(&self, path: &str) -> Result<Value> {
        let text = self.raw(reqwest::Method::GET, &format!("{}{path}", self.base), None).await?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// The server's name, version and machine id.
    pub async fn info(&self) -> Result<(String, String, String)> {
        let v = self.server("/").await?;
        let m = &v["MediaContainer"];
        Ok((s(m, "friendlyName"), s(m, "version"), s(m, "machineIdentifier")))
    }

    /// Every library.
    pub async fn libraries(&self) -> Result<Vec<Library>> {
        let v = self.server("/library/sections").await?;
        Ok(v["MediaContainer"]["Directory"]
            .as_array()
            .map(|a| a.iter().map(|d| Library { id: s(d, "key"), name: s(d, "title"), kind: s(d, "type") }).collect())
            .unwrap_or_default())
    }

    /// Asks Plex to scan every movie and TV library.
    pub async fn scan(&self) -> Result<()> {
        for l in self.libraries().await?.iter().filter(|l| l.kind == "movie" || l.kind == "show") {
            self.server(&format!("/library/sections/{}/refresh", seg(&l.id))).await?;
        }
        Ok(())
    }

    /// Every movie and series Plex has that carries a TMDB id.
    pub async fn items(&self) -> Result<Vec<Item>> {
        let mut out = Vec::new();
        for l in self.libraries().await?.iter().filter(|l| l.kind == "movie" || l.kind == "show") {
            let v = self.server(&format!("/library/sections/{}/all?includeGuids=1", seg(&l.id))).await?;
            let kind = if l.kind == "show" { "tv" } else { "movie" };
            for m in v["MediaContainer"]["Metadata"].as_array().into_iter().flatten() {
                let tmdb = m["Guid"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|g| g["id"].as_str())
                    .find_map(|g| g.strip_prefix("tmdb://").and_then(|t| t.parse::<i64>().ok()));
                if let Some(tmdb_id) = tmdb {
                    out.push(Item { id: s(m, "ratingKey"), kind, tmdb_id });
                }
            }
        }
        Ok(out)
    }

    /// plex.tv's ids of the server's libraries, by the server's own key: shares name libraries by these.
    async fn section_ids(&self, machine: &str) -> Result<Vec<(String, String)>> {
        let xml = self.raw(reqwest::Method::GET, &format!("{}/api/servers/{}", self.tv, seg(machine)), None).await?;
        Ok(xml_elements(&xml, "Section").iter().map(|a| (attr(a, "key"), attr(a, "id"))).collect())
    }

    /// Every share of the server with machine id `machine`.
    pub async fn shares(&self, machine: &str) -> Result<Vec<Share>> {
        let xml = self.raw(reqwest::Method::GET, &format!("{}/api/servers/{}/shared_servers", self.tv, seg(machine)), None).await?;
        Ok(xml_elements(&xml, "SharedServer")
            .iter()
            .map(|a| Share { id: attr(a, "id"), user_id: attr(a, "userID"), username: attr(a, "username"), email: attr(a, "email") })
            .collect())
    }

    /// plex.tv's ids for the libraries `keys` (the server's own keys; `None`: every library).
    async fn plex_tv_ids(&self, machine: &str, keys: Option<&[String]>) -> Result<Vec<i64>> {
        let all = self.section_ids(machine).await?;
        Ok(all.iter().filter(|(key, _)| keys.is_none_or(|k| k.contains(key))).filter_map(|(_, id)| id.parse().ok()).collect())
    }

    /// Shares the libraries `keys` (`None`: every library) with the person at `email`; the share.
    pub async fn invite(&self, machine: &str, email: &str, keys: Option<&[String]>) -> Result<Share> {
        let ids = self.plex_tv_ids(machine, keys).await?;
        let body = json!({
            "machineIdentifier": machine,
            "librarySectionIds": ids,
            "settings": { "allowSync": false, "allowCameraUpload": false, "allowChannels": false },
            "invitedEmail": email,
        });
        let text = self.raw(reqwest::Method::POST, &format!("{}/api/v2/shared_servers", self.tv), Some(body)).await?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let id = v.get("id").map(|i| i.to_string().trim_matches('"').to_string()).unwrap_or_default();
        let user_id =
            v.get("invitedId").or_else(|| v.get("userID")).map(|i| i.to_string().trim_matches('"').to_string()).unwrap_or_default();
        Ok(Share { id, user_id, username: String::new(), email: email.to_string() })
    }

    /// Changes the libraries of the share `share_id` to `keys` (`None`: every library).
    pub async fn update_share(&self, machine: &str, share_id: &str, keys: Option<&[String]>) -> Result<()> {
        let ids = self.plex_tv_ids(machine, keys).await?;
        let body = json!({ "server_id": machine, "shared_server": { "library_section_ids": ids } });
        let url = format!("{}/api/servers/{}/shared_servers/{}", self.tv, seg(machine), seg(share_id));
        self.raw(reqwest::Method::PUT, &url, Some(body)).await.map(|_| ())
    }

    /// Removes the share `share_id`: the person no longer sees the server.
    pub async fn remove_share(&self, machine: &str, share_id: &str) -> Result<()> {
        let url = format!("{}/api/servers/{}/shared_servers/{}", self.tv, seg(machine), seg(share_id));
        self.raw(reqwest::Method::DELETE, &url, None).await.map(|_| ())
    }

    /// Starts a PIN sign-in: the PIN's id and code.
    pub async fn new_pin(&self) -> Result<(String, String)> {
        let text = self.raw(reqwest::Method::POST, &format!("{}/api/v2/pins?strong=true", self.tv), None).await?;
        let v: Value = serde_json::from_str(&text).map_err(|_| err(502, "plex.tv's answer can't be read"))?;
        Ok((v["id"].to_string().trim_matches('"').to_string(), s(&v, "code")))
    }

    /// The token the PIN `id` was signed in with, `None` while the person hasn't finished.
    pub async fn pin_token(&self, id: &str) -> Result<Option<String>> {
        let text = self.raw(reqwest::Method::GET, &format!("{}/api/v2/pins/{}", self.tv, seg(id)), None).await?;
        let v: Value = serde_json::from_str(&text).map_err(|_| err(502, "plex.tv's answer can't be read"))?;
        Ok(v["authToken"].as_str().filter(|t| !t.is_empty()).map(str::to_string))
    }

    /// The plex.tv account of this client's token.
    pub async fn account(&self) -> Result<Account> {
        let text = self.raw(reqwest::Method::GET, &format!("{}/api/v2/user", self.tv), None).await?;
        let v: Value = serde_json::from_str(&text).map_err(|_| err(502, "plex.tv's answer can't be read"))?;
        Ok(Account { id: v["id"].to_string().trim_matches('"').to_string(), username: s(&v, "username"), email: s(&v, "email") })
    }

    /// Whether this client's token may open the server with machine id `machine`.
    pub async fn can_open(&self, machine: &str) -> Result<bool> {
        let text = self.raw(reqwest::Method::GET, &format!("{}/api/v2/resources", self.tv), None).await?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(v.as_array().into_iter().flatten().any(|r| r["clientIdentifier"].as_str() == Some(machine)))
    }
}

/// The address a person opens to sign in with the PIN `code`.
pub fn auth_url(st: &AppState, code: &str) -> String {
    format!(
        "https://app.plex.tv/auth#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D=Marquee",
        client_id(st),
        code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>()
    )
}

/// A string (or number) field, "" when missing.
fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// `id` safe as one path segment.
fn seg(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

/// The attributes of every `<tag …>` element in `xml`, as (name, value) pairs with the five XML entities
/// read.
pub fn xml_elements(xml: &str, tag: &str) -> Vec<Vec<(String, String)>> {
    let open = format!("<{tag}");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        // `<SectionX` is another tag
        if !after.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            rest = after;
            continue;
        }
        let end = after.find('>').unwrap_or(after.len());
        out.push(attributes(&after[..end]));
        rest = &after[end..];
    }
    out
}

/// The `name="value"` pairs of an element's inside.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim().trim_start_matches('/').trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let body = &after[1..];
        let close = body.find(q).unwrap_or(body.len());
        let name = name.rsplit(char::is_whitespace).next().unwrap_or("").to_string();
        out.push((name, unescape(&body[..close])));
        rest = body.get(close + 1..).unwrap_or("");
    }
    out
}

/// `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&apos;` read.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The value of attribute `name`, "" when missing.
fn attr(a: &[(String, String)], name: &str) -> String {
    a.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{attr, xml_elements};

    #[test]
    fn reads_plex_tv_xml() {
        let xml = r#"<?xml version="1.0"?><MediaContainer><Server name="Home"><Section id="101" key="1" title="Movies &amp; more" type="movie"/>
            <Section id='102' key="2" title="TV" type="show"></Section><SectionX id="9"/></Server></MediaContainer>"#;
        let s = xml_elements(xml, "Section");
        assert_eq!(s.len(), 2);
        assert_eq!((attr(&s[0], "id"), attr(&s[0], "key"), attr(&s[0], "title")), ("101".into(), "1".into(), "Movies & more".into()));
        assert_eq!(attr(&s[1], "id"), "102");
        assert_eq!(attr(&s[1], "missing"), "");
    }
}
