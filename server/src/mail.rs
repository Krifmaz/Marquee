//! Sending mail: invites, password reset links, account expiry reminders and bug report replies. Plain text
//! mail through the SMTP server in Settings → Media servers → Mail (`smtp_*`).
//!
//! The mail server `file` (`MARQUEE_SMTP_HOST=file`) writes each mail to `<data>/outbox/` instead of sending
//! it: for tests, and to see what Marquee would send.

use crate::AppState;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// Whether mail is set up: a mail server and a From address.
pub fn enabled(st: &AppState) -> bool {
    let host = st.settings.str("smtp_host");
    !host.trim().is_empty() && (host == "file" || !st.settings.str("smtp_from").trim().is_empty())
}

/// One mail, as [`send`] takes it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mail {
    /// the address it goes to
    pub to: String,
    /// the subject line
    pub subject: String,
    /// the text
    pub text: String,
}

/// Sends `mail`; the reason in plain words when it can't.
pub async fn send(st: &AppState, mail: Mail) -> Result<(), String> {
    if !enabled(st) {
        return Err("Mail isn't set up. An admin can set it up in Settings → Media servers → Mail.".into());
    }
    let to: Mailbox = mail.to.trim().parse().map_err(|_| format!("{} isn't a mail address", mail.to.trim()))?;
    let host = st.settings.str("smtp_host");
    if host == "file" {
        return to_outbox(st, &mail);
    }
    let from: Mailbox =
        st.settings.str("smtp_from").trim().parse().map_err(|_| "The From address in Settings → Mail isn't a mail address".to_string())?;
    let msg = Message::builder()
        .from(from)
        .to(to)
        .subject(mail.subject)
        .header(lettre::message::header::ContentType::TEXT_PLAIN)
        .body(mail.text)
        .map_err(|e| format!("The mail can't be written: {e}"))?;
    let port = st.settings.int("smtp_port").clamp(1, 65535) as u16;
    let builder = match st.settings.str("smtp_security").as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(host.trim()),
        "none" => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host.trim())),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host.trim()),
    }
    .map_err(|e| format!("The mail server {host} can't be used: {e}"))?;
    let mut builder = builder.port(port).timeout(Some(std::time::Duration::from_secs(20)));
    let user = st.settings.str("smtp_user");
    if !user.trim().is_empty() {
        builder = builder.credentials(Credentials::new(user.trim().to_string(), st.settings.str("smtp_password")));
    }
    builder.build().send(msg).await.map(|_| ()).map_err(|e| format!("The mail server {host} didn't take the mail: {e}"))
}

/// Writes `mail` to `<data>/outbox/<time>-<n>.json` (the mail server `file`).
fn to_outbox(st: &AppState, mail: &Mail) -> Result<(), String> {
    let dir = st.data_dir.join("outbox");
    std::fs::create_dir_all(&dir).map_err(|e| format!("The outbox folder can't be made: {e}"))?;
    let name = format!("{:020}-{}.json", (crate::auth::now() * 1e6) as u64, crate::auth::random_token(4));
    std::fs::write(dir.join(name), serde_json::to_vec(mail).unwrap_or_default()).map_err(|e| format!("The mail can't be written: {e}"))
}

/// Every mail in `<data>/outbox/`, oldest first (the mail server `file`).
pub fn outbox(st: &AppState) -> Vec<Mail> {
    let Ok(dir) = std::fs::read_dir(st.data_dir.join("outbox")) else { return vec![] };
    let mut names: Vec<_> = dir.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    names.iter().filter_map(|p| std::fs::read(p).ok()).filter_map(|b| serde_json::from_slice(&b).ok()).collect()
}

/// Sends `mail` in the background and logs a failure (for mails nobody waits for).
pub fn send_later(st: &AppState, mail: Mail) {
    let st = st.clone();
    tokio::spawn(async move {
        let to = mail.to.clone();
        if let Err(e) = send(&st, mail).await {
            tracing::warn!("A mail to {to} wasn't sent: {e}");
        }
    });
}

/// The address people open Marquee at, without a trailing `/` (the `public_url` setting); "" when not set.
pub fn public_url(st: &AppState) -> String {
    st.settings.str("signup_public_url").trim().trim_end_matches('/').to_string()
}
