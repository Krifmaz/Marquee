//! `#/trash`: TRaSH Guides, the quality defaults. Admins pick what movies, TV and anime get in a short wizard,
//! look for TRaSH updates, and decide about updates to things they edited themselves (TRaSH's version or
//! theirs). Everyone signed in sees what changed. The shell already asks [`page`] for every route it doesn't
//! know and shows [`nav`]'s links.

use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// This track's page for `path`, `None` when the path isn't one of its own.
pub fn page(path: &str, _query: &str, admin: bool) -> Option<AnyView> {
    (path == "/trash").then(|| view! { <Trash admin=admin/> }.into_any())
}

/// This track's sidebar links.
pub fn nav(_admin: bool) -> Vec<(&'static str, &'static str)> {
    vec![("/trash", "Quality")]
}

/// A change's status in words and its badge style.
pub fn status_words(status: &str) -> (&'static str, &'static str) {
    match status {
        "added" => ("Added", "ok"),
        "updated" => ("Updated", "ok"),
        "waiting" => ("Waiting for you", "warn"),
        "taken" => ("TRaSH's version used", "info"),
        "kept" => ("Your version kept", ""),
        _ => ("Replaced by a newer change", ""),
    }
}

/// What a change is about, in words.
pub fn item_words(item: &str, kind: &str) -> String {
    let what = match item {
        "format" => "Custom format",
        "profile" => "Quality profile",
        "sizes" => "Quality sizes",
        _ => "Naming",
    };
    let kind = match kind {
        "movie" => "movies",
        "anime" => "anime",
        _ => "TV",
    };
    format!("{what} · {kind}")
}

/// A short commit id and its date in words (`d0f1a6c, 3 Nov 2026`).
pub fn version_words(commit: &str, date: &str) -> String {
    let short: String = commit.chars().take(7).collect();
    let day = date.get(..10).unwrap_or(date);
    if short.is_empty() {
        "none yet".into()
    } else {
        format!("{short}, {day}")
    }
}

/// `#/trash`.
#[component]
pub fn Trash(admin: bool) -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let reload = Callback::new(move |_: ()| set_tick.update(|t| *t += 1));
    let status = LocalResource::new(move || {
        tick.get();
        async { api::get("/trash").await.map_err(|e| e.message) }
    });
    let changes = LocalResource::new(move || {
        tick.get();
        async { api::get("/trash/changes?limit=100").await.map_err(|e| e.message) }
    });
    let sync = move |_| {
        leptos::task::spawn_local(async move {
            toast("Looking for TRaSH Guides updates…", "info");
            match api::post_empty("/trash/sync").await {
                Ok(v) if v["new"] == true => {
                    toast(&format!("Updated: {} changes made, {} waiting for you", v["changed"], v["waiting"]), "ok")
                }
                Ok(_) => toast("Already up to date", "ok"),
                Err(e) => toast(&e.message, "err"),
            }
            reload.run(());
        });
    };
    let head = move || {
        match status.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => view! {
            <div class="card" style="margin-bottom:18px">
                <p style="margin-top:0">
                    "Quality profiles, custom formats, sizes and naming follow the "
                    <a href="https://trash-guides.info/" target="_blank" rel="noopener">"TRaSH Guides"</a>
                    ". Marquee updates what you haven't changed and asks before touching what you have."
                </p>
                <div class="small muted">"TRaSH Guides version: "{version_words(&s(&v, "commit"), &s(&v, "date"))}</div>
                <div class="small muted">
                    {match (v["auto"] == true, s(&v, "checkedAt")) {
                        (false, _) => "Automatic updates are off (Settings → TRaSH Guides).".to_string(),
                        (true, c) if c.is_empty() => format!("Looks for updates every {} hours.", v["syncHours"]),
                        (true, c) => format!("Looks for updates every {} hours. Last looked: {}.", v["syncHours"], c.get(..16).unwrap_or(&c).replace('T', " ")),
                    }}
                </div>
                {v["lastError"].as_str().map(|e| view! { <div class="error-box" style="margin-top:10px">{format!("The last update failed: {e}")}</div> })}
            </div>
            {waiting_list(&v, admin, reload)}
            {admin.then(|| view! { <Wizard status=v.clone() reload=reload/> })}
        }
        .into_any(),
    }
    };
    let history = move || match changes.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items: Vec<Value> = v.as_array().cloned().unwrap_or_default().into_iter().filter(|c| c["status"] != "waiting").collect();
            if items.is_empty() {
                return view! { <p class="muted">"Nothing has changed yet."</p> }.into_any();
            }
            view! {
                <div class="table-wrap"><table class="table" id="trash-changes">
                    <thead><tr><th>"When"</th><th>"What"</th><th>"Changes"</th><th>"Result"</th></tr></thead>
                    <tbody>{items.into_iter().map(|c| {
                        let (words, badge) = status_words(&s(&c, "status"));
                        let lines: Vec<String> = c["lines"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect();
                        view! {
                            <tr>
                                <td class="small nowrap">{s(&c, "at").get(..10).unwrap_or("").to_string()}</td>
                                <td><div>{s(&c, "name")}</div><div class="small faint">{item_words(&s(&c, "item"), &s(&c, "kind"))}</div></td>
                                <td class="small">{lines.into_iter().map(|l| view! { <div>{l}</div> }).collect_view()}</td>
                                <td><span class=format!("badge {badge}")>{words}</span></td>
                            </tr>
                        }
                    }).collect_view()}</tbody>
                </table></div>
            }
            .into_any()
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Quality"</h1>
            {admin.then(|| view! { <button class="btn" id="trash-sync" on:click=sync>"Look for updates now"</button> })}
        </div>
        {head}
        <h2>"What changed"</h2>
        {history}
    }
}

/// The changes that wait for an admin: TRaSH changed something the person edited.
fn waiting_list(status: &Value, admin: bool, reload: Callback<()>) -> impl IntoView {
    let count = status["waiting"].as_i64().unwrap_or(0);
    let waiting = LocalResource::new(move || async move {
        if count == 0 {
            return Ok(json!([]));
        }
        api::get("/trash/changes?status=waiting").await.map_err(|e| e.message)
    });
    move || {
        let list = match waiting.get() {
            Some(Ok(v)) => v.as_array().cloned().unwrap_or_default(),
            Some(Err(e)) => return view! { <div class="error-box">{e}</div> }.into_any(),
            None => vec![],
        };
        if list.is_empty() {
            return ().into_any();
        }
        view! {
            <div class="card" id="trash-waiting" style="margin-bottom:18px">
                <h2 style="margin-top:0">"TRaSH changed things you edited"</h2>
                <p class="small muted">"Marquee left your versions as they are. Pick for each one."</p>
                {list.into_iter().map(|c| waiting_row(c, admin, reload)).collect_view()}
            </div>
        }
        .into_any()
    }
}

/// One waiting change with its two choices.
fn waiting_row(c: Value, admin: bool, reload: Callback<()>) -> impl IntoView {
    let id = c["id"].as_i64().unwrap_or(0);
    let name = s(&c, "name");
    let decide = move |take: bool| {
        let name = name.clone();
        move |_| {
            let name = name.clone();
            leptos::task::spawn_local(async move {
                if take {
                    let q = kit_rules::dialogs::Confirm::new(format!("Use TRaSH's version of {name}?"))
                        .text("Your own changes to it are replaced.")
                        .ok("Use TRaSH's version")
                        .danger(true);
                    if !kit_leptos::modal::confirm(q).await {
                        return;
                    }
                }
                let path = format!("/trash/changes/{id}/{}", if take { "take" } else { "keep" });
                match api::post_empty(&path).await {
                    Ok(_) => toast(if take { "TRaSH's version is in use" } else { "Your version stays" }, "ok"),
                    Err(e) => toast(&e.message, "err"),
                }
                reload.run(());
            });
        }
    };
    let lines: Vec<String> = c["lines"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect();
    view! {
        <div class="row wrap between" style="gap:10px;padding:10px 0;border-top:1px solid var(--border)">
            <div class="grow" style="min-width:0">
                <div style="font-weight:650">{s(&c, "name")}</div>
                <div class="small faint">{item_words(&s(&c, "item"), &s(&c, "kind"))}</div>
                <div class="small">{lines.into_iter().map(|l| view! { <div>{l}</div> }).collect_view()}</div>
            </div>
            {admin.then(|| view! {
                <div class="row" style="gap:8px">
                    <button class="btn ghost" on:click=decide(false)>"Keep mine"</button>
                    <button class="btn" on:click=decide(true)>"Use TRaSH's"</button>
                </div>
            })}
        </div>
    }
}

/// The TRaSH profile the wizard's pick gives, from the status's `options`.
fn picked<'a>(options: &'a Value, kind: &str, source: &str, resolution: &str) -> Option<&'a Value> {
    let res = if resolution == "both" { "1080p" } else { resolution };
    options[kind].as_array()?.iter().find(|o| o["source"] == source && o["resolution"] == res).map(|o| &o["profile"])
}

/// The short setup wizard: media server, movies, TV, anime.
#[component]
fn Wizard(status: Value, reload: Callback<()>) -> impl IntoView {
    let c = status["choices"].clone();
    let options = status["options"].clone();
    let server = RwSignal::new(s(&c, "server"));
    let movie_res = RwSignal::new(s(&c["movie"], "resolution"));
    let movie_src = RwSignal::new(s(&c["movie"], "source"));
    let tv_res = RwSignal::new(s(&c["tv"], "resolution"));
    let tv_src = RwSignal::new(s(&c["tv"], "source"));
    let anime = RwSignal::new(c["anime"] != false);
    let busy = RwSignal::new(false);
    let set_up = status["setUp"] == true;
    let save = move |_| {
        let body = json!({
            "server": server.get(),
            "movie": {"resolution": movie_res.get(), "source": movie_src.get()},
            "tv": {"resolution": tv_res.get(), "source": tv_src.get()},
            "anime": anime.get(),
        });
        busy.set(true);
        leptos::task::spawn_local(async move {
            match api::put("/trash/choices", body).await {
                Ok(v) if v["waiting"].as_i64().unwrap_or(0) > 0 => {
                    toast(&format!("Saved. {} things you edited wait for you above.", v["waiting"]), "ok")
                }
                Ok(_) => toast("Saved. New movies and series get these profiles.", "ok"),
                Err(e) => toast(&e.message, "err"),
            }
            busy.set(false);
            reload.run(());
        });
    };
    let about = move |kind: &'static str| {
        let options = options.clone();
        move || {
            let (res, src) = if kind == "movie" { (movie_res.get(), movie_src.get()) } else { (tv_res.get(), tv_src.get()) };
            let name = picked(&options, kind, &src, &res).map(|p| s(p, "name")).unwrap_or_default();
            let extra = match res.as_str() {
                "both" => picked(&options, kind, &src, "2160p").map(|p| format!(" and {}", s(p, "name"))).unwrap_or_default(),
                _ => String::new(),
            };
            format!("TRaSH profile: {name}{extra}")
        }
    };
    let defaults = status["defaults"].clone();
    let current = move |kind: &str| defaults[kind]["name"].as_str().map(|n| format!("New items get: {n}")).unwrap_or_default();
    let select = |label: &'static str, id: &'static str, sig: RwSignal<String>, opts: Vec<(&'static str, &'static str)>| {
        view! {
            <label class="field">
                <span>{label}</span>
                <select class="input" id=id prop:value=move || sig.get() on:change=move |e| sig.set(event_target_value(&e))>
                    {opts.into_iter().map(|(v, l)| view! { <option value=v selected=move || sig.get() == v>{l}</option> }).collect_view()}
                </select>
            </label>
        }
    };
    let resolutions = || vec![("1080p", "1080p (HD)"), ("2160p", "2160p (4K)"), ("both", "Both, 1080p first")];
    view! {
        <div class="card" id="trash-wizard" style="margin-bottom:18px">
            <h2 style="margin-top:0">{if set_up { "Your choices" } else { "Set up" }}</h2>
            <p class="small muted">"Pick what you want; Marquee adds TRaSH's matching profiles and makes them the default for new items. Profiles you have stay."</p>
            {select("Media server (for file and folder names)", "trash-server", server, vec![("jellyfin", "Jellyfin"), ("plex", "Plex"), ("emby", "Emby"), ("none", "None of these")])}
            <h3>"Movies"</h3>
            <div class="row wrap" style="gap:12px">
                {select("Resolution", "trash-movie-res", movie_res, resolutions())}
                {select("Source", "trash-movie-src", movie_src, vec![("web", "Streaming (WEB)"), ("bluray", "Blu-ray and WEB"), ("remux", "Remux and WEB (biggest files)")])}
            </div>
            <div class="small faint">{about("movie")}</div>
            <div class="small faint">{current("movie")}</div>
            <h3>"TV"</h3>
            <div class="row wrap" style="gap:12px">
                {select("Resolution", "trash-tv-res", tv_res, resolutions())}
                {select("Source", "trash-tv-src", tv_src, vec![("web", "Streaming (WEB)"), ("remux", "Remux and WEB (biggest files)")])}
            </div>
            <div class="small faint">{about("tv")}</div>
            <div class="small faint">{current("tv")}</div>
            <h3>"Anime"</h3>
            <label class="switch">
                <input type="checkbox" id="trash-anime" prop:checked=move || anime.get() on:change=move |e| anime.set(event_target_checked(&e))/>
                <i></i>
                <span>"Use TRaSH's anime profile (Blu-ray and WEB tiers, dual audio, dubs only)"</span>
            </label>
            <div class="row" style="margin-top:14px">
                <button class="btn primary" id="trash-save" on:click=save disabled=move || busy.get()>{if set_up { "Save" } else { "Set up" }}</button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_read_as_words() {
        assert_eq!(status_words("waiting").0, "Waiting for you");
        assert_eq!(item_words("profile", "anime"), "Quality profile · anime");
        assert_eq!(item_words("format", "tv"), "Custom format · TV");
        assert_eq!(version_words("d0f1a6cb29bee8b53bd1d7f8c73333649972f21c", "2026-11-03T09:43:03Z"), "d0f1a6c, 2026-11-03");
        assert_eq!(version_words("", ""), "none yet");
    }

    #[test]
    fn the_wizard_shows_the_profile_a_pick_gives() {
        let options = json!({"movie": [{"source": "bluray", "resolution": "1080p", "profile": {"name": "HD Bluray + WEB"}}]});
        assert_eq!(picked(&options, "movie", "bluray", "both").unwrap()["name"], "HD Bluray + WEB");
        assert!(picked(&options, "tv", "web", "1080p").is_none());
    }
}
