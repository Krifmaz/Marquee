//! Requests and discovery (the requests track). docs/TRACKS.md says who owns this file.
//!
//! | Where | What |
//! | --- | --- |
//! | `#/discover` | trending movies and TV, or what a search finds on TMDB; Request (pick seasons for TV, and the language for anime) |
//! | `#/requests` | the requests (everybody's for admins, your own otherwise) with where each stands; Approve, Decline, Retry and Remove |
//!
//! Both pages use the Seerr API at `/api/v1` (the same one LunaSea and nzb360 use). The shell asks [`page`] for
//! every route it doesn't know and shows [`nav`]'s links.

use crate::movies::{encode, poster, s};
use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// This track's page for `path` (`#/<path>?<query>`), `None` when the path isn't one of its own.
pub fn page(path: &str, _query: &str, admin: bool) -> Option<AnyView> {
    match path {
        "/discover" => Some(view! { <Discover/> }.into_any()),
        "/requests" => Some(view! { <Requests admin=admin/> }.into_any()),
        _ => None,
    }
}

/// This track's sidebar links: (href, label).
pub fn nav(_admin: bool) -> Vec<(&'static str, &'static str)> {
    vec![("/discover", "Discover"), ("/requests", "Requests")]
}

/// TMDB's poster at a size for the grid, "" for none.
pub fn tmdb_poster(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!("https://image.tmdb.org/t/p/w342{path}")
    }
}

/// A Seerr result (movie or TV) as the poster grid reads it: `title`, `year`, `poster`.
pub fn as_poster(r: &Value) -> Value {
    let title = if r["mediaType"] == "tv" { s(r, "name") } else { s(r, "title") };
    let date = if r["mediaType"] == "tv" { s(r, "firstAirDate") } else { s(r, "releaseDate") };
    let year = date.get(..4).and_then(|y| y.parse::<i64>().ok()).unwrap_or(0);
    json!({ "title": title, "year": year, "poster": tmdb_poster(&s(r, "posterPath")) })
}

/// Where a title stands from its `mediaInfo.status`, in words, and the badge style; ("", "") when nobody asked.
pub fn media_badge(status: Option<i64>) -> (&'static str, &'static str) {
    match status {
        Some(5) => ("Available", "ok"),
        Some(4) => ("Partly there", "ok"),
        Some(3) => ("Approved", "info"),
        Some(2) => ("Requested", "info"),
        _ => ("", ""),
    }
}

/// The badge style for a request's words (`statusWords`).
pub fn request_style(words: &str) -> &'static str {
    match words {
        "Available" => "ok",
        "Downloading" | "Approved" => "info",
        "Waiting for approval" => "warn",
        "Didn't work" => "err",
        _ => "",
    }
}

/// Whether a TV answer is anime: Japanese and animated.
pub fn is_anime(tv: &Value) -> bool {
    tv["originalLanguage"] == "ja" && tv["genres"].as_array().is_some_and(|g| g.iter().any(|x| x["name"] == "Animation"))
}

/// The words for a request's seasons: "Season 1", "Seasons 1, 2", "" for a movie.
pub fn season_words(r: &Value) -> String {
    let list: Vec<String> =
        r["seasons"].as_array().into_iter().flatten().filter_map(|s| s["seasonNumber"].as_i64()).map(|n| n.to_string()).collect();
    match list.len() {
        0 => String::new(),
        1 => format!("Season {}", list[0]),
        _ => format!("Seasons {}", list.join(", ")),
    }
}

/// The words for an anime request's language choice: "Dual audio, en subtitles".
pub fn language_words(audio: &str, subtitles: &str) -> String {
    let a = match audio {
        "dub" => "Dub only",
        "dual" => "Dual audio",
        "sub" => "Sub only",
        _ => return String::new(),
    };
    format!("{a}, {subtitles} subtitles")
}

/// `#/discover`: trending, or what the search finds, with Request.
#[component]
pub fn Discover() -> impl IntoView {
    let (term, set_term) = signal(String::new());
    let (tick, set_tick) = signal(0u32);
    let me = LocalResource::new(|| async { api::get("/v1/auth/me").await.ok() });
    let list = LocalResource::new(move || {
        let t = term.get();
        tick.get();
        async move {
            let path =
                if t.trim().is_empty() { "/v1/discover/trending".to_string() } else { format!("/v1/search?query={}", encode(t.trim())) };
            api::get(&path).await.map_err(|e| e.message)
        }
    });
    let (picking, set_picking) = signal(None::<Value>);
    let input = NodeRef::<leptos::html::Input>::new();
    let search = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        set_term.set(input.get().map(|i| i.value()).unwrap_or_default());
    };
    let can_request = move || me.get().flatten().is_some_and(|m| m["role"] != "viewer");
    let request_movie = move |id: i64, title: String| {
        leptos::task::spawn_local(async move {
            match api::post("/v1/request", json!({ "mediaType": "movie", "mediaId": id })).await {
                Ok(r) => {
                    let words = s(&r, "statusWords");
                    toast(
                        &format!(
                            "You asked for {title}. {}",
                            if words == "Waiting for approval" { "An admin will look at it." } else { "Marquee is looking for it now." }
                        ),
                        "ok",
                    );
                    set_tick.update(|t| *t += 1);
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let grid = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let results: Vec<Value> = v["results"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r["mediaType"] == "movie" || r["mediaType"] == "tv")
                .collect();
            if results.is_empty() {
                return view! { <div class="empty"><p class="muted">"Nothing found. Try other words."</p></div> }.into_any();
            }
            let can = can_request();
            view! {
                <div class="posters" id="discover">
                    {results.into_iter().map(|r| {
                        let status = r["mediaInfo"]["status"].as_i64();
                        let (words, kind) = media_badge(status);
                        let id = r["id"].as_i64().unwrap_or(0);
                        let p = as_poster(&r);
                        let title = s(&p, "title");
                        let open = status.is_none_or(|st| st < 5) && can;
                        let action = match (open, r["mediaType"] == "tv") {
                            (false, _) => None,
                            (true, true) => {
                                let r = r.clone();
                                Some(view! { <button class="btn" on:click=move |_| set_picking.set(Some(r.clone()))>"Request…"</button> }.into_any())
                            }
                            (true, false) if status.is_none() => Some(view! {
                                <button class="btn primary" on:click=move |_| request_movie(id, title.clone())>"Request"</button>
                            }.into_any()),
                            _ => None,
                        };
                        poster(&p, (words, kind), action)
                    }).collect_view()}
                </div>
            }
            .into_any()
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">{move || if term.get().trim().is_empty() { "Trending".to_string() } else { format!("Found for “{}”", term.get().trim()) }}</h1>
            <form class="row" style="gap:8px" on:submit=search>
                <input class="input" type="search" placeholder="Find a movie or show" aria-label="Find a movie or show" node_ref=input/>
                <button class="btn" type="submit">"Search"</button>
            </form>
        </div>
        {move || picking.get().map(|r| view! { <SeasonPicker result=r on_done=move |asked| { set_picking.set(None); if asked { set_tick.update(|t| *t += 1); } }/> })}
        {grid}
    }
}

/// The card that asks which seasons of a show (and for anime, which language) to request.
#[component]
fn SeasonPicker(result: Value, on_done: impl Fn(bool) + Copy + Send + Sync + 'static) -> impl IntoView {
    let id = result["id"].as_i64().unwrap_or(0);
    let title = s(&as_poster(&result), "title");
    let details = LocalResource::new(move || async move {
        let tv = api::get(&format!("/v1/tv/{id}")).await.map_err(|e| e.message)?;
        let me = api::get("/v1/auth/me").await.unwrap_or(Value::Null);
        Ok::<_, String>((tv, me))
    });
    let (chosen, set_chosen) = signal(Vec::<i64>::new());
    let (audio, set_audio) = signal(String::new());
    let subtitles = NodeRef::<leptos::html::Input>::new();
    let (busy, set_busy) = signal(false);
    let send = move |anime: bool, title: String| {
        let seasons = chosen.get();
        if seasons.is_empty() {
            toast("Pick at least one season", "warn");
            return;
        }
        let mut body = json!({ "mediaType": "tv", "mediaId": id, "seasons": seasons });
        if anime {
            body["animeAudio"] = json!(audio.get());
            body["animeSubtitles"] = json!(subtitles.get().map(|i| i.value()).unwrap_or_else(|| "en".into()));
        }
        set_busy.set(true);
        leptos::task::spawn_local(async move {
            match api::post("/v1/request", body).await {
                Ok(_) => {
                    toast(&format!("You asked for {title}"), "ok");
                    on_done(true);
                }
                Err(e) => toast(&e.message, "err"),
            }
            set_busy.set(false);
        });
    };
    let heading = format!("Which seasons of {title}?");
    let body = move || match details.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok((tv, me))) => {
            let have: Vec<(i64, i64)> = tv["mediaInfo"]["seasons"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| Some((x["seasonNumber"].as_i64()?, x["status"].as_i64()?)))
                .collect();
            let seasons: Vec<Value> = tv["seasons"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|x| x["seasonNumber"].as_i64().unwrap_or(0) > 0)
                .collect();
            let anime = is_anime(&tv);
            if anime && audio.get_untracked().is_empty() {
                set_audio.set(s(&me, "animeAudio").chars().next().map(|_| s(&me, "animeAudio")).unwrap_or_else(|| "dual".into()));
            }
            let subs_default = Some(s(&me, "animeSubtitles")).filter(|x| !x.is_empty()).unwrap_or_else(|| "en".into());
            let t = title.clone();
            view! {
                <div class="stack">
                    {seasons.into_iter().map(|x| {
                        let n = x["seasonNumber"].as_i64().unwrap_or(0);
                        let episodes = x["episodeCount"].as_i64().unwrap_or(0);
                        let (words, _) = media_badge(have.iter().find(|(s, _)| *s == n).map(|(_, st)| *st));
                        let taken = !words.is_empty();
                        let label = format!("Season {n} · {episodes} episodes{}", if taken { format!(" · {words}") } else { String::new() });
                        view! {
                            <label class="switch">
                                <input type="checkbox" disabled=taken
                                    on:change=move |e| {
                                        let on = event_target_checked(&e);
                                        set_chosen.update(|c| { c.retain(|x| *x != n); if on { c.push(n); c.sort_unstable(); } });
                                    }/>
                                <i></i>
                                <span>{label}</span>
                            </label>
                        }
                    }).collect_view()}
                    {anime.then(|| view! {
                        <div class="field">
                            <span>"Audio"</span>
                            <div class="seg" role="group" aria-label="Audio">
                                {[("dub", "Dub only"), ("dual", "Dual audio"), ("sub", "Sub only")].into_iter().map(|(k, label)| view! {
                                    <button type="button" class:on=move || audio.get() == k on:click=move |_| set_audio.set(k.to_string())>{label}</button>
                                }).collect_view()}
                            </div>
                        </div>
                        <label class="field">
                            <span>"Subtitle language"</span>
                            <input class="input" maxlength="2" value=subs_default node_ref=subtitles/>
                            <small>"A two-letter code, like en."</small>
                        </label>
                    })}
                    <div class="row" style="gap:8px">
                        <button class="btn primary" disabled=move || busy.get() on:click=move |_| send(anime, t.clone())>"Request"</button>
                        <button class="btn ghost" on:click=move |_| on_done(false)>"Cancel"</button>
                    </div>
                </div>
            }
            .into_any()
        }
    };
    view! {
        <div class="card" style="margin-bottom:18px" id="season-picker">
            <h3 style="margin-top:0">{heading}</h3>
            {body}
        </div>
    }
}

/// `#/requests`: the requests with where each stands, and what an admin (or the asker) can do with them.
#[component]
pub fn Requests(admin: bool) -> impl IntoView {
    let (filter, set_filter) = signal("all".to_string());
    let (tick, set_tick) = signal(0u32);
    let list = LocalResource::new(move || {
        let f = filter.get();
        tick.get();
        async move { api::get(&format!("/v1/request?take=200&filter={f}")).await.map_err(|e| e.message) }
    });
    let act = move |path: String, done: &'static str| {
        leptos::task::spawn_local(async move {
            match api::post_empty(&path).await {
                Ok(r) if r["status"] == 4 => toast(&format!("It didn't work: {}", s(&r, "message")), "err"),
                Ok(_) => {
                    toast(done, "ok");
                    set_tick.update(|t| *t += 1);
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let remove = move |id: i64, title: String| {
        leptos::task::spawn_local(async move {
            let q = kit_rules::dialogs::Confirm::new(format!("Remove the request for {title}?"))
                .text("The request is forgotten. Anything already downloaded stays in the library.")
                .ok("Remove")
                .danger(true);
            if !kit_leptos::modal::confirm(q).await {
                return;
            }
            match api::del(&format!("/v1/request/{id}")).await {
                Ok(_) => {
                    toast("The request was removed", "ok");
                    set_tick.update(|t| *t += 1);
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let filters = [
        ("all", "All"),
        ("pending", "Waiting"),
        ("processing", "Approved"),
        ("available", "Available"),
        ("declined", "Declined"),
        ("failed", "Didn't work"),
    ];
    let grid = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v["results"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return view! { <div class="empty"><p class="muted">"No requests here. Find something to ask for under Discover."</p></div> }.into_any();
            }
            view! {
                <div class="posters" id="requests">
                    {items.into_iter().map(|r| {
                        let id = r["id"].as_i64().unwrap_or(0);
                        let title = s(&r, "title");
                        let words = s(&r, "statusWords");
                        let p = json!({ "title": title, "year": r["year"], "poster": s(&r, "posterUrl") });
                        let who = s(&r["requestedBy"], "displayName");
                        let what = season_words(&r);
                        let lang = language_words(&s(&r, "animeAudio"), &s(&r, "animeSubtitles"));
                        let why = s(&r, "message");
                        let pending = r["status"] == 1;
                        let failed = r["status"] == 4;
                        let can_remove = r["canRemove"] == true;
                        let t = title.clone();
                        let action = view! {
                            <div class="small faint">{[who, what, lang].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")}</div>
                            {(!why.is_empty() && failed).then(|| view! { <div class="small" title=why.clone()>{why.clone()}</div> })}
                            <div class="row wrap" style="gap:6px">
                                {(admin && pending).then(|| view! {
                                    <button class="btn primary" on:click=move |_| act(format!("/v1/request/{id}/approve"), "Approved. Marquee is looking for it now.")>"Approve"</button>
                                    <button class="btn" on:click=move |_| act(format!("/v1/request/{id}/decline"), "Declined")>"Decline"</button>
                                })}
                                {(admin && failed).then(|| view! {
                                    <button class="btn" on:click=move |_| act(format!("/v1/request/{id}/retry"), "Approved. Marquee is looking for it now.")>"Retry"</button>
                                })}
                                {can_remove.then(|| view! { <button class="btn ghost" on:click=move |_| remove(id, t.clone())>"Remove"</button> })}
                            </div>
                        }
                        .into_any();
                        poster(&p, (words.clone(), request_style(&words)), Some(action))
                    }).collect_view()}
                </div>
            }
            .into_any()
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Requests"</h1>
            <div class="seg" role="group" aria-label="Show">
                {filters.into_iter().map(|(k, label)| view! {
                    <button type="button" class:on=move || filter.get() == k on:click=move |_| set_filter.set(k.to_string())>{label}</button>
                }).collect_view()}
            </div>
        </div>
        {grid}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_become_posters_with_where_they_stand() {
        let p = as_poster(&json!({ "mediaType": "tv", "name": "Severance", "firstAirDate": "2022-02-17", "posterPath": "/x.jpg" }));
        assert_eq!(p, json!({ "title": "Severance", "year": 2022, "poster": "https://image.tmdb.org/t/p/w342/x.jpg" }));
        assert_eq!(as_poster(&json!({ "mediaType": "movie", "title": "Dune" }))["year"], 0);
        assert_eq!(media_badge(Some(5)), ("Available", "ok"));
        assert_eq!(media_badge(None), ("", ""));
        assert_eq!(request_style("Waiting for approval"), "warn");
    }

    #[test]
    fn seasons_and_languages_read_as_words() {
        assert_eq!(season_words(&json!({ "seasons": [{ "seasonNumber": 1 }] })), "Season 1");
        assert_eq!(season_words(&json!({ "seasons": [{ "seasonNumber": 1 }, { "seasonNumber": 2 }] })), "Seasons 1, 2");
        assert_eq!(season_words(&json!({ "seasons": [] })), "");
        assert_eq!(language_words("dual", "en"), "Dual audio, en subtitles");
        assert_eq!(language_words("", "en"), "");
        assert!(is_anime(&json!({ "originalLanguage": "ja", "genres": [{ "name": "Animation" }] })));
        assert_eq!(nav(false).len(), 2);
    }
}
