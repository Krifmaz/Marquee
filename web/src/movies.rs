//! `#/movies`: every movie as a poster with where it stands, a search that finds movies on TMDB, and Add (admins).

use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
pub fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Where a movie stands, in words, and the badge style for it.
pub fn standing(m: &Value) -> (&'static str, &'static str) {
    let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
    if flag("has_file") {
        ("Downloaded", "ok")
    } else if !flag("monitored") {
        ("Not looking", "")
    } else if flag("available") {
        ("Missing", "warn")
    } else {
        ("Not out yet", "info")
    }
}

/// One poster: the picture (or the title when there is none), the title and year, and a badge.
pub fn poster(m: &Value, badge: (impl Into<String>, &'static str), action: Option<AnyView>) -> impl IntoView {
    let title = s(m, "title");
    let year = m.get("year").and_then(Value::as_i64).filter(|y| *y > 0).map(|y| y.to_string()).unwrap_or_default();
    let img = s(m, "poster");
    let art = if img.is_empty() {
        view! { <div class="art small">{title.clone()}</div> }.into_any()
    } else {
        view! { <div class="art"><img src=img alt="" loading="lazy" decoding="async"/></div> }.into_any()
    };
    let (words, kind) = (badge.0.into(), badge.1);
    view! {
        <div class="poster">
            {art}
            <div class="name" title=title.clone()>{title.clone()}</div>
            <div class="small faint">{year}</div>
            {(!words.is_empty()).then(|| view! { <span class=format!("badge {kind}")>{words}</span> })}
            {action}
        </div>
    }
}

/// `#/movies`.
#[component]
pub fn Movies(admin: bool) -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let list = LocalResource::new(move || {
        tick.get();
        async { api::get("/movies").await.map_err(|e| e.message) }
    });
    let (results, set_results) = signal(None::<Result<Vec<Value>, String>>);
    let term = NodeRef::<leptos::html::Input>::new();
    let search = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let t = term.get().map(|i| i.value()).unwrap_or_default();
        if t.trim().is_empty() {
            set_results.set(None);
            return;
        }
        set_results.set(Some(Ok(vec![])));
        leptos::task::spawn_local(async move {
            let q = encode(&t);
            let r = api::get(&format!("/movies/lookup?term={q}"))
                .await
                .map(|v| v.as_array().cloned().unwrap_or_default())
                .map_err(|e| e.message);
            set_results.set(Some(r));
        });
    };
    let add = move |tmdb_id: i64, title: String| {
        leptos::task::spawn_local(async move {
            match api::post("/movies", json!({ "tmdb_id": tmdb_id })).await {
                Ok(_) => {
                    toast(&format!("{title} was added"), "ok");
                    set_results.set(None);
                    if let Some(i) = term.get() {
                        i.set_value("");
                    }
                    set_tick.update(|t| *t += 1);
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let found = move || {
        results.get().map(|r| match r {
            Err(e) => view! { <div class="error-box mt">{e}</div> }.into_any(),
            Ok(v) if v.is_empty() => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
            Ok(v) => view! {
                <div class="card mt" id="movie-results">
                    <h3 style="margin-top:0">"Found on TMDB"</h3>
                    <div class="posters">
                        {v.into_iter().map(|m| {
                            let added = m.get("id").and_then(Value::as_i64).is_some();
                            let id = m.get("tmdb_id").and_then(Value::as_i64).unwrap_or(0);
                            let title = s(&m, "title");
                            let action = if added {
                                None
                            } else if admin {
                                Some(view! { <button class="btn primary" on:click=move |_| add(id, title.clone())>"Add"</button> }.into_any())
                            } else {
                                None
                            };
                            poster(&m, if added { ("Added", "ok") } else { ("", "") }, action)
                        }).collect_view()}
                    </div>
                </div>
            }
            .into_any(),
        })
    };
    let grid = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let movies = v.as_array().cloned().unwrap_or_default();
            if movies.is_empty() {
                view! { <div class="empty"><p class="muted">"No movies yet. Search above to add the first one."</p></div> }.into_any()
            } else {
                view! {
                    <div class="posters" id="movies">
                        {movies.iter().map(|m| {
                            let id = m.get("id").and_then(Value::as_i64).unwrap_or(0);
                            let action = (admin && standing(m).0 == "Missing").then(|| search_button(format!("/movies/{id}/search"), s(m, "title")));
                            poster(m, standing(m), action)
                        }).collect_view()}
                    </div>
                }
                .into_any()
            }
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Movies"</h1>
            <form class="row" style="gap:8px" on:submit=search>
                <input class="input" type="search" placeholder="Find a movie" aria-label="Find a movie" node_ref=term/>
                <button class="btn" type="submit">"Search"</button>
            </form>
        </div>
        {found}
        <div class="mt">{grid}</div>
    }
}

/// A Search button: asks the server to search Lantern (`path`) and says what happened.
pub fn search_button(path: String, title: String) -> AnyView {
    let (busy, set_busy) = signal(false);
    let go = move |_| {
        let (path, title) = (path.clone(), title.clone());
        set_busy.set(true);
        leptos::task::spawn_local(async move {
            match api::post_empty(&path).await {
                Ok(v) => match v.get("reason").and_then(Value::as_str) {
                    Some(why) if v["grabbed"].as_array().is_none_or(Vec::is_empty) => toast(why, "warn"),
                    _ => toast(&format!("Found {title}: it's downloading (see Activity)"), "ok"),
                },
                Err(e) => toast(&e.message, "err"),
            }
            set_busy.set(false);
        });
    };
    view! { <button class="btn" disabled=move || busy.get() on:click=go>{move || if busy.get() { "Searching…" } else { "Search" }}</button> }.into_any()
}

/// `t` made safe for a URL's query (letters, digits and `-_.~` stay, the rest is %-encoded).
pub fn encode(t: &str) -> String {
    t.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{encode, standing};
    use serde_json::json;

    #[test]
    fn movies_say_where_they_stand() {
        assert_eq!(standing(&json!({ "has_file": true, "monitored": true })).0, "Downloaded");
        assert_eq!(standing(&json!({ "monitored": false })).0, "Not looking");
        assert_eq!(standing(&json!({ "monitored": true, "available": true })).0, "Missing");
        assert_eq!(standing(&json!({ "monitored": true, "available": false })).0, "Not out yet");
        assert_eq!(encode("Dune: 2021"), "Dune%3A%202021");
    }
}
