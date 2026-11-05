//! `#/tv`: every series as a poster with where it stands, a search that finds series on TMDB, and Add (admins).

use crate::movies::{encode, poster, s, search_button};
use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// Where a series stands, in words, and the badge style for it.
pub fn standing(v: &Value) -> (String, &'static str) {
    let n = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    let (aired, files) = (n("aired_episodes"), n("episode_files"));
    if !v.get("monitored").and_then(Value::as_bool).unwrap_or(false) {
        ("Not looking".into(), "")
    } else if aired > files {
        let missing = aired - files;
        (if missing == 1 { "1 episode missing".into() } else { format!("{missing} episodes missing") }, "warn")
    } else if aired > 0 {
        ("Up to date".into(), "ok")
    } else {
        ("Not out yet".into(), "info")
    }
}

/// `#/tv`.
#[component]
pub fn Tv(admin: bool) -> impl IntoView {
    let (tick, set_tick) = signal(0u32);
    let list = LocalResource::new(move || {
        tick.get();
        async { api::get("/series").await.map_err(|e| e.message) }
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
            let r = api::get(&format!("/series/lookup?term={}", encode(&t)))
                .await
                .map(|v| v.as_array().cloned().unwrap_or_default())
                .map_err(|e| e.message);
            set_results.set(Some(r));
        });
    };
    let add = move |tmdb_id: i64, title: String| {
        leptos::task::spawn_local(async move {
            match api::post("/series", json!({ "tmdb_id": tmdb_id })).await {
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
                <div class="card mt" id="series-results">
                    <h3 style="margin-top:0">"Found on TMDB"</h3>
                    <div class="posters">
                        {v.into_iter().map(|m| {
                            let added = m.get("id").and_then(Value::as_i64).is_some();
                            let id = m.get("tmdb_id").and_then(Value::as_i64).unwrap_or(0);
                            let title = s(&m, "title");
                            let action = (!added && admin).then(|| {
                                view! { <button class="btn primary" on:click=move |_| add(id, title.clone())>"Add"</button> }.into_any()
                            });
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
            let shows = v.as_array().cloned().unwrap_or_default();
            if shows.is_empty() {
                view! { <div class="empty"><p class="muted">"No series yet. Search above to add the first one."</p></div> }.into_any()
            } else {
                view! {
                    <div class="posters" id="series">
                        {shows.iter().map(|m| {
                            let id = m.get("id").and_then(Value::as_i64).unwrap_or(0);
                            let (words, kind) = standing(m);
                            let action = (admin && kind == "warn").then(|| search_button(format!("/series/{id}/search"), s(m, "title")));
                            poster(m, (words, kind), action)
                        }).collect_view()}
                    </div>
                }
                .into_any()
            }
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"TV"</h1>
            <form class="row" style="gap:8px" on:submit=search>
                <input class="input" type="search" placeholder="Find a series" aria-label="Find a series" node_ref=term/>
                <button class="btn" type="submit">"Search"</button>
            </form>
        </div>
        {found}
        <div class="mt">{grid}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::standing;
    use serde_json::json;

    #[test]
    fn series_say_where_they_stand() {
        assert_eq!(standing(&json!({ "monitored": false })).0, "Not looking");
        assert_eq!(standing(&json!({ "monitored": true, "aired_episodes": 3, "episode_files": 1 })).0, "2 episodes missing");
        assert_eq!(standing(&json!({ "monitored": true, "aired_episodes": 2, "episode_files": 1 })).0, "1 episode missing");
        assert_eq!(standing(&json!({ "monitored": true, "aired_episodes": 2, "episode_files": 2 })).0, "Up to date");
        assert_eq!(standing(&json!({ "monitored": true, "aired_episodes": 0 })).0, "Not out yet");
    }
}
