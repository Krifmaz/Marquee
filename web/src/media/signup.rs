//! The public pages, drawn like the sign-in page (the kit's `auth-card`): sign up with an invite, pick a new
//! password from a reset mail, ask for a reset mail, sign in with Jellyfin; and [`SignInOptions`], the
//! Plex and Jellyfin buttons and the "Forgot your password?" link under the sign-in form.

use super::s;
use crate::app_name;
use kit_leptos::api;
use leptos::prelude::*;
use serde_json::{json, Value};

/// The value of the input `r`, "" when it is not drawn.
fn value(r: NodeRef<leptos::html::Input>) -> String {
    r.get().map(|i| i.value()).unwrap_or_default()
}

/// Opens the app signed in (after sign-up or a Plex or Jellyfin sign-in).
fn enter() {
    crate::navigate("/");
    kit_leptos::rt::reload();
}

/// The card every public page sits in.
#[component]
fn Card(title: String, children: Children) -> impl IntoView {
    view! {
        <div class="auth-wrap">
            <div class="auth-card">
                <div class="brand">{app_name()}</div>
                <p class="muted" style="text-align:center;margin:0">{title}</p>
                {children()}
                <p class="small" style="text-align:center;margin:16px 0 0"><a href="#/" on:click=|_| { crate::navigate("/"); kit_leptos::rt::reload(); }>"Back to sign in"</a></p>
            </div>
        </div>
    }
}

/// `#/invite/<code>`: the invite's sign-up form; signed in at once after it.
#[component]
pub fn SignUp(code: String) -> impl IntoView {
    let c = code.clone();
    let info = LocalResource::new(move || {
        let c = c.clone();
        async move { api::get(&format!("/public/invites/{c}")).await.map_err(|e| e.message) }
    });
    let (error, set_error) = signal(None::<String>);
    let (busy, set_busy) = signal(false);
    let (done, set_done) = signal(None::<Value>);
    let (username, password, again, email, plex_email) = (NodeRef::new(), NodeRef::new(), NodeRef::new(), NodeRef::new(), NodeRef::new());
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if value(password) != value(again) {
            set_error.set(Some("The two passwords aren't the same".into()));
            return;
        }
        let body =
            json!({ "username": value(username), "password": value(password), "email": value(email), "plex_email": value(plex_email) });
        let code = code.clone();
        set_busy.set(true);
        set_error.set(None);
        kit_leptos::rt::spawn(async move {
            match api::post(&format!("/public/invites/{code}"), body).await {
                Ok(v) => set_done.set(Some(v)),
                Err(ex) => {
                    set_busy.set(false);
                    set_error.set(Some(ex.message));
                }
            }
        });
    };
    let body = move || match (info.get(), done.get()) {
        (_, Some(v)) => {
            let errors: Vec<String> =
                v["media_errors"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(str::to_string)).collect();
            let jf = info.get().and_then(|i| i.ok()).map(|i| s(&i, "jellyfin_url")).unwrap_or_default();
            view! {
                <div class="mt">
                    <p>{format!("Welcome, {}! Your account is ready.", s(&v, "username"))}</p>
                    {(!jf.is_empty()).then(|| view! { <p class="small muted">"Jellyfin: sign in at "<a href=jf.clone() target="_blank" rel="noopener">{jf.clone()}</a>" with the same name and password."</p> })}
                    {(!errors.is_empty()).then(|| view! { <div class="error-box">{format!("Some of it didn't work yet; an admin can finish it: {}", errors.join(" "))}</div> })}
                    <button class="btn primary mt" style="width:100%;padding:12px" on:click=|_| enter()>"Open "{app_name()}</button>
                </div>
            }
            .into_any()
        }
        (None, _) => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        (Some(Err(e)), _) => view! { <div class="error-box mt">{e}</div> }.into_any(),
        (Some(Ok(i)), _) => {
            let servers = match (i["jellyfin"].as_bool() == Some(true), i["plex"].as_bool() == Some(true)) {
                (true, true) => "You'll get Jellyfin and Plex too.",
                (true, false) => "You'll get Jellyfin too, with the same name and password.",
                (false, true) => "You'll get Plex too: use the mail address of your Plex account.",
                _ => "",
            };
            let plex = i["plex"].as_bool() == Some(true);
            let ends = match i["account_days"].as_i64().unwrap_or(0) {
                0 => String::new(),
                d => format!(" The account lasts {d} days."),
            };
            view! {
                <p class="small muted" style="text-align:center">{format!("{}{}", servers, ends)}</p>
                <div class="error-box" class:hidden=move || error.get().is_none() style="margin-top:16px">{move || error.get()}</div>
                <form autocomplete="on" on:submit=submit.clone()>
                    <label class="field"><span>"Username"</span><input class="input" name="username" autocomplete="username" required minlength="3" maxlength="32" autofocus node_ref=username/></label>
                    <label class="field"><span>"Password"</span><input class="input" type="password" autocomplete="new-password" required minlength="8" node_ref=password/></label>
                    <label class="field"><span>"Password again"</span><input class="input" type="password" autocomplete="new-password" required minlength="8" node_ref=again/></label>
                    <label class="field"><span>{if plex { "Mail address (your Plex account's)" } else { "Mail address (for a forgotten password)" }}</span>
                        <input class="input" type="email" autocomplete="email" required=plex node_ref=email/></label>
                    {plex.then(|| view! { <label class="field"><span>"Plex mail address, if it's another one"</span><input class="input" type="email" node_ref=plex_email/></label> })}
                    <button class="btn primary" type="submit" style="padding:12px" disabled=busy>"Make my account"</button>
                </form>
            }
            .into_any()
        }
    };
    view! { <Card title="You're invited".into()>{body}</Card> }
}

/// `#/reset/<token>`: pick a new password.
#[component]
pub fn Reset(token: String) -> impl IntoView {
    let (error, set_error) = signal(None::<String>);
    let (done, set_done) = signal(false);
    let (busy, set_busy) = signal(false);
    let (password, again) = (NodeRef::new(), NodeRef::new());
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if value(password) != value(again) {
            set_error.set(Some("The two passwords aren't the same".into()));
            return;
        }
        let body = json!({ "token": token.clone(), "password": value(password) });
        set_busy.set(true);
        kit_leptos::rt::spawn(async move {
            match api::post("/public/password/reset", body).await {
                Ok(_) => set_done.set(true),
                Err(ex) => {
                    set_busy.set(false);
                    set_error.set(Some(ex.message));
                }
            }
        });
    };
    view! {
        <Card title="Pick a new password".into()>
            <Show when=move || !done.get() fallback=|| view! { <p class="mt" style="text-align:center">"Done. Sign in with your new password; Jellyfin has it too."</p> }>
                <div class="error-box" class:hidden=move || error.get().is_none() style="margin-top:16px">{move || error.get()}</div>
                <form on:submit=submit.clone()>
                    <label class="field"><span>"New password"</span><input class="input" type="password" autocomplete="new-password" required minlength="8" autofocus node_ref=password/></label>
                    <label class="field"><span>"New password again"</span><input class="input" type="password" autocomplete="new-password" required minlength="8" node_ref=again/></label>
                    <button class="btn primary" type="submit" style="padding:12px" disabled=busy>"Save the new password"</button>
                </form>
            </Show>
        </Card>
    }
}

/// `#/forgot`: ask for a reset mail.
#[component]
pub fn Forgot() -> impl IntoView {
    let (error, set_error) = signal(None::<String>);
    let (sent, set_sent) = signal(false);
    let login = NodeRef::new();
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let body = json!({ "login": value(login) });
        kit_leptos::rt::spawn(async move {
            match api::post("/public/password/forgot", body).await {
                Ok(_) => set_sent.set(true),
                Err(ex) => set_error.set(Some(ex.message)),
            }
        });
    };
    view! {
        <Card title="Forgot your password?".into()>
            <Show when=move || !sent.get() fallback=|| view! { <p class="mt" style="text-align:center">"If that account has a mail address, a link is on its way. It works for an hour."</p> }>
                <div class="error-box" class:hidden=move || error.get().is_none() style="margin-top:16px">{move || error.get()}</div>
                <form on:submit=submit>
                    <label class="field"><span>"Username or mail address"</span><input class="input" required autofocus node_ref=login/></label>
                    <button class="btn primary" type="submit" style="padding:12px">"Send me a link"</button>
                </form>
            </Show>
        </Card>
    }
}

/// `#/jellyfin`: sign in with a Jellyfin name and password.
#[component]
pub fn JellyfinSignIn() -> impl IntoView {
    let (error, set_error) = signal(None::<String>);
    let (busy, set_busy) = signal(false);
    let (username, password) = (NodeRef::new(), NodeRef::new());
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let body = json!({ "username": value(username), "password": value(password) });
        set_busy.set(true);
        kit_leptos::rt::spawn(async move {
            match api::post("/auth/jellyfin", body).await {
                Ok(_) => enter(),
                Err(ex) => {
                    set_busy.set(false);
                    set_error.set(Some(ex.message));
                }
            }
        });
    };
    view! {
        <Card title="Sign in with Jellyfin".into()>
            <div class="error-box" class:hidden=move || error.get().is_none() style="margin-top:16px">{move || error.get()}</div>
            <form autocomplete="on" on:submit=submit>
                <label class="field"><span>"Jellyfin username"</span><input class="input" autocomplete="username" required autofocus node_ref=username/></label>
                <label class="field"><span>"Jellyfin password"</span><input class="input" type="password" autocomplete="current-password" node_ref=password/></label>
                <button class="btn primary" type="submit" style="padding:12px" disabled=busy>"Sign in"</button>
            </form>
        </Card>
    }
}

/// Signs in with Plex: opens plex.tv in a new window and asks Marquee every two seconds (for three minutes)
/// whether the person finished there.
#[allow(dead_code)] // used by SignInOptions, which core's sign-in page draws
fn plex_sign_in(set_error: WriteSignal<Option<String>>, set_busy: WriteSignal<bool>) {
    // the window opens on the click, so the browser doesn't block it; plex.tv's address follows
    let popup = kit_leptos::rt::window().and_then(|w| w.open_with_url_and_target("about:blank", "_blank").ok().flatten());
    set_busy.set(true);
    kit_leptos::rt::spawn(async move {
        let fail = |m: String| {
            set_busy.set(false);
            set_error.set(Some(m));
        };
        let start = match api::post_empty("/auth/plex/start").await {
            Ok(v) => v,
            Err(e) => {
                if let Some(p) = &popup {
                    let _ = p.close();
                }
                return fail(e.message);
            }
        };
        let url = s(&start, "url");
        match &popup {
            Some(p) => {
                let _ = p.location().set_href(&url);
            }
            None => {
                let _ = kit_leptos::rt::window().map(|w| w.open_with_url_and_target(&url, "_blank"));
            }
        }
        let id = s(&start, "id");
        for _ in 0..90 {
            kit_leptos::rt::sleep(2000).await;
            match api::post("/auth/plex/finish", json!({ "id": id })).await {
                Ok(v) if v.get("pending").and_then(Value::as_bool) == Some(true) => continue,
                Ok(_) => {
                    if let Some(p) = &popup {
                        let _ = p.close();
                    }
                    return enter();
                }
                Err(e) => return fail(e.message),
            }
        }
        fail("Plex didn't finish signing in. Try again.".into());
    });
}

/// Under the sign-in form: "Sign in with Plex", "Sign in with Jellyfin" and "Forgot your password?", each
/// only when this server offers it.
#[allow(dead_code)] // core's sign-in page (pages.rs) draws it
#[component]
pub fn SignInOptions() -> impl IntoView {
    let opts = LocalResource::new(|| async { api::get("/public/sign-in-options").await.ok() });
    let (error, set_error) = signal(None::<String>);
    let (busy, set_busy) = signal(false);
    move || {
        let Some(Some(o)) = opts.get() else { return ().into_any() };
        let on = |k: &str| o.get(k).and_then(Value::as_bool) == Some(true);
        let (plex, jf, reset) = (on("plex"), on("jellyfin"), on("reset"));
        view! {
            <div id="sign-in-options" class="mt" style="display:flex;flex-direction:column;gap:8px">
                <div class="error-box" class:hidden=move || error.get().is_none()>{move || error.get()}</div>
                {plex.then(|| view! { <button class="btn" style="padding:12px" disabled=busy on:click=move |_| plex_sign_in(set_error, set_busy)>"Sign in with Plex"</button> })}
                {jf.then(|| view! { <a class="btn" style="padding:12px;text-align:center" href="#/jellyfin" on:click=|_| kit_leptos::rt::after(0, kit_leptos::rt::reload)>"Sign in with Jellyfin"</a> })}
                {reset.then(|| view! { <p class="small" style="text-align:center;margin:4px 0 0"><a href="#/forgot" on:click=|_| kit_leptos::rt::after(0, kit_leptos::rt::reload)>"Forgot your password?"</a></p> })}
            </div>
        }
        .into_any()
    }
}
