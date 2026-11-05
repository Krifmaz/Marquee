//! `#/activity`: every download Marquee started through Lantern, with how far along it is and, when it is stuck,
//! why in words. Admins can stop one or ask Lantern about all of them now.

use crate::movies::s;
use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::Value;

/// Where a download stands, in words, and the badge style for it.
pub fn standing(d: &Value) -> (&'static str, &'static str) {
    match d.get("status").and_then(Value::as_str).unwrap_or("") {
        "queued" => ("Waiting", "info"),
        "downloading" => ("Downloading", "info"),
        "paused" => ("Paused", ""),
        "completed" | "importing" => ("Putting in the library", "info"),
        "imported" => ("In the library", "ok"),
        "importBlocked" => ("Needs a look", "warn"),
        "review" => ("Waiting in the anime Review queue", "warn"),
        "turnedDown" => ("Turned down, searching again", ""),
        "failed" => ("Failed", "err"),
        "removed" => ("Stopped", ""),
        _ => ("Unknown", ""),
    }
}

/// Bytes in words (`1.4 GB`).
pub fn size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b >= 1_073_741_824.0 {
        format!("{:.1} GB", b / 1_073_741_824.0)
    } else if b >= 1_048_576.0 {
        format!("{:.0} MB", b / 1_048_576.0)
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

/// `#/activity`.
#[component]
pub fn Activity(admin: bool) -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let list = LocalResource::new(move || {
        tick.get();
        async { api::get("/downloads").await.map_err(|e| e.message) }
    });
    let reload = Callback::new(move |_: ()| set_tick.update(|t| *t += 1));
    let check = move |_| {
        leptos::task::spawn_local(async move {
            match api::post_empty("/lantern/sync").await {
                Ok(_) => toast("Lantern was asked about every download", "ok"),
                Err(e) => toast(&e.message, "err"),
            }
            reload.run(());
        });
    };
    let rows = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return view! { <div class="empty"><p class="muted">"Nothing has been downloaded yet. Search for a movie or series to start."</p></div> }
                    .into_any();
            }
            view! {
                <div class="table-wrap"><table class="table" id="downloads">
                    <thead><tr><th>"Release"</th><th>"Quality"</th><th>"Size"</th><th>"Where it stands"</th><th></th></tr></thead>
                    <tbody>{items.into_iter().map(|d| row(d, admin, reload)).collect_view()}</tbody>
                </table></div>
            }
            .into_any()
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Activity"</h1>
            {admin.then(|| view! { <button class="btn" on:click=check>"Ask Lantern now"</button> })}
        </div>
        {rows}
    }
}

/// One download's row (the Anime tab's Activity uses it too).
pub fn row(d: Value, admin: bool, reload: Callback<()>) -> impl IntoView {
    let id = d.get("id").and_then(Value::as_i64).unwrap_or(0);
    let title = s(&d, "title");
    let (words, kind) = standing(&d);
    let pct = (d.get("progress").and_then(Value::as_f64).unwrap_or(0.0) * 100.0).round() as i64;
    let status = s(&d, "status");
    let active = !matches!(status.as_str(), "imported" | "removed");
    let detail = if status == "downloading" { format!("{pct}%") } else { String::new() };
    let message = s(&d, "message");
    let stop = {
        let title = title.clone();
        move |_| {
            let title = title.clone();
            leptos::task::spawn_local(async move {
                let q = kit_rules::dialogs::Confirm::new(format!("Stop {title}?"))
                    .text("Lantern stops the download and removes what it downloaded so far.")
                    .ok("Stop")
                    .danger(true);
                if !kit_leptos::modal::confirm(q).await {
                    return;
                }
                match api::del(&format!("/downloads/{id}")).await {
                    Ok(_) => toast(&format!("{title} was stopped"), "ok"),
                    Err(e) => toast(&e.message, "err"),
                }
                reload.run(());
            });
        }
    };
    view! {
        <tr>
            <td class="mono small" style="word-break:break-all">{title.clone()}</td>
            <td class="small">{s(&d, "quality")}</td>
            <td class="small">{size(d.get("size").and_then(Value::as_i64).unwrap_or(0))}</td>
            <td>
                <span class=format!("badge {kind}")>{words}</span>" "<span class="small faint">{detail}</span>
                {(!message.is_empty()).then(|| view! { <div class="small muted">{message.clone()}</div> })}
            </td>
            <td>{(admin && active).then(|| view! { <button class="btn ghost" on:click=stop>"Stop"</button> })}</td>
        </tr>
    }
}

#[cfg(test)]
mod tests {
    use super::{size, standing};
    use serde_json::json;

    #[test]
    fn downloads_say_where_they_stand() {
        assert_eq!(standing(&json!({ "status": "importBlocked" })).0, "Needs a look");
        assert_eq!(standing(&json!({ "status": "imported" })).0, "In the library");
        assert_eq!(size(1_610_612_736), "1.5 GB");
        assert_eq!(size(5 * 1_048_576), "5 MB");
    }
}
