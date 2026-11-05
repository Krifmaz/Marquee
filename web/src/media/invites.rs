//! `#/invites` (admins): every invite link with its uses and end date, Copy, Send by mail, Turn off and
//! Remove, and the New invite form with the role, servers and libraries new accounts get.

use super::{day_words, ends_words, now_secs, s};
use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// The value of the input `r`, "" when it is not drawn.
fn value(r: NodeRef<leptos::html::Input>) -> String {
    r.get().map(|i| i.value()).unwrap_or_default()
}

/// Uses in words: "1 of 1 used", "3 used (no limit)".
pub fn uses_words(used: i64, max: i64) -> String {
    if max == 0 {
        format!("{used} used (no limit)")
    } else {
        format!("{used} of {max} used")
    }
}

/// The servers of an invite in words.
fn servers_words(i: &Value) -> String {
    let lib = |v: &Value| match &v["libraries"] {
        Value::Array(a) => format!(" ({} libraries)", a.len()),
        _ => String::new(),
    };
    let mut parts = Vec::new();
    if !i["jellyfin"].is_null() {
        parts.push(format!("Jellyfin{}", lib(&i["jellyfin"])));
    }
    if !i["plex"].is_null() {
        parts.push(format!("Plex{}", lib(&i["plex"])));
    }
    if parts.is_empty() {
        "Marquee only".into()
    } else {
        parts.join(", ")
    }
}

/// The library ticks of one server: every library, or the ones ticked.
#[component]
pub fn LibraryPicker(server: &'static str, chosen: RwSignal<Option<Vec<String>>>) -> impl IntoView {
    let libs =
        LocalResource::new(move || async move { api::get(&format!("/media-servers/{server}/libraries")).await.map_err(|e| e.message) });
    move || match libs.get() {
        None => view! { <span class="spinner"></span> }.into_any(),
        Some(Err(e)) => view! { <div class="small faint">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let list = v.as_array().cloned().unwrap_or_default();
            view! {
                <div class="row wrap" style="gap:12px">
                    <label class="row" style="gap:6px"><input type="checkbox" checked=move || chosen.get().is_none()
                        on:change=move |e| chosen.set(if event_target_checked(&e) { None } else { Some(vec![]) })/>"Every library"</label>
                    {list.into_iter().map(|l| {
                        let id = s(&l, "id");
                        let (a, b) = (id.clone(), id.clone());
                        view! {
                            <label class="row" style="gap:6px" class:faint=move || chosen.get().is_none()>
                                <input type="checkbox" disabled=move || chosen.get().is_none()
                                    checked=move || chosen.get().is_none_or(|c| c.contains(&a))
                                    on:change=move |e| {
                                        let on = event_target_checked(&e);
                                        chosen.update(|c| if let Some(c) = c { c.retain(|x| *x != b); if on { c.push(b.clone()); } });
                                    }/>
                                {s(&l, "name")}
                            </label>
                        }
                    }).collect_view()}
                </div>
            }
            .into_any()
        }
    }
}

/// The JSON of a library choice: `"*"` or the ids.
pub fn libraries_json(c: &Option<Vec<String>>) -> Value {
    match c {
        None => json!("*"),
        Some(l) => json!(l),
    }
}

/// One invite of the list.
fn invite_row(i: Value, reload: Callback<()>) -> impl IntoView {
    let code = s(&i, "code");
    let link = s(&i, "link");
    let disabled = i["disabled"].as_bool() == Some(true);
    let why = i["why_not"].as_str().map(str::to_string);
    let ends = ends_words(i["expires_at"].as_f64(), now_secs());
    let account = match i["account_days"].as_i64().unwrap_or(0) {
        0 => "Accounts never end".to_string(),
        d => format!("Accounts last {d} days"),
    };
    let users: Vec<String> = i["uses"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| format!("{} ({})", s(u, "username"), day_words(u["at"].as_f64().unwrap_or(0.0))))
        .collect();
    let copy = {
        let link = link.clone();
        move |_| kit_leptos::clipboard::copy_then(&link, "Link copied")
    };
    let c1 = code.clone();
    let toggle = move |_| {
        let c = c1.clone();
        leptos::task::spawn_local(async move {
            if let Err(e) = api::patch(&format!("/invites/{c}"), json!({ "disabled": !disabled })).await {
                toast(&e.message, "err");
            }
            reload.run(());
        });
    };
    let c2 = code.clone();
    let sending = RwSignal::new(false);
    let to = NodeRef::<leptos::html::Input>::new();
    let send = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let (c, to) = (c2.clone(), value(to));
        leptos::task::spawn_local(async move {
            match api::post(&format!("/invites/{c}/send"), json!({ "email": to.trim() })).await {
                Ok(_) => toast(&format!("Sent to {}", to.trim()), "ok"),
                Err(e) => toast(&e.message, "err"),
            }
            reload.run(());
        });
    };
    let c3 = code.clone();
    let remove = move |_| {
        let c = c3.clone();
        leptos::task::spawn_local(async move {
            let q = kit_rules::dialogs::Confirm::new("Remove this invite?")
                .text("The link stops working. Accounts made with it stay.")
                .ok("Remove")
                .danger(true);
            if kit_leptos::modal::confirm(q).await {
                if let Err(e) = api::del(&format!("/invites/{c}")).await {
                    toast(&e.message, "err");
                }
                reload.run(());
            }
        });
    };
    let label = if s(&i, "label").is_empty() { "Invite".to_string() } else { s(&i, "label") };
    view! {
        <div class="list-item" data-invite=code.clone() style="flex-wrap:wrap;gap:10px">
            <div class="grow" style="min-width:240px">
                <div style="font-weight:650">{label}" "{why.clone().map(|_| view! { <span class="badge">"Can't be used"</span> })}</div>
                <div class="small mono ellipsis">{link.clone()}</div>
                <div class="small faint">{format!("{} · {} · {} · {} · {}", uses_words(i["used"].as_i64().unwrap_or(0), i["max_uses"].as_i64().unwrap_or(0)), ends, account, servers_words(&i), if s(&i, "role") == "viewer" { "Read-only" } else { "Can request" })}</div>
                {(!users.is_empty()).then(|| view! { <div class="small faint">{format!("Used by {}", users.join(", "))}</div> })}
                {why.map(|w| view! { <div class="small faint">{w}</div> })}
                <Show when=move || sending.get()>
                    <form class="row wrap mt" style="gap:6px" on:submit=send.clone()>
                        <input class="input grow" type="email" required placeholder="friend@example.com" aria-label="Mail address" node_ref=to/>
                        <button class="btn primary" type="submit">"Send"</button>
                    </form>
                </Show>
            </div>
            <div class="row wrap" style="gap:6px">
                <button class="btn" on:click=copy>"Copy link"</button>
                <button class="btn ghost" on:click=move |_| sending.update(|v| *v = !*v)>"Send by mail"</button>
                <button class="btn ghost" on:click=toggle>{if disabled { "Turn on" } else { "Turn off" }}</button>
                <button class="btn ghost danger" on:click=remove>"Remove"</button>
            </div>
        </div>
    }
}

/// `#/invites`.
#[component]
pub fn Invites() -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let reload = Callback::new(move |_| set_tick.update(|t| *t += 1));
    let list = LocalResource::new(move || {
        tick.get();
        async { api::get("/invites").await.map_err(|e| e.message) }
    });
    let servers = LocalResource::new(|| async { api::get("/media-servers").await.ok() });
    let (label, uses, days, account_days, send_to) = (NodeRef::new(), NodeRef::new(), NodeRef::new(), NodeRef::new(), NodeRef::new());
    let role = NodeRef::<leptos::html::Select>::new();
    let (jf_on, plex_on) = (RwSignal::new(false), RwSignal::new(false));
    let (jf_libs, plex_libs) = (RwSignal::new(None::<Vec<String>>), RwSignal::new(None::<Vec<String>>));
    let num = |r: NodeRef<leptos::html::Input>, d: i64| value(r).trim().parse::<i64>().unwrap_or(d);
    let add = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let mut body = json!({
            "label": value(label), "max_uses": num(uses, 1), "expires_in_days": num(days, 7), "account_days": num(account_days, 0),
            "role": role.get().map(|r| r.value()).unwrap_or_default(), "send_to": value(send_to),
        });
        if jf_on.get_untracked() {
            body["jellyfin"] = json!({ "libraries": libraries_json(&jf_libs.get_untracked()) });
        }
        if plex_on.get_untracked() {
            body["plex"] = json!({ "libraries": libraries_json(&plex_libs.get_untracked()) });
        }
        leptos::task::spawn_local(async move {
            match api::post("/invites", body).await {
                Ok(i) => {
                    kit_leptos::clipboard::copy_then(&s(&i, "link"), "Invite made; the link is copied");
                    reload.run(());
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let rows = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                view! { <div class="empty"><p class="muted">"No invites yet. Make one below and send the link to the people you want."</p></div> }.into_any()
            } else {
                view! { <div class="list" id="invites">{items.into_iter().map(|i| invite_row(i, reload)).collect_view()}</div> }.into_any()
            }
        }
    };
    let set_up = move |k: &'static str| servers.get().flatten().is_some_and(|v| v[k]["configured"].as_bool() == Some(true));
    view! {
        <h1>"Invites"</h1>
        <p class="muted" style="margin-top:0">"Anyone with an invite link can make their own account here, with Jellyfin and Plex if the invite gives them."</p>
        <div class="card flat mt">{rows}</div>
        <form class="card mt" id="invite-new" on:submit=add>
            <h3 style="margin-top:0">"New invite"</h3>
            <div class="row wrap" style="gap:12px">
                <label class="field grow"><span>"Note (only admins see it)"</span><input class="input" placeholder="Family" node_ref=label/></label>
                <label class="field"><span>"How many people (0: no limit)"</span><input class="input" type="number" min="0" value="1" node_ref=uses/></label>
                <label class="field"><span>"Link works for (days, 0: always)"</span><input class="input" type="number" min="0" value="7" node_ref=days/></label>
                <label class="field"><span>"Accounts last (days, 0: always)"</span><input class="input" type="number" min="0" value="0" node_ref=account_days/></label>
                <label class="field"><span>"Role"</span><select class="input" node_ref=role><option value="requester" selected>"Can request"</option><option value="viewer">"Read-only"</option></select></label>
            </div>
            {move || set_up("jellyfin").then(|| view! {
                <label class="row mt" style="gap:6px"><input type="checkbox" on:change=move |e| jf_on.set(event_target_checked(&e))/>"A Jellyfin account, with the same password"</label>
                <Show when=move || jf_on.get()><div style="margin:8px 0 0 24px"><LibraryPicker server="jellyfin" chosen=jf_libs/></div></Show>
            })}
            {move || set_up("plex").then(|| view! {
                <label class="row mt" style="gap:6px"><input type="checkbox" on:change=move |e| plex_on.set(event_target_checked(&e))/>"Share Plex with them"</label>
                <Show when=move || plex_on.get()><div style="margin:8px 0 0 24px"><LibraryPicker server="plex" chosen=plex_libs/></div></Show>
            })}
            <label class="field mt"><span>"Send the link to (optional)"</span><input class="input" type="email" placeholder="friend@example.com" node_ref=send_to/></label>
            <button class="btn primary mt" type="submit">"Make the invite"</button>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::{servers_words, uses_words};
    use serde_json::json;

    #[test]
    fn invites_read_plainly() {
        assert_eq!(uses_words(0, 1), "0 of 1 used");
        assert_eq!(uses_words(3, 0), "3 used (no limit)");
        assert_eq!(servers_words(&json!({ "jellyfin": null, "plex": null })), "Marquee only");
        assert_eq!(
            servers_words(&json!({ "jellyfin": { "libraries": "*" }, "plex": { "libraries": ["1", "2"] } })),
            "Jellyfin, Plex (2 libraries)"
        );
    }
}
