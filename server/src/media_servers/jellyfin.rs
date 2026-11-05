//! Jellyfin's API, the parts Marquee needs: the server's name, its libraries, its users (make, change
//! password, turn off, library access, remove), signing a person in, scanning the library and the TMDB ids of
//! what it has. The address and API key are the `jellyfin_url` and `jellyfin_api_key` settings.

use crate::AppState;
use kit_server_core::{err, Result};
use serde_json::{json, Value};

/// One of the server's libraries.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Library {
    /// Jellyfin's id of the library folder
    pub id: String,
    /// its name
    pub name: String,
    /// `movies`, `tvshows`, `music`…, "" when mixed
    pub kind: String,
}

/// A Jellyfin user.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct User {
    /// Jellyfin's id
    pub id: String,
    /// the sign-in name
    pub name: String,
    /// whether Jellyfin turned it off
    pub disabled: bool,
    /// whether it is a Jellyfin administrator
    pub admin: bool,
    /// the libraries it sees: `None` for every library
    pub libraries: Option<Vec<String>>,
}

/// Something Jellyfin has, with its TMDB id.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Jellyfin's id
    pub id: String,
    /// `movie` or `tv`
    pub kind: &'static str,
    /// TMDB's id
    pub tmdb_id: i64,
}

/// A Jellyfin client for one server.
#[derive(Clone)]
pub struct Jellyfin {
    /// the address, without a trailing `/`
    base: String,
    /// the API key ("" to sign a person in)
    key: String,
    /// the shared HTTP client
    http: reqwest::Client,
}

impl Jellyfin {
    /// The client for the server in the settings; `None` when the address or key is missing.
    pub fn from_settings(st: &AppState) -> Option<Jellyfin> {
        let base = st.settings.str("jellyfin_url").trim().trim_end_matches('/').to_string();
        let key = st.settings.str("jellyfin_api_key").trim().to_string();
        (!base.is_empty() && !key.is_empty()).then(|| Jellyfin { base, key, http: st.http.clone() })
    }

    /// The `Authorization` header Jellyfin reads (with the API key when there is one).
    fn auth(&self) -> String {
        let mut h = format!(r#"MediaBrowser Client="Marquee", Device="Marquee", DeviceId="marquee-server", Version="{}""#, crate::VERSION);
        if !self.key.is_empty() {
            h.push_str(&format!(r#", Token="{}""#, self.key));
        }
        h
    }

    /// One call; the JSON answer (`null` for an empty one), or the reason in words.
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut r = self.http.request(method, format!("{}{path}", self.base)).header("Authorization", self.auth());
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.map_err(|e| err(502, format!("Jellyfin at {} can't be reached: {}", self.base, plain(&e))))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(err(502, "Jellyfin turned the API key down. Make a new one in Jellyfin → Dashboard → API Keys."));
        }
        if !status.is_success() {
            let why = text.trim().chars().take(200).collect::<String>();
            return Err(err(
                502,
                format!("Jellyfin answered {status} to {path}{}", if why.is_empty() { String::new() } else { format!(": {why}") }),
            ));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// The server's name and version.
    pub async fn info(&self) -> Result<(String, String)> {
        let v = self.call(reqwest::Method::GET, "/System/Info", None).await?;
        Ok((s(&v, "ServerName"), s(&v, "Version")))
    }

    /// Every library.
    pub async fn libraries(&self) -> Result<Vec<Library>> {
        let v = self.call(reqwest::Method::GET, "/Library/VirtualFolders", None).await?;
        Ok(v.as_array()
            .map(|a| a.iter().map(|l| Library { id: s(l, "ItemId"), name: s(l, "Name"), kind: s(l, "CollectionType") }).collect())
            .unwrap_or_default())
    }

    /// Every user.
    pub async fn users(&self) -> Result<Vec<User>> {
        let v = self.call(reqwest::Method::GET, "/Users", None).await?;
        Ok(v.as_array().map(|a| a.iter().map(user).collect()).unwrap_or_default())
    }

    /// Makes the user `name` with `password`; its id.
    pub async fn create_user(&self, name: &str, password: &str) -> Result<String> {
        let v = self.call(reqwest::Method::POST, "/Users/New", Some(json!({ "Name": name, "Password": password }))).await?;
        let id = s(&v, "Id");
        if id.is_empty() {
            return Err(err(502, format!("Jellyfin didn't make the user {name}")));
        }
        Ok(id)
    }

    /// Sets the password of user `id` (as an administrator: the old one isn't needed).
    pub async fn set_password(&self, id: &str, password: &str) -> Result<()> {
        let body = json!({ "NewPw": password, "ResetPassword": false });
        self.call(reqwest::Method::POST, &format!("/Users/{}/Password", seg(id)), Some(body)).await.map(|_| ())
    }

    /// Turns user `id` off or on and sets its libraries (`None`: every library). Its other rights stay as they are.
    pub async fn set_policy(&self, id: &str, disabled: bool, libraries: Option<&[String]>) -> Result<()> {
        let u = self.call(reqwest::Method::GET, &format!("/Users/{}", seg(id)), None).await?;
        let mut policy = u.get("Policy").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        policy["IsDisabled"] = json!(disabled);
        match libraries {
            None => {
                policy["EnableAllFolders"] = json!(true);
                policy["EnabledFolders"] = json!([]);
            }
            Some(l) => {
                policy["EnableAllFolders"] = json!(false);
                policy["EnabledFolders"] = json!(l);
            }
        }
        // Jellyfin needs these two on every policy it is sent
        for k in ["AuthenticationProviderId", "PasswordResetProviderId"] {
            if policy.get(k).and_then(Value::as_str).unwrap_or("").is_empty() {
                policy[k] = json!(match k {
                    "AuthenticationProviderId" => "Jellyfin.Server.Implementations.Users.DefaultAuthenticationProvider",
                    _ => "Jellyfin.Server.Implementations.Users.DefaultPasswordResetProvider",
                });
            }
        }
        self.call(reqwest::Method::POST, &format!("/Users/{}/Policy", seg(id)), Some(policy)).await.map(|_| ())
    }

    /// Removes user `id`.
    pub async fn delete_user(&self, id: &str) -> Result<()> {
        self.call(reqwest::Method::DELETE, &format!("/Users/{}", seg(id)), None).await.map(|_| ())
    }

    /// Asks Jellyfin to scan every library.
    pub async fn scan(&self) -> Result<()> {
        self.call(reqwest::Method::POST, "/Library/Refresh", None).await.map(|_| ())
    }

    /// Every movie and series Jellyfin has that carries a TMDB id.
    pub async fn items(&self) -> Result<Vec<Item>> {
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            let path = format!(
                "/Items?Recursive=true&IncludeItemTypes=Movie,Series&Fields=ProviderIds&EnableImages=false&StartIndex={start}&Limit=1000"
            );
            let v = self.call(reqwest::Method::GET, &path, None).await?;
            let page = v.get("Items").and_then(Value::as_array).cloned().unwrap_or_default();
            for i in &page {
                let kind = if s(i, "Type") == "Series" { "tv" } else { "movie" };
                let tmdb = i.get("ProviderIds").and_then(|p| p.get("Tmdb").or_else(|| p.get("tmdb"))).and_then(Value::as_str).unwrap_or("");
                if let Ok(tmdb_id) = tmdb.parse::<i64>() {
                    out.push(Item { id: s(i, "Id"), kind, tmdb_id });
                }
            }
            start += page.len();
            let total = v.get("TotalRecordCount").and_then(Value::as_u64).unwrap_or(0) as usize;
            if page.is_empty() || start >= total {
                return Ok(out);
            }
        }
    }

    /// Signs `username` in with `password`; the Jellyfin user, or `None` when Jellyfin says the name or
    /// password is wrong.
    pub async fn authenticate(st: &AppState, username: &str, password: &str) -> Result<Option<User>> {
        let base = st.settings.str("jellyfin_url").trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err(err(400, "Jellyfin isn't set up on this server"));
        }
        let anon = Jellyfin { base, key: String::new(), http: st.http.clone() };
        let r = anon
            .http
            .post(format!("{}/Users/AuthenticateByName", anon.base))
            .header("Authorization", anon.auth())
            .json(&json!({ "Username": username, "Pw": password }))
            .send()
            .await
            .map_err(|e| err(502, format!("Jellyfin can't be reached: {}", plain(&e))))?;
        if matches!(r.status().as_u16(), 400 | 401 | 403 | 404) {
            return Ok(None);
        }
        if !r.status().is_success() {
            return Err(err(502, format!("Jellyfin answered {} to the sign-in", r.status())));
        }
        let v: Value = r.json().await.map_err(|_| err(502, "Jellyfin's answer to the sign-in can't be read"))?;
        Ok(v.get("User").map(user))
    }
}

/// A user from Jellyfin's JSON.
fn user(u: &Value) -> User {
    let policy = u.get("Policy").cloned().unwrap_or(Value::Null);
    User {
        id: s(u, "Id"),
        name: s(u, "Name"),
        disabled: policy.get("IsDisabled").and_then(Value::as_bool).unwrap_or(false),
        admin: policy.get("IsAdministrator").and_then(Value::as_bool).unwrap_or(false),
        libraries: if policy.get("EnableAllFolders").and_then(Value::as_bool).unwrap_or(true) {
            None
        } else {
            Some(
                policy
                    .get("EnabledFolders")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
            )
        },
    }
}

/// A string field, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// `id` safe as one path segment (Jellyfin's ids are hex; anything else is dropped).
fn seg(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

/// A request error in a few plain words.
pub fn plain(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "it took too long to answer".into()
    } else if e.is_connect() {
        "nothing answered at that address".into()
    } else {
        e.to_string()
    }
}
