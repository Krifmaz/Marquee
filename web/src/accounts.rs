//! Settings → Accounts and Settings → Folders (admins): the people who use Marquee, their roles, request limits
//! and auto-approve, and the root folders for movies, TV and anime. Words and markup are the kit's.

use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A number field of a JSON object, 0 when missing.
fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

/// A request limit in words: "No limit" or "5 every 7 days".
pub fn limit_words(limit: i64, days: i64) -> String {
    match (limit, days) {
        (0, _) => "No limit".into(),
        (l, 1) => format!("{l} a day"),
        (l, 7) => format!("{l} a week"),
        (l, d) => format!("{l} every {d} days"),
    }
}

/// A size in bytes in words ("1.2 TB free").
pub fn free_words(bytes: Option<u64>) -> String {
    let Some(b) = bytes else { return "Free space unknown".into() };
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B free")
    } else {
        format!("{v:.1} {} free", units[i])
    }
}

/// The value of the input `r`, "" when it is not drawn.
fn value(r: NodeRef<leptos::html::Input>) -> String {
    r.get().map(|i| i.value()).unwrap_or_default()
}

/// The value of the select `r`.
fn chosen(r: NodeRef<leptos::html::Select>) -> String {
    r.get().map(|i| i.value()).unwrap_or_default()
}

/// One account of the list: role, limits, and the admin's buttons.
fn account_row(u: Value, reload: Callback<()>) -> impl IntoView {
    let id = n(&u, "id");
    let owner = s(&u, "role") == "owner";
    let disabled = u["disabled"].as_bool().unwrap_or(false);
    let movies = limit_words(n(&u, "movie_quota_limit"), n(&u, "movie_quota_days"));
    let tv = limit_words(n(&u, "tv_quota_limit"), n(&u, "tv_quota_days"));
    let auto = match (u["auto_approve_movies"].as_bool() == Some(true), u["auto_approve_tv"].as_bool() == Some(true)) {
        (true, true) => "Movies and TV",
        (true, false) => "Movies",
        (false, true) => "TV",
        _ => "No",
    };
    let name = s(&u, "username");
    let set_role = move |e: leptos::ev::Event| {
        let role = event_target_value(&e);
        leptos::task::spawn_local(async move {
            match api::patch(&format!("/users/{id}"), json!({ "role": role })).await {
                Ok(_) => toast("Role changed", "ok"),
                Err(e) => toast(&e.message, "err"),
            }
            reload.run(());
        });
    };
    let toggle = move |_| {
        leptos::task::spawn_local(async move {
            if let Err(e) = api::patch(&format!("/users/{id}"), json!({ "disabled": !disabled })).await {
                toast(&e.message, "err");
            }
            reload.run(());
        });
    };
    let remove = {
        let name = name.clone();
        move |_| {
            let name = name.clone();
            leptos::task::spawn_local(async move {
                let q = kit_rules::dialogs::Confirm::new(format!("Remove {name}?"))
                    .text("They can't sign in any more. Their requests stay.")
                    .ok("Remove")
                    .danger(true);
                if !kit_leptos::modal::confirm(q).await {
                    return;
                }
                match api::del(&format!("/users/{id}")).await {
                    Ok(_) => toast(&format!("{name} was removed"), "ok"),
                    Err(e) => toast(&e.message, "err"),
                }
                reload.run(());
            });
        }
    };
    let role = s(&u, "role");
    let role_cell = if owner {
        view! { <span class="badge">"Owner"</span> }.into_any()
    } else {
        let opt = move |v: &'static str, label: &'static str| view! { <option value=v selected=role == v>{label}</option> };
        view! {
            <select class="input" style="width:auto" on:change=set_role aria-label="Role">
                {opt("admin", "Admin")}{opt("requester", "Can request")}{opt("viewer", "Read-only")}
            </select>
        }
        .into_any()
    };
    view! {
        <tr data-user=id>
            <td><div style="font-weight:650">{name}</div><div class="small faint">{s(&u, "email")}</div></td>
            <td>{role_cell}</td>
            <td class="small">{movies}</td>
            <td class="small">{tv}</td>
            <td class="small">{auto}</td>
            <td>
                {(!owner).then(|| view! {
                    <div class="row" style="gap:6px;justify-content:flex-end">
                        <button class="btn ghost" on:click=toggle>{if disabled { "Turn on" } else { "Turn off" }}</button>
                        <button class="btn ghost danger" on:click=remove.clone()>"Remove"</button>
                    </div>
                })}
            </td>
        </tr>
    }
}

/// Settings → Accounts: every account and the form for a new one.
#[component]
pub fn Accounts() -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let reload = Callback::new(move |_| set_tick.update(|t| *t += 1));
    let list = LocalResource::new(move || {
        tick.get();
        async { api::get("/users").await.map_err(|e| e.message) }
    });
    let (username, password, email) = (NodeRef::new(), NodeRef::new(), NodeRef::new());
    let (movie_limit, tv_limit) = (NodeRef::new(), NodeRef::new());
    let (auto_movies, auto_tv) = (NodeRef::<leptos::html::Input>::new(), NodeRef::<leptos::html::Input>::new());
    let role = NodeRef::new();
    let add = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let num = |r: NodeRef<leptos::html::Input>| value(r).trim().parse::<i64>().unwrap_or(0);
        let checked = |r: NodeRef<leptos::html::Input>| r.get().map(|i| i.checked()).unwrap_or(false);
        let body = json!({
            "username": value(username), "password": value(password), "email": value(email), "role": chosen(role),
            "movie_quota_limit": num(movie_limit), "tv_quota_limit": num(tv_limit),
            "auto_approve_movies": checked(auto_movies), "auto_approve_tv": checked(auto_tv),
        });
        leptos::task::spawn_local(async move {
            match api::post("/users", body).await {
                Ok(u) => {
                    toast(&format!("{} can sign in now", s(&u, "username")), "ok");
                    for r in [username, password, email] {
                        if let Some(i) = r.get() {
                            i.set_value("");
                        }
                    }
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
            let users = v.as_array().cloned().unwrap_or_default();
            view! {
                <div class="table-wrap"><table class="table" id="accounts">
                    <thead><tr><th>"Account"</th><th>"Role"</th><th>"Movies"</th><th>"TV seasons"</th><th>"Auto-approve"</th><th></th></tr></thead>
                    <tbody>{users.into_iter().map(|u| account_row(u, reload)).collect_view()}</tbody>
                </table></div>
            }
            .into_any()
        }
    };
    view! {
        <div class="card flat">{rows}</div>
        <form class="card mt" id="account-new" on:submit=add>
            <h3 style="margin-top:0">"New account"</h3>
            <div class="row wrap" style="gap:12px">
                <label class="field grow"><span>"Username"</span><input class="input" required node_ref=username autocomplete="off"/></label>
                <label class="field grow"><span>"Password"</span><input class="input" type="password" required minlength="8" node_ref=password autocomplete="new-password"/></label>
                <label class="field grow"><span>"Email (for notifications)"</span><input class="input" type="email" node_ref=email/></label>
            </div>
            <div class="row wrap" style="gap:12px">
                <label class="field"><span>"Role"</span>
                    <select class="input" node_ref=role>
                        <option value="requester" selected>"Can request"</option>
                        <option value="viewer">"Read-only"</option>
                        <option value="admin">"Admin"</option>
                    </select>
                </label>
                <label class="field"><span>"Movies a week (0: no limit)"</span><input class="input" type="number" min="0" value="0" node_ref=movie_limit/></label>
                <label class="field"><span>"TV seasons a week (0: no limit)"</span><input class="input" type="number" min="0" value="0" node_ref=tv_limit/></label>
            </div>
            <div class="row wrap" style="gap:16px">
                <label class="row" style="gap:6px"><input type="checkbox" node_ref=auto_movies/>"Approve their movie requests by itself"</label>
                <label class="row" style="gap:6px"><input type="checkbox" node_ref=auto_tv/>"Approve their TV requests by itself"</label>
            </div>
            <button class="btn primary mt" type="submit">"Add account"</button>
        </form>
    }
}

/// Settings → Folders: the root folders for movies and TV.
#[component]
pub fn Folders() -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let list = LocalResource::new(move || {
        tick.get();
        async { api::get("/root-folders").await.map_err(|e| e.message) }
    });
    let (path, kind) = (NodeRef::new(), NodeRef::new());
    let add = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let body = json!({ "kind": chosen(kind), "path": value(path) });
        leptos::task::spawn_local(async move {
            match api::post("/root-folders", body).await {
                Ok(_) => {
                    toast("Folder added", "ok");
                    set_tick.update(|t| *t += 1);
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let remove = move |id: i64, p: String| {
        leptos::task::spawn_local(async move {
            let q = kit_rules::dialogs::Confirm::new(format!("Stop using {p}?"))
                .text("The files in it stay where they are.")
                .ok("Stop using")
                .danger(true);
            if kit_leptos::modal::confirm(q).await {
                if let Err(e) = api::del(&format!("/root-folders/{id}")).await {
                    toast(&e.message, "err");
                }
                set_tick.update(|t| *t += 1);
            }
        });
    };
    let rows = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let folders = v.as_array().cloned().unwrap_or_default();
            if folders.is_empty() {
                return view! { <div class="empty"><p class="muted">"No folders yet. Add where your movies and TV live, for example /data/media/movies, /data/media/tv and /data/media/anime."</p></div> }.into_any();
            }
            view! {
                <div class="list" id="folders">
                    {folders.into_iter().map(|f| {
                        let (id, p) = (n(&f, "id"), s(&f, "path"));
                        let ok = f["accessible"].as_bool() == Some(true);
                        let kind = match s(&f, "kind").as_str() { "tv" => "TV", "anime" => "Anime", _ => "Movies" };
                        let free = free_words(f["free_space"].as_u64());
                        let pp = p.clone();
                        view! {
                            <div class="list-item" style="gap:10px">
                                <span class="badge">{kind}</span>
                                <div class="grow"><div class="mono">{p}</div><div class="small faint">{if ok { free } else { "The server can't open this folder".into() }}</div></div>
                                <button class="btn ghost danger" on:click=move |_| remove(id, pp.clone())>"Stop using"</button>
                            </div>
                        }
                    }).collect_view()}
                </div>
            }
            .into_any()
        }
    };
    view! {
        <div class="card flat">{rows}</div>
        <form class="card mt" on:submit=add>
            <h3 style="margin-top:0">"Add a folder"</h3>
            <p class="small muted" style="margin-top:0">"Keep downloads and media on the same disk (for example /data/torrents and /data/media) so imports are instant and take no extra space."</p>
            <div class="row wrap" style="gap:12px">
                <label class="field"><span>"For"</span><select class="input" node_ref=kind><option value="movie">"Movies"</option><option value="tv">"TV"</option><option value="anime">"Anime"</option></select></label>
                <label class="field grow"><span>"Folder"</span><input class="input mono" required placeholder="/data/media/movies" node_ref=path/></label>
            </div>
            <button class="btn primary" type="submit">"Add folder"</button>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::{free_words, limit_words};

    #[test]
    fn limits_and_space_read_plainly() {
        assert_eq!(limit_words(0, 7), "No limit");
        assert_eq!(limit_words(5, 7), "5 a week");
        assert_eq!(limit_words(2, 1), "2 a day");
        assert_eq!(limit_words(3, 30), "3 every 30 days");
        assert_eq!(free_words(Some(1_234_000_000_000)), "1.2 TB free");
        assert_eq!(free_words(Some(512)), "512 B free");
        assert_eq!(free_words(None), "Free space unknown");
    }
}
