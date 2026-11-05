//! `#/anime`: the Anime tab, its own section next to Movies and TV. Four views, picked by `?v=`:
//!
//! | View | What |
//! | --- | --- |
//! | `library` (default) | every season of every anime series as its own poster, and anime films, with AniList's posters, airing status and the next episode's countdown |
//! | `discover` | AniList's trending, this season, upcoming and top rated, with genre and studio filters, and Request |
//! | `calendar` | anime episodes and films coming out in the next two weeks |
//! | `activity` | the downloads of anime only |
//!
//! Requests go through the Seerr API (`/api/v1/request`) like the Discover page, and each one asks Dub only / Dual
//! audio / Sub only and the subtitle language, with the account's default picked first.

use crate::movies::{encode, poster, s};
use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// The views of the tab: (`?v=` word, name).
pub const VIEWS: [(&str, &str); 4] = [("library", "Library"), ("discover", "Discover"), ("calendar", "Calendar"), ("activity", "Activity")];

/// Discover's lists: (`list=` word, name).
pub const LISTS: [(&str, &str); 4] = [("trending", "Trending"), ("season", "This season"), ("upcoming", "Upcoming"), ("top", "Top rated")];

/// The subtitle languages a request can pick: (ISO 639-1, name).
pub const SUBTITLES: [(&str, &str); 16] = [
    ("en", "English"),
    ("es", "Spanish"),
    ("pt", "Portuguese"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("ru", "Russian"),
    ("ar", "Arabic"),
    ("pl", "Polish"),
    ("nl", "Dutch"),
    ("sv", "Swedish"),
    ("tr", "Turkish"),
    ("id", "Indonesian"),
    ("zh", "Chinese"),
    ("ko", "Korean"),
    ("ja", "Japanese"),
];

/// The audio choices of an anime request: (word, name).
pub const AUDIO: [(&str, &str); 3] = [("dub", "Dub only"), ("dual", "Dual audio"), ("sub", "Sub only")];

/// The time until an episode airs, in words: "in 2 days 4 hours", "in 3 hours 5 minutes", "in 12 minutes", "now".
pub fn countdown(seconds: i64) -> String {
    if seconds <= 60 {
        return "now".into();
    }
    let (d, h, m) = (seconds / 86_400, seconds % 86_400 / 3600, seconds % 3600 / 60);
    let unit = |n: i64, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    if d > 0 {
        if h > 0 {
            format!("in {} {}", unit(d, "day"), unit(h, "hour"))
        } else {
            format!("in {}", unit(d, "day"))
        }
    } else if h > 0 {
        format!("in {} {}", unit(h, "hour"), unit(m, "minute"))
    } else {
        format!("in {}", unit(m, "minute"))
    }
}

/// A library entry's badge: what is still missing, else its airing status.
pub fn standing(e: &Value) -> (String, &'static str) {
    let n = |k: &str| e[k].as_i64().unwrap_or(0);
    let status = s(e, "status");
    if !e["monitored"].as_bool().unwrap_or(false) {
        return ("Not looking".into(), "");
    }
    if e["kind"] == "movie" {
        return if n("files") > 0 { ("Downloaded".into(), "ok") } else { ("Missing".into(), "warn") };
    }
    match status.as_str() {
        "Airing" => ("Airing".into(), "info"),
        "Not yet aired" => ("Not yet aired".into(), "info"),
        _ if n("files") >= n("episodes") && n("episodes") > 0 => ("Complete".into(), "ok"),
        _ => (status, ""),
    }
}

/// The line under a library poster: the next episode's countdown, else how many episodes are there.
pub fn line(e: &Value, now_s: i64) -> String {
    if let Some(at) = e["next_episode"]["airs"].as_i64() {
        let n = e["next_episode"]["episode"].as_i64().unwrap_or(0);
        return format!("Episode {n} {}", countdown(at - now_s));
    }
    if e["kind"] == "movie" {
        return "Film".into();
    }
    format!("{} of {} episodes", e["files"].as_i64().unwrap_or(0), e["episodes"].as_i64().unwrap_or(0))
}

/// Where a Discover entry stands: in the library, or its request's state; ("", "") when nobody asked.
pub fn discover_badge(e: &Value) -> (&'static str, &'static str) {
    match e["media_status"].as_i64() {
        Some(st) => crate::requests::media_badge(Some(st)),
        None if e["series_id"].is_i64() || e["movie_id"].is_i64() => ("In the library", "ok"),
        None => ("", ""),
    }
}

/// The Seerr request body for an AniList entry with a language choice; `None` when the mapping list doesn't
/// know its TMDB id yet. A series asks for the TheTVDB season the entry is (all seasons when unknown).
pub fn request_body(e: &Value, audio: &str, subtitles: &str) -> Option<Value> {
    let tmdb = e["tmdb_id"].as_i64()?;
    let mut b = json!({ "mediaType": s(e, "media_type"), "mediaId": tmdb, "animeAudio": audio, "animeSubtitles": subtitles });
    if e["media_type"] == "tv" {
        b["tvdbId"] = e["tvdb_id"].clone();
        b["seasons"] = match e["tvdb_season"].as_i64().filter(|n| *n > 0) {
            Some(n) => json!([n]),
            None => json!("all"),
        };
    }
    Some(b)
}

/// The time now in Unix seconds.
fn now_s() -> i64 {
    (kit_leptos::rt::now_ms() / 1000) as i64
}

/// `#/anime`.
#[component]
pub fn Anime(admin: bool, view: String) -> impl IntoView {
    let v = if VIEWS.iter().any(|(k, _)| *k == view) { view } else { "library".into() };
    let body = match v.as_str() {
        "discover" => view! { <Discover/> }.into_any(),
        "calendar" => view! { <Calendar/> }.into_any(),
        "activity" => view! { <Downloads admin=admin/> }.into_any(),
        _ => view! { <Library/> }.into_any(),
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Anime"</h1>
            <div class="tabs" role="tablist" id="anime-views">
                {VIEWS.iter().map(|(k, label)| {
                    let on = *k == v;
                    view! { <button role="tab" class:active=on aria-selected=on.to_string()
                        on:click=move |_| crate::navigate(&format!("/anime?v={k}"))>{*label}</button> }
                }).collect_view()}
            </div>
        </div>
        {body}
    }
}

/// The anime library.
#[component]
fn Library() -> impl IntoView {
    let list = LocalResource::new(|| async { api::get("/anime/library").await.map_err(|e| e.message) });
    // the countdowns move on every minute
    let (now, set_now) = signal(now_s());
    kit_leptos::rt::every(60_000, move || {
        let _ = set_now.try_set(now_s());
    });
    move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return view! { <div class="empty"><p class="muted">"No anime yet. Open Discover to find some."</p></div> }.into_any();
            }
            view! {
                <div class="posters" id="anime-library">
                    {items.into_iter().map(|e| {
                        let (words, kind) = standing(&e);
                        let e2 = e.clone();
                        let under = view! { <div class="small muted">{move || line(&e2, now.get())}</div> }.into_any();
                        poster(&e, (words, kind), Some(under))
                    }).collect_view()}
                </div>
            }
            .into_any()
        }
    }
}

/// AniList's lists with filters and Request.
#[component]
fn Discover() -> impl IntoView {
    let (list_word, set_list) = signal("trending".to_string());
    let (genre, set_genre) = signal(String::new());
    let (studio, set_studio) = signal(String::new());
    let (page, set_page) = signal(1i64);
    let (tick, set_tick) = signal(0u32);
    let (shown, set_shown) = signal(Vec::<Value>::new());
    let (more, set_more) = signal(false);
    let (error, set_error) = signal(None::<String>);
    let (loading, set_loading) = signal(true);
    let (picking, set_picking) = signal(None::<Value>);
    let me = LocalResource::new(|| async { api::get("/v1/auth/me").await.ok() });
    let genres = LocalResource::new(|| async { api::get("/anime/genres").await.ok() });
    Effect::new(move |_| {
        let (l, g, st, p) = (list_word.get(), genre.get(), studio.get(), page.get());
        tick.get();
        set_loading.set(true);
        leptos::task::spawn_local(async move {
            let path = format!("/anime/discover?list={l}&genre={}&studio={}&page={p}", encode(&g), encode(&st));
            match api::get(&path).await {
                Ok(v) => {
                    let found = v["results"].as_array().cloned().unwrap_or_default();
                    set_shown.update(|s| if p == 1 { *s = found } else { s.extend(found) });
                    set_more.set(v["more"].as_bool().unwrap_or(false));
                    set_error.set(None);
                }
                Err(e) => set_error.set(Some(e.message)),
            }
            set_loading.set(false);
        });
    });
    let restart = move || {
        set_page.set(1);
    };
    let studio_in = NodeRef::<leptos::html::Input>::new();
    let can_request = move || me.get().flatten().is_some_and(|m| m["role"] != "viewer");
    let on_studio = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        set_studio.set(studio_in.get().map(|i| i.value().trim().to_string()).unwrap_or_default());
        restart();
    };
    let grid = move || {
        if let Some(e) = error.get() {
            return view! { <div class="error-box">{e}</div> }.into_any();
        }
        let items = shown.get();
        if items.is_empty() {
            return if loading.get() {
                view! { <div class="empty"><span class="spinner"></span></div> }.into_any()
            } else {
                view! { <div class="empty"><p class="muted">"Nothing here. Try another list or filter."</p></div> }.into_any()
            };
        }
        let can = can_request();
        view! {
            <div class="posters" id="anime-discover">
                {items.into_iter().map(|e| {
                    let (words, kind) = discover_badge(&e);
                    let sub = [s(&e, "season"), s(&e, "status")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                    let open = can && words.is_empty();
                    let ec = e.clone();
                    let action = view! {
                        <div class="small muted">{sub}</div>
                        {open.then(|| view! { <button class="btn" on:click=move |_| set_picking.set(Some(ec.clone()))>"Request…"</button> })}
                    }
                    .into_any();
                    poster(&json!({ "title": s(&e, "title"), "year": e["year"], "poster": s(&e, "poster") }), (words, kind), Some(action))
                }).collect_view()}
            </div>
            {move || more.get().then(|| view! {
                <div class="row mt" style="justify-content:center">
                    <button class="btn" disabled=move || loading.get() on:click=move |_| set_page.update(|p| *p += 1)>"Show more"</button>
                </div>
            })}
        }
        .into_any()
    };
    view! {
        <div class="row wrap" style="gap:10px;margin-bottom:14px">
            <div class="seg" role="group" aria-label="List">
                {LISTS.iter().map(|(k, label)| view! {
                    <button type="button" class:on=move || list_word.get() == *k
                        on:click=move |_| { set_list.set(k.to_string()); restart(); }>{*label}</button>
                }).collect_view()}
            </div>
            <select class="input" style="width:auto" aria-label="Genre"
                on:change=move |e| { set_genre.set(event_target_value(&e)); restart(); }>
                <option value="">"Every genre"</option>
                {move || genres.get().flatten().and_then(|g| g.as_array().cloned()).unwrap_or_default().into_iter().map(|g| {
                    let name = g.as_str().unwrap_or("").to_string();
                    view! { <option value=name.clone()>{name.clone()}</option> }
                }).collect_view()}
            </select>
            <form class="row" style="gap:8px" on:submit=on_studio>
                <input class="input" type="search" placeholder="Studio, like MAPPA" aria-label="Studio" node_ref=studio_in/>
                <button class="btn" type="submit">"Filter"</button>
            </form>
        </div>
        {move || picking.get().map(|e| view! {
            <RequestCard entry=e me=me.get().flatten().unwrap_or(Value::Null)
                on_done=Callback::new(move |asked: bool| {
                    set_picking.set(None);
                    if asked { set_page.set(1); set_tick.update(|t| *t += 1); }
                })/>
        })}
        {grid}
    }
}

/// The request card: what is asked for, the audio and the subtitle language (the account's default first).
#[component]
fn RequestCard(entry: Value, me: Value, on_done: Callback<bool>) -> impl IntoView {
    let default_audio = Some(s(&me, "animeAudio")).filter(|a| AUDIO.iter().any(|(k, _)| k == a)).unwrap_or_else(|| "dual".into());
    let default_subs = Some(s(&me, "animeSubtitles")).filter(|x| !x.is_empty()).unwrap_or_else(|| "en".into());
    let (audio, set_audio) = signal(default_audio);
    let (subs, set_subs) = signal(default_subs.clone());
    let (busy, set_busy) = signal(false);
    let title = s(&entry, "title");
    let what = if entry["media_type"] == "movie" {
        "the film".to_string()
    } else {
        match entry["tvdb_season"].as_i64().filter(|n| *n > 0) {
            Some(n) => format!("season {n}"),
            None => "every season".into(),
        }
    };
    let mapped = entry["tmdb_id"].is_i64();
    let mut langs: Vec<(String, String)> = SUBTITLES.iter().map(|(k, n)| (k.to_string(), n.to_string())).collect();
    if !langs.iter().any(|(k, _)| *k == default_subs) {
        langs.insert(0, (default_subs.clone(), default_subs.to_uppercase()));
    }
    let send = {
        let (entry, title) = (entry.clone(), title.clone());
        move |_| {
            let Some(body) = request_body(&entry, &audio.get_untracked(), &subs.get_untracked()) else { return };
            let title = title.clone();
            set_busy.set(true);
            leptos::task::spawn_local(async move {
                match api::post("/v1/request", body).await {
                    Ok(r) => {
                        let waiting = s(&r, "statusWords") == "Waiting for approval";
                        toast(
                            &format!(
                                "You asked for {title}. {}",
                                if waiting { "An admin will look at it." } else { "Marquee is looking for it now." }
                            ),
                            "ok",
                        );
                        on_done.run(true);
                    }
                    Err(e) => toast(&e.message, "err"),
                }
                set_busy.set(false);
            });
        }
    };
    view! {
        <div class="card" style="margin-bottom:18px" id="anime-request">
            <h3 style="margin-top:0">{format!("Request {title}")}</h3>
            <div class="stack">
                <p class="muted" style="margin:0">{format!("This asks for {what}.")}</p>
                {(!mapped).then(|| view! {
                    <div class="error-box">"Marquee can't match this one to TMDB yet, so it can't be requested. The anime list is updated every day."</div>
                })}
                <div class="field">
                    <span>"Audio"</span>
                    <div class="seg" role="group" aria-label="Audio">
                        {AUDIO.iter().map(|(k, label)| view! {
                            <button type="button" class:on=move || audio.get() == *k on:click=move |_| set_audio.set(k.to_string())>{*label}</button>
                        }).collect_view()}
                    </div>
                </div>
                <label class="field">
                    <span>"Subtitle language"</span>
                    <select class="input" style="width:auto" on:change=move |e| set_subs.set(event_target_value(&e))>
                        {langs.into_iter().map(|(k, n)| {
                            let on = k == default_subs;
                            view! { <option value=k selected=on>{n}</option> }
                        }).collect_view()}
                    </select>
                    <small>"Your default is picked first. Change it for this request only."</small>
                </label>
                <div class="row" style="gap:8px">
                    <button class="btn primary" disabled=move || busy.get() || !mapped on:click=send>"Request"</button>
                    <button class="btn ghost" on:click=move |_| on_done.run(false)>"Cancel"</button>
                </div>
            </div>
        </div>
    }
}

/// The day `days` after today, `YYYY-MM-DD`, from the browser's clock.
fn day_after(days: i64) -> String {
    let d = web_sys::js_sys::Date::new(&wasm_bindgen::JsValue::from_f64((now_s() + days * 86_400) as f64 * 1000.0));
    format!("{:04}-{:02}-{:02}", d.get_utc_full_year(), d.get_utc_month() + 1, d.get_utc_date())
}

/// What a calendar entry is, in words: "Episode 14 · S02E02 · The Title" or "Film · out online".
pub fn calendar_words(e: &Value) -> String {
    if e["kind"] == "movie" {
        let how = match e["release"].as_str() {
            Some("cinemas") => "in cinemas",
            Some("physical") => "out on disc",
            _ => "out online",
        };
        return format!("Film · {how}");
    }
    let (se, ep) = (e["season"].as_i64().unwrap_or(0), e["episode"].as_i64().unwrap_or(0));
    let mut parts = vec![];
    if let Some(a) = e["absolute"].as_i64() {
        parts.push(format!("Episode {a}"));
    }
    parts.push(format!("S{se:02}E{ep:02}"));
    let t = s(e, "title");
    if !t.is_empty() {
        parts.push(t);
    }
    parts.join(" · ")
}

/// Anime coming out in the next two weeks, by day.
#[component]
fn Calendar() -> impl IntoView {
    let list = LocalResource::new(|| async move {
        api::get(&format!("/anime/calendar?start={}&end={}", day_after(0), day_after(14))).await.map_err(|e| e.message)
    });
    move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return view! { <div class="empty"><p class="muted">"No anime comes out in the next two weeks."</p></div> }.into_any();
            }
            let mut days: Vec<(String, Vec<Value>)> = Vec::new();
            for e in items {
                let d = s(&e, "date");
                match days.last_mut() {
                    Some((last, list)) if *last == d => list.push(e),
                    _ => days.push((d, vec![e])),
                }
            }
            view! {
                <div class="stack" id="anime-calendar">
                    {days.into_iter().map(|(d, list)| view! {
                        <div class="card">
                            <h3 style="margin-top:0">{d}</h3>
                            {list.into_iter().map(|e| {
                                let name = if e["kind"] == "movie" { s(&e, "title") } else { s(&e, "series_title") };
                                let (badge, kind) = if e["has_file"].as_bool().unwrap_or(false) { ("Downloaded", "ok") } else { ("", "") };
                                view! {
                                    <div class="list-item">
                                        <div class="grow" style="min-width:0">
                                            <div class="ellipsis" style="font-weight:650">{name}</div>
                                            <div class="small muted ellipsis">{calendar_words(&e)}</div>
                                        </div>
                                        {(!badge.is_empty()).then(|| view! { <span class=format!("badge {kind}")>{badge}</span> })}
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    }).collect_view()}
                </div>
            }
            .into_any()
        }
    }
}

/// The downloads of anime only, like Activity.
#[component]
fn Downloads(admin: bool) -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let list = LocalResource::new(move || {
        tick.get();
        async { api::get("/anime/activity").await.map_err(|e| e.message) }
    });
    let reload = Callback::new(move |_: ()| set_tick.update(|t| *t += 1));
    move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return view! { <div class="empty"><p class="muted">"No anime is downloading. Request some from Discover."</p></div> }
                    .into_any();
            }
            view! {
                <div class="table-wrap"><table class="table" id="anime-downloads">
                    <thead><tr><th>"Release"</th><th>"Quality"</th><th>"Size"</th><th>"Where it stands"</th><th></th></tr></thead>
                    <tbody>{items.into_iter().map(|d| crate::activity::row(d, admin, reload)).collect_view()}</tbody>
                </table></div>
            }
            .into_any()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdowns_read_in_words() {
        assert_eq!(countdown(30), "now");
        assert_eq!(countdown(12 * 60), "in 12 minutes");
        assert_eq!(countdown(3 * 3600 + 5 * 60), "in 3 hours 5 minutes");
        assert_eq!(countdown(86_400 + 3600), "in 1 day 1 hour");
        assert_eq!(countdown(2 * 86_400), "in 2 days");
    }

    #[test]
    fn library_entries_say_where_they_stand() {
        let airing = json!({ "kind": "season", "monitored": true, "status": "Airing", "files": 4, "episodes": 12,
            "next_episode": { "episode": 5, "airs": 1000 + 2 * 86_400 + 4 * 3600 } });
        assert_eq!(standing(&airing), ("Airing".into(), "info"));
        assert_eq!(line(&airing, 1000), "Episode 5 in 2 days 4 hours");
        let done = json!({ "kind": "season", "monitored": true, "status": "Finished", "files": 12, "episodes": 12 });
        assert_eq!((standing(&done).0.as_str(), line(&done, 0).as_str()), ("Complete", "12 of 12 episodes"));
        let film = json!({ "kind": "movie", "monitored": true, "files": 0 });
        assert_eq!(standing(&film).0, "Missing");
        assert_eq!(standing(&json!({ "monitored": false })).0, "Not looking");
    }

    #[test]
    fn requests_ask_for_the_season_and_the_language() {
        let tv = json!({ "media_type": "tv", "tmdb_id": 209867, "tvdb_id": 424536, "tvdb_season": 2 });
        assert_eq!(
            request_body(&tv, "sub", "en").unwrap(),
            json!({ "mediaType": "tv", "mediaId": 209867, "tvdbId": 424536, "seasons": [2], "animeAudio": "sub", "animeSubtitles": "en" })
        );
        let film = json!({ "media_type": "movie", "tmdb_id": 1 });
        assert_eq!(request_body(&film, "dub", "es").unwrap()["seasons"], Value::Null);
        assert!(request_body(&json!({ "media_type": "tv" }), "dual", "en").is_none(), "not mapped yet");
        assert_eq!(discover_badge(&json!({ "series_id": 3 })).0, "In the library");
        assert_eq!(discover_badge(&json!({ "media_status": 2 })).0, "Requested");
    }

    #[test]
    fn calendar_entries_show_absolute_numbers() {
        let e = json!({ "kind": "episode", "season": 2, "episode": 2, "absolute": 30, "title": "A Trip" });
        assert_eq!(calendar_words(&e), "Episode 30 · S02E02 · A Trip");
        assert_eq!(calendar_words(&json!({ "kind": "movie", "release": "digital" })), "Film · out online");
    }
}
