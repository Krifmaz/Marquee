//! `#/media` (admins): whether Jellyfin and Plex answer (with Scan now), each account's Jellyfin and Plex access,
//! libraries and end date (saved to both servers at once), and the servers' own users and shares, to link to
//! accounts that already exist.

use super::invites::{libraries_json, LibraryPicker};
use super::{ends_words, now_secs, s};
use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// One server's state in words: its name and version, why it doesn't answer, or "Not set up".
pub fn server_words(v: &Value) -> String {
    if v["configured"].as_bool() != Some(true) {
        "Not set up. Add it in Settings → Media servers.".into()
    } else if v["ok"].as_bool() == Some(true) {
        format!("{} · version {}", s(v, "name"), s(v, "version"))
    } else {
        s(v, "error")
    }
}

/// The servers card: each server's state, Scan now and Read again.
#[component]
fn Servers() -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let status = LocalResource::new(move || {
        tick.get();
        async { api::get("/media-servers").await.map_err(|e| e.message) }
    });
    let scan = move |_| {
        leptos::task::spawn_local(async move {
            match api::post("/media-servers/scan", json!({})).await {
                Ok(v) if v["ok"].as_bool() == Some(true) => toast("Jellyfin and Plex are scanning their libraries", "ok"),
                Ok(v) => {
                    toast(&v["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(" "), "err")
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let refresh = move |_| {
        leptos::task::spawn_local(async move {
            match api::post_empty("/media-servers/refresh").await {
                Ok(_) => toast("Marquee knows again what each server has", "ok"),
                Err(e) => toast(&e.message, "err"),
            }
            set_tick.update(|t| *t += 1);
        });
    };
    view! {
        <div class="card flat" id="media-servers">
            {move || match status.get() {
                None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
                Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
                Some(Ok(v)) => view! {
                    <div class="list">
                        {[("Jellyfin", "jellyfin"), ("Plex", "plex")].into_iter().map(|(name, k)| {
                            let ok = v[k]["ok"].as_bool() == Some(true);
                            view! { <div class="list-item" style="gap:10px"><span class="badge">{name}</span><div class="grow small" class:faint=!ok>{server_words(&v[k])}</div></div> }
                        }).collect_view()}
                        <div class="list-item" style="gap:10px"><span class="badge">"Mail"</span><div class="grow small faint">{if v["mail"]["configured"].as_bool() == Some(true) { "Set up: invites, password reset and reminders go out by mail." } else { "Not set up: people can't reset their own password. Add it in Settings → Media servers." }}</div></div>
                    </div>
                }.into_any(),
            }}
            <div class="row wrap mt" style="gap:8px">
                <button class="btn" on:click=scan>"Scan libraries now"</button>
                <button class="btn ghost" on:click=refresh>"Check what they have again"</button>
                <a class="btn ghost" href="#/settings?s=media">"Settings"</a>
            </div>
        </div>
    }
}

/// One account's access editor: Jellyfin and Plex on or off, their libraries, the Plex mail address and the
/// end date.
#[component]
fn AccessEditor(user: Value, reload: Callback<()>) -> impl IntoView {
    let id = user["id"].as_i64().unwrap_or(0);
    let access = LocalResource::new(move || async move { api::get(&format!("/users/{id}/media")).await.map_err(|e| e.message) });
    let (jf_on, plex_on) = (RwSignal::new(false), RwSignal::new(false));
    let (jf_libs, plex_libs) = (RwSignal::new(None::<Vec<String>>), RwSignal::new(None::<Vec<String>>));
    let plex_email = NodeRef::<leptos::html::Input>::new();
    let days = NodeRef::<leptos::html::Input>::new();
    let loaded = RwSignal::new(false);
    Effect::new(move |_| {
        if let Some(Ok(a)) = access.get() {
            let libs = |v: &Value| match v {
                Value::Array(l) => Some(l.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()),
                _ => None,
            };
            jf_on.set(a["jellyfin"]["active"].as_bool() == Some(true));
            plex_on.set(a["plex"]["active"].as_bool() == Some(true));
            jf_libs.set(libs(&a["jellyfin"]["libraries"]));
            plex_libs.set(libs(&a["plex"]["libraries"]));
            loaded.set(true);
        }
    });
    let email = StoredValue::new(s(&user, "email"));
    let save = move |_| {
        let mut body = json!({});
        body["jellyfin"] =
            if jf_on.get_untracked() { json!({ "libraries": libraries_json(&jf_libs.get_untracked()) }) } else { Value::Null };
        let pe = plex_email.get().map(|i| i.value()).unwrap_or_default();
        body["plex"] = if plex_on.get_untracked() {
            json!({ "libraries": libraries_json(&plex_libs.get_untracked()), "email": pe })
        } else {
            Value::Null
        };
        let d = days.get().map(|i| i.value()).unwrap_or_default();
        leptos::task::spawn_local(async move {
            if !d.trim().is_empty() {
                let b = match d.trim().parse::<i64>() {
                    Ok(0) | Err(_) => json!({ "expires_at": null }),
                    Ok(n) => json!({ "days": n }),
                };
                if let Err(e) = api::put(&format!("/users/{id}/expiry"), b).await {
                    return toast(&e.message, "err");
                }
            }
            match api::put(&format!("/users/{id}/media"), body).await {
                Ok(v) => {
                    let errors: Vec<&str> = v["errors"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    if errors.is_empty() {
                        toast("Saved on Jellyfin and Plex", "ok");
                    } else {
                        toast(&errors.join(" "), "err");
                    }
                }
                Err(e) => toast(&e.message, "err"),
            }
            reload.run(());
        });
    };
    let state = move || match access.get() {
        Some(Ok(a)) => {
            let line = |k: &str, name: &str| -> Option<String> {
                let v = &a[k];
                if v.is_null() {
                    return None;
                }
                let err = s(v, "error");
                Some(if !err.is_empty() {
                    format!("{name}: {err}")
                } else if v["password_pending"].as_bool() == Some(true) {
                    format!("{name}: {} gets their Marquee password the next time they sign in here.", s(v, "remote_name"))
                } else if v["linked"].as_bool() == Some(true) {
                    format!("{name}: {}", if s(v, "remote_name").is_empty() { "linked".to_string() } else { s(v, "remote_name") })
                } else {
                    format!("{name}: not made yet")
                })
            };
            let lines: Vec<String> = [line("jellyfin", "Jellyfin"), line("plex", "Plex")].into_iter().flatten().collect();
            view! { <div class="small faint">{lines.into_iter().map(|l| view! { <div>{l}</div> }).collect_view()}</div> }.into_any()
        }
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        None => view! { <span class="spinner"></span> }.into_any(),
    };
    view! {
        <div class="card mt" style="margin-left:0">
            {state}
            <Show when=move || loaded.get()>
                <label class="row mt" style="gap:6px"><input type="checkbox" prop:checked=move || jf_on.get() on:change=move |e| jf_on.set(event_target_checked(&e))/>"Jellyfin"</label>
                <Show when=move || jf_on.get()><div style="margin:8px 0 0 24px"><LibraryPicker server="jellyfin" chosen=jf_libs/></div></Show>
                <label class="row mt" style="gap:6px"><input type="checkbox" prop:checked=move || plex_on.get() on:change=move |e| plex_on.set(event_target_checked(&e))/>"Plex"</label>
                <Show when=move || plex_on.get()>
                    <div style="margin:8px 0 0 24px">
                        <label class="field"><span>"Their Plex account's mail address"</span><input class="input" type="email" value=move || email.get_value() node_ref=plex_email/></label>
                        <LibraryPicker server="plex" chosen=plex_libs/>
                    </div>
                </Show>
            </Show>
            <label class="field mt"><span>"Account ends in (days, 0: never; empty: unchanged)"</span><input class="input" type="number" min="0" node_ref=days/></label>
            <div class="row wrap" style="gap:8px">
                <button class="btn primary" on:click=save>"Save"</button>
            </div>
        </div>
    }
}

/// One account of the list, with its access editor behind Change.
fn account_row(u: Value, reload: Callback<()>) -> impl IntoView {
    let open = RwSignal::new(false);
    let ends = ends_words(u["expires_at"].as_f64(), now_secs());
    let off = u["disabled"].as_bool() == Some(true);
    let uu = u.clone();
    view! {
        <div class="list-item" style="flex-direction:column;align-items:stretch;gap:6px" data-user=u["id"].as_i64()>
            <div class="row" style="gap:10px">
                <div class="grow">
                    <div style="font-weight:650">{s(&u, "username")}{off.then(|| view! { " "<span class="badge">"Turned off"</span> })}</div>
                    <div class="small faint">{format!("{} · {}", s(&u, "role_label"), ends)}</div>
                </div>
                <button class="btn ghost" on:click=move |_| open.update(|o| *o = !*o)>{move || if open.get() { "Close" } else { "Change" }}</button>
            </div>
            <Show when=move || open.get()><AccessEditor user=uu.clone() reload=reload/></Show>
        </div>
    }
}

/// The server's own users or shares, each linked to an account or with a Link picker.
#[component]
fn LinkList(server: &'static str, accounts: Vec<Value>, reload: Callback<()>) -> impl IntoView {
    let list = LocalResource::new(move || async move { api::get(&format!("/media-servers/{server}/users")).await.map_err(|e| e.message) });
    move || match list.get() {
        None => view! { <span class="spinner"></span> }.into_any(),
        Some(Err(e)) => view! { <div class="small faint">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let rows = v.as_array().cloned().unwrap_or_default();
            if rows.is_empty() {
                return view! { <p class="small faint">"Nobody yet."</p> }.into_any();
            }
            let accounts = accounts.clone();
            view! {
                <div class="list">
                    {rows.into_iter().map(|r| {
                        let remote = s(&r, "id");
                        let pick = NodeRef::<leptos::html::Select>::new();
                        let link = move |_| {
                            let remote = remote.clone();
                            let Some(id) = pick.get().and_then(|p| p.value().parse::<i64>().ok()) else { return };
                            leptos::task::spawn_local(async move {
                                match api::post(&format!("/users/{id}/media/{server}/link"), json!({ "remote_id": remote })).await {
                                    Ok(_) => toast("Linked", "ok"),
                                    Err(e) => toast(&e.message, "err"),
                                }
                                reload.run(());
                            });
                        };
                        let who = r["username"].as_str().map(str::to_string);
                        let opts = accounts.iter().map(|a| view! { <option value=a["id"].as_i64().unwrap_or(0).to_string()>{s(a, "username")}</option> }).collect_view();
                        view! {
                            <div class="list-item" style="gap:10px;flex-wrap:wrap">
                                <div class="grow">{s(&r, "name")}{(r["admin"].as_bool() == Some(true)).then(|| view! { " "<span class="badge">"Jellyfin admin"</span> })}</div>
                                {match who {
                                    Some(w) => view! { <span class="small faint">{format!("Linked to {w}")}</span> }.into_any(),
                                    None => view! {
                                        <div class="row" style="gap:6px">
                                            <select class="input" style="width:auto" aria-label="Account" node_ref=pick>{opts}</select>
                                            <button class="btn" on:click=link>"Link"</button>
                                        </div>
                                    }.into_any(),
                                }}
                            </div>
                        }
                    }).collect_view()}
                </div>
            }
            .into_any()
        }
    }
}

/// `#/media`.
#[component]
pub fn MediaAccess() -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let reload = Callback::new(move |_| set_tick.update(|t| *t += 1));
    let users = LocalResource::new(move || {
        tick.get();
        async { api::get("/users").await.map_err(|e| e.message) }
    });
    let status = LocalResource::new(|| async { api::get("/media-servers").await.ok() });
    let set_up = move |k: &str| status.get().flatten().is_some_and(|v| v[k]["configured"].as_bool() == Some(true));
    view! {
        <h1>"Media access"</h1>
        <p class="muted" style="margin-top:0">"Accounts are made here and nowhere else. Jellyfin and Plex follow: made, turned off and removed with the account, with the libraries you pick. Admins see every library."</p>
        <Servers/>
        <h3 class="mt">"Accounts"</h3>
        <div class="card flat">
            {move || match users.get() {
                None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
                Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
                Some(Ok(v)) => view! { <div class="list" id="media-accounts">{v.as_array().cloned().unwrap_or_default().into_iter().map(|u| account_row(u, reload)).collect_view()}</div> }.into_any(),
            }}
        </div>
        {move || {
            let accounts = users.get().and_then(|r| r.ok()).and_then(|v| v.as_array().cloned()).unwrap_or_default();
            let jf = set_up("jellyfin").then(|| {
                let a = accounts.clone();
                view! { <h3 class="mt">"Jellyfin users"</h3><p class="small muted" style="margin-top:0">"Link a Jellyfin user to its account; it takes the Marquee password the next time the person signs in here."</p><div class="card flat"><LinkList server="jellyfin" accounts=a reload=reload/></div> }
            });
            let px = set_up("plex").then(|| {
                let a = accounts.clone();
                view! { <h3 class="mt">"Plex shares"</h3><div class="card flat"><LinkList server="plex" accounts=a reload=reload/></div> }
            });
            view! { {jf}{px} }
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::server_words;
    use serde_json::json;

    #[test]
    fn servers_read_plainly() {
        assert_eq!(server_words(&json!({ "configured": false })), "Not set up. Add it in Settings → Media servers.");
        assert_eq!(server_words(&json!({ "configured": true, "ok": true, "name": "Den", "version": "10.10.7" })), "Den · version 10.10.7");
        assert_eq!(
            server_words(&json!({ "configured": true, "ok": false, "error": "Jellyfin can't be reached" })),
            "Jellyfin can't be reached"
        );
    }
}
