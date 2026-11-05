//! `#/move-in` (admins): moving in from Radarr, Sonarr, jfa-go and Seerr. Each source has its address and key, a
//! Check button (a dry run that changes nothing) and, once checked, Move in. The report lists every title or
//! account with what happens to it, and the notes say what to set up first.

use crate::movies::s;
use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// One source on the page: its route, its name, whether it signs in with a username and password (jfa-go)
/// instead of an API key, and whether it needs path rewrites.
struct Source {
    /// the server route after `/api`
    route: &'static str,
    /// the app's name
    name: &'static str,
    /// an example address
    example: &'static str,
    /// jfa-go signs in with a username and password
    password: bool,
    /// where the key is, in words
    hint: &'static str,
    /// whether the other app sees folders, so the page offers path rewrites (Radarr and Sonarr)
    paths: bool,
}

/// The sources, in the order they are best moved in (accounts before the requests that name them).
const SOURCES: [Source; 4] = [
    Source {
        route: "/migrate/radarr",
        name: "Radarr",
        example: "http://radarr:7878",
        password: false,
        hint: "Radarr → Settings → General → API Key",
        paths: true,
    },
    Source {
        route: "/migrate/sonarr",
        name: "Sonarr",
        example: "http://sonarr:8989",
        password: false,
        hint: "Sonarr → Settings → General → API Key",
        paths: true,
    },
    Source {
        route: "/migrate/jfa-go",
        name: "jfa-go",
        example: "http://jfa-go:8056",
        password: true,
        hint: "jfa-go's admin username and password",
        paths: false,
    },
    Source {
        route: "/migrate/seerr",
        name: "Seerr",
        example: "http://seerr:5055",
        password: false,
        hint: "Seerr → Settings → General → API Key",
        paths: false,
    },
];

/// Reads the path rewrites box: one `from => to` per line.
pub fn path_map(text: &str) -> Value {
    json!(text
        .lines()
        .filter_map(|l| l.split_once("=>"))
        .map(|(a, b)| json!({ "from": a.trim(), "to": b.trim() }))
        .filter(|m| !s(m, "from").is_empty() && !s(m, "to").is_empty())
        .collect::<Vec<_>>())
}

/// The body a source's route takes.
fn body(src: &Source, url: &str, key: &str, user: &str, maps: &str, apply: bool) -> Value {
    if src.password {
        json!({ "url": url.trim(), "username": user.trim(), "password": key, "apply": apply })
    } else {
        let mut b = json!({ "url": url.trim(), "api_key": key.trim(), "apply": apply });
        if src.paths {
            b["path_map"] = path_map(maps);
        }
        b
    }
}

/// A report in words: the counts, the notes and every item.
fn report(v: Value) -> impl IntoView {
    let n = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    let applied = v.get("applied").and_then(Value::as_bool).unwrap_or(false);
    let head = if applied {
        format!("Moved in: {} added, {} already here, {} with problems, {} files kept", n("added"), n("skipped"), n("problems"), n("files"))
    } else {
        format!(
            "Nothing was changed yet. Move in would add {}, leave out {} already here, and {} have problems ({} files kept)",
            n("added"),
            n("skipped"),
            n("problems"),
            n("files")
        )
    };
    let notes: Vec<String> = v
        .get("notes")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let items = v.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    view! {
        <p style="margin:12px 0 8px"><strong>{head}</strong></p>
        {(!notes.is_empty()).then(|| view! { <ul class="small">{notes.into_iter().map(|t| view! { <li>{t}</li> }).collect_view()}</ul> })}
        <div class="table-wrap"><table class="table">
            <thead><tr><th>"Title"</th><th>"What happens"</th></tr></thead>
            <tbody>{items.into_iter().map(|i| {
                let badge = match s(&i, "action").as_str() { "add" => "ok", "skip" => "", _ => "warn" };
                view! { <tr><td>{s(&i, "title")}</td><td><span class=format!("badge {badge}")>{s(&i, "detail")}</span></td></tr> }
            }).collect_view()}</tbody>
        </table></div>
    }
}

/// One source's card: address, key (or username and password), path rewrites, Check and Move in.
#[component]
fn SourceCard(index: usize) -> impl IntoView {
    let src = &SOURCES[index];
    let (url, set_url) = signal(String::new());
    let (key, set_key) = signal(String::new());
    let (user, set_user) = signal(String::new());
    let (maps, set_maps) = signal(String::new());
    let (busy, set_busy) = signal(false);
    let (checked, set_checked) = signal(false);
    let (result, set_result) = signal(None::<Result<Value, String>>);
    let run = move |apply: bool| {
        let src = &SOURCES[index];
        let b = body(src, &url.get_untracked(), &key.get_untracked(), &user.get_untracked(), &maps.get_untracked(), apply);
        set_busy.set(true);
        leptos::task::spawn_local(async move {
            let r = api::post(src.route, b).await.map_err(|e| e.message);
            match &r {
                Ok(_) if apply => toast(&format!("Moved in from {}", src.name), "ok"),
                Ok(_) => set_checked.set(true),
                Err(e) => toast(e, "err"),
            }
            set_result.set(Some(r));
            set_busy.set(false);
        });
    };
    let move_in = move |_| {
        leptos::task::spawn_local(async move {
            let q = format!(
                "Move in from {} now? Marquee adds what the check listed. Nothing in {} is changed or deleted.",
                SOURCES[index].name, SOURCES[index].name
            );
            if kit_leptos::modal::confirm_dialog(&format!("Move in from {}", SOURCES[index].name), &q, "Move in", false).await {
                run(true);
            }
        });
    };
    let edited = move || set_checked.set(false);
    view! {
        <div class="card" style="margin-bottom:16px">
            <h2 style="margin-top:0">{src.name}</h2>
            <label class="field"><span>"Address"</span>
                <input class="input" placeholder=src.example prop:value=url on:input=move |e| { set_url.set(event_target_value(&e)); edited() }/></label>
            {src.password.then(|| view! {
                <label class="field"><span>"Username"</span>
                    <input class="input" autocomplete="off" prop:value=user on:input=move |e| { set_user.set(event_target_value(&e)); edited() }/></label>
            })}
            <label class="field"><span>{if src.password { "Password" } else { "API key" }}</span>
                <input class="input" type="password" autocomplete="off" prop:value=key on:input=move |e| { set_key.set(event_target_value(&e)); edited() }/>
                <small class="faint">{src.hint}</small></label>
            {src.paths.then(|| view! {
                <label class="field"><span>"Path rewrites (optional)"</span>
                    <textarea class="input" rows="2" placeholder="/movies => /data/media/movies" prop:value=maps
                        on:input=move |e| { set_maps.set(event_target_value(&e)); edited() }></textarea>
                    <small class="faint">"One per line, when the other app sees your folders under different paths."</small></label>
            })}
            <div class="row" style="gap:10px">
                <button class="btn" disabled=busy on:click=move |_| run(false)>"Check"</button>
                <button class="btn primary" disabled=move || busy.get() || !checked.get() on:click=move_in>"Move in"</button>
            </div>
            {move || result.get().map(|r| match r {
                Ok(v) => report(v).into_any(),
                Err(e) => view! { <div class="error-box" style="margin-top:12px">{e}</div> }.into_any(),
            })}
        </div>
    }
}

/// `#/move-in`.
#[component]
pub fn MoveIn() -> impl IntoView {
    view! {
        <h1>"Move in"</h1>
        <p class="muted" style="max-width:640px">"Bring your movies, series, accounts and requests over from the apps Marquee replaces. Check first: it changes nothing and lists what will happen. Your files stay where they are."</p>
        {(0..SOURCES.len()).map(|i| view! { <SourceCard index=i/> }).collect_view()}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_rewrites_are_read_one_per_line() {
        assert_eq!(
            path_map("/movies => /data/movies\n\nbad line\n /tv=>/data/tv "),
            json!([{ "from": "/movies", "to": "/data/movies" }, { "from": "/tv", "to": "/data/tv" }])
        );
    }

    #[test]
    fn jfa_go_signs_in_with_a_password() {
        let b = body(&SOURCES[2], " http://jfa:8056 ", "pw", "admin", "", false);
        assert_eq!(b, json!({ "url": "http://jfa:8056", "username": "admin", "password": "pw", "apply": false }));
        let b = body(&SOURCES[0], "http://radarr:7878", " key ", "", "/a => /b", true);
        assert_eq!(b["api_key"], "key");
        assert_eq!(b["path_map"][0]["to"], "/b");
    }

    #[test]
    fn seerr_sends_only_its_address_and_key() {
        let b = body(&SOURCES[3], "http://seerr:5055", "key", "", "/a => /b", false);
        assert_eq!(b, json!({ "url": "http://seerr:5055", "api_key": "key", "apply": false }));
    }

    #[test]
    fn every_source_is_on_the_page() {
        let routes: Vec<&str> = SOURCES.iter().map(|s| s.route).collect();
        assert_eq!(routes, ["/migrate/radarr", "/migrate/sonarr", "/migrate/jfa-go", "/migrate/seerr"]);
    }
}
