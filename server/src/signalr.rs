//! Port of Sonarr's and Radarr's `SignalR` live updates (`MessageHub`, `SignalRMessageBroadcaster`): the forked
//! web apps connect to `<app>/signalr/messages` and get `receiveMessage` calls whenever something changes, so lists
//! update without a reload.
//!
//! Only what the upstream frontends use of the ASP.NET Core SignalR protocol is spoken here: the negotiate call,
//! the WebSockets transport, the JSON hub protocol (records end with `0x1E`), the handshake, pings (type 6) and
//! invocations (type 1) of `receiveMessage`. Upstream's other transports (Server-Sent Events, long polling) are
//! not offered; every browser the forked UIs support has WebSockets.
//!
//! ```text
//! POST /<app>/signalr/messages/negotiate?negotiateVersion=1   → {connectionId, connectionToken, …}
//! GET  /<app>/signalr/messages?id=…&access_token=<API key>    → WebSocket
//! ```
//!
//! Other modules send updates with [`broadcast`], as upstream's services do with `BroadcastMessage`.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::broadcast;

/// The record separator that ends every SignalR JSON message.
const RS: char = '\u{1e}';

/// How often the server pings (upstream's default `KeepAliveInterval`, 15 seconds).
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// Which app's hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    /// Sonarr
    Sonarr,
    /// Radarr
    Radarr,
}

impl App {
    /// The version the hub announces when a client connects (the same one `system/status` answers, so the UI
    /// doesn't think it was updated).
    fn version(self) -> &'static str {
        match self {
            App::Sonarr => crate::sonarr::COMPAT_VERSION,
            App::Radarr => crate::radarr::COMPAT_VERSION,
        }
    }

    /// The hub's channel.
    fn channel(self) -> &'static broadcast::Sender<String> {
        match self {
            App::Sonarr => &SONARR,
            App::Radarr => &RADARR,
        }
    }
}

/// Sonarr's hub: every message for its connected clients, as finished `receiveMessage` invocations.
static SONARR: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| broadcast::channel(256).0);
/// Radarr's hub.
static RADARR: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| broadcast::channel(256).0);

/// A `receiveMessage` invocation of `SignalRMessage { name, body }`, as one protocol record.
fn invocation(name: &str, body: &Value) -> String {
    let msg = json!({ "type": 1, "target": "receiveMessage", "arguments": [{ "name": name, "body": body }] });
    format!("{msg}{RS}")
}

/// Sends `{name, body}` to every client of `app` (upstream's `BroadcastMessage`); nothing happens when nobody is
/// connected. `body` is usually `{"action": "updated" | "deleted" | "sync", "resource": {...}}`.
pub fn broadcast(app: App, name: &str, body: Value) {
    let _ = app.channel().send(invocation(name, &body));
}

/// Whether a client of `app` is connected (upstream's `IsConnected`).
pub fn is_connected(app: App) -> bool {
    app.channel().receiver_count() > 0
}

/// A new connection id: 22 URL-safe characters, like ASP.NET Core's.
fn connection_id() -> String {
    use base64::Engine;
    let bytes: [u8; 16] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `POST negotiate`: the transports on offer (WebSockets only).
async fn negotiate() -> Json<Value> {
    let id = connection_id();
    Json(json!({
        "negotiateVersion": 1,
        "connectionId": id,
        "connectionToken": id,
        "availableTransports": [{ "transport": "WebSockets", "transferFormats": ["Text", "Binary"] }],
    }))
}

/// Runs one client's connection: handshake, the `version` message, then hub messages and pings until it closes.
async fn run(mut socket: WebSocket, app: App) {
    // handshake: the client says which protocol; only JSON is spoken
    let ok = loop {
        match socket.recv().await {
            Some(Ok(Message::Text(t))) => break t.trim_end_matches(RS).contains("\"json\""),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return,
        }
    };
    if !ok {
        let _ = socket.send(Message::Text(format!("{{\"error\":\"Only the JSON protocol is supported.\"}}{RS}").into())).await;
        return;
    }
    if socket.send(Message::Text(format!("{{}}{RS}").into())).await.is_err() {
        return;
    }
    let mut rx = app.channel().subscribe();
    // `OnConnectedAsync`: upstream tells every client the version when one connects
    broadcast(app, "version", json!({ "version": app.version() }));
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(text) => if socket.send(Message::Text(text.into())).await.is_err() { return },
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = ping.tick() => if socket.send(Message::Text(format!("{{\"type\":6}}{RS}").into())).await.is_err() { return },
            incoming = socket.recv() => match incoming {
                // the client's pings and anything else it sends need no answer; type 7 is its close
                Some(Ok(Message::Text(t))) if t.contains("\"type\":7") => return,
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

/// The hub routes of `app` under its URL base (`/sonarr/signalr/…`, `/radarr/signalr/…`); they sit behind the API
/// key, which the web apps send as `access_token`.
pub fn routes<S: Clone + Send + Sync + 'static>(app: App) -> Router<S> {
    let base = match app {
        App::Sonarr => "/sonarr",
        App::Radarr => "/radarr",
    };
    Router::new()
        .route(&format!("{base}/signalr/messages/negotiate"), post(negotiate))
        .route(&format!("{base}/signalr/messages"), get(move |ws: WebSocketUpgrade| async move { upgrade(ws, app) }))
}

/// Accepts the WebSocket.
fn upgrade(ws: WebSocketUpgrade, app: App) -> Response {
    ws.on_upgrade(move |socket| run(socket, app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_receive_message_records() {
        let text = invocation("series", &json!({ "action": "updated", "resource": { "id": 1 } }));
        assert!(text.ends_with(RS));
        let v: Value = serde_json::from_str(text.trim_end_matches(RS)).unwrap();
        assert_eq!(v["target"], "receiveMessage");
        assert_eq!(v["arguments"][0]["name"], "series");
        assert_eq!(v["arguments"][0]["body"]["resource"]["id"], 1);
    }

    #[test]
    fn connection_ids_look_like_aspnet_ones() {
        assert_eq!(connection_id().len(), 22);
    }
}
