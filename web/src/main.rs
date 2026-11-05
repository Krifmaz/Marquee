//! The web app: the kit's shell (sidebar, notice slot, view) drawn with Leptos, and every kit feature in it.
//!
//! | Where | What |
//! | --- | --- |
//! | everywhere | toasts with Copy, the details dialog, the data folder notice with Check again, the offline and connection banners, self-update after a new server build |
//! | after signing in | the What's new popup, once per release |
//! | `#/` | Home: the app's own start page (a bug report button to begin with) |
//! | `#/movies` | every movie as a poster, TMDB search and Add (admins) |
//! | `#/activity` | every download through Lantern, how far along, why one is stuck; Stop (admins) |
//! | `#/move-in` | moving in from Radarr, Sonarr, jfa-go and Seerr: Check (a dry run), then Move in (admins) |
//! | `#/tv` | every series as a poster with its missing episodes, TMDB search and Add (admins) |
//! | `#/anime?v=<view>` | the Anime tab: its library (each season its own entry), AniList's Discover with Request, its calendar and downloads |
//! | `#/bug-reports` | the account's bug reports and the New report sheet |
//! | `#/settings?s=<page>` | the server settings form from the server's schema, Accounts and Folders (admins), Diagnostics and What's new |
//!
//! The app's own pages go next to [`Home`], one component each, with a link in [`Shell`]. On phones the sidebar
//! folds away and [`TabBar`] at the bottom holds Movies, TV, Anime, Activity and More (which opens the sidebar).

mod accounts;
mod activity;
mod anime;
mod media;
mod move_in;
mod movies;
mod pages;
mod requests;
mod trash;
mod tv;

use kit_leptos::api::{self, HttpBackend};
use kit_leptos::{details, dropzone, notice, offline, rt, toasts, update, whats_new};
use leptos::prelude::*;
use serde_json::Value;
use std::rc::Rc;
use wasm_bindgen::JsCast;

/// The package name, `<project>-web`: trunk names the script `<project>-web-<hash>.js`, whose hash the
/// server's `/api/health` names as the web build.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The app's name as people see it, from `<meta name="application-name">` in index.html.
pub fn app_name() -> String {
    rt::qs(r#"meta[name="application-name"]"#).and_then(|m| m.get_attribute("content")).unwrap_or_else(|| "App".into())
}

/// The project name (`NAME` without `-web`): the server's binary, the `X-Requested-With` value, the `x-<project>`
/// mark on the server's answers and the prefix of the app's events (`<project>:server-down`). The name people
/// see ([`app_name`]) may have spaces and capitals, which a header name may not.
pub fn project() -> &'static str {
    NAME.strip_suffix("-web").unwrap_or(NAME)
}

/// This page's web build: the hash in its script's name, `<project>-web-<hash>.js`.
fn this_build() -> Option<String> {
    let head = rt::document()?.head()?.inner_html();
    kit_rules::update::build_in(&head, NAME)
}

/// The connection banner (kit-leptos `offline::watch_connection`, with the project name for the events and
/// the app's name in the words): "Not connected" while offline, "The <app> server isn't answering" while the
/// server is down.
fn watch_connection(app: &str) {
    use kit_rules::offline::mode::{server_down, NOT_CONNECTED};
    let down = server_down(app);
    rt::on_window("offline", |_| offline::conn_banner(true, NOT_CONNECTED));
    rt::on_window("online", |_| offline::conn_banner(false, ""));
    rt::on_window(&format!("{}:server-down", project()), move |_| offline::conn_banner(true, &down));
    rt::on_window(&format!("{}:server-up", project()), |_| offline::conn_banner(false, ""));
}

/// The kit's configuration for this app: its id and name, its script stem and where its What's new page is.
/// The server uses the kit's default routes, so no path is set here.
fn kit_config(app: &str) -> kit_leptos::config::AppConfig {
    let mut cfg = kit_leptos::config::AppConfig::new(kit_rules::app::App::new(project(), app));
    cfg.script = NAME.to_string();
    cfg.routes.changelog = "/settings?s=whats-new".into();
    cfg
}

/// The page the address names: its path (`/settings`) and query (`s=diagnostics`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Route {
    /// the part after `#`, before `?`
    pub path: String,
    /// the part after `?`
    pub query: String,
}

impl Route {
    /// The route of a `location.hash` (`#/settings?s=general`).
    pub fn parse(hash: &str) -> Route {
        let h = hash.trim_start_matches('#');
        let (path, query) = h.split_once('?').unwrap_or((h, ""));
        Route { path: if path.is_empty() { "/".into() } else { path.to_string() }, query: query.to_string() }
    }

    /// The value of `key` in the query, "" when it is not there.
    pub fn param(&self, key: &str) -> String {
        self.query.split('&').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v.to_string()).unwrap_or_default()
    }
}

/// The address's route now.
fn current_route() -> Route {
    Route::parse(&rt::window().and_then(|w| w.location().hash().ok()).unwrap_or_default())
}

/// Opens `path` of the app (`/settings?s=whats-new`).
pub fn navigate(path: &str) {
    if let Some(w) = rt::window() {
        let _ = w.location().set_hash(path);
    }
}

/// Starts the kit's parts that live outside the page, then draws the sign-in page or the shell.
fn main() {
    console_error();
    let app = app_name();
    kit_leptos::config::set(kit_config(&app));
    api::set_backend(HttpBackend::new(project()));
    toasts::mount();
    details::init();
    dropzone::init();
    watch_connection(&app);
    let mut cfg = update::UpdateConfig::new(&app);
    cfg.build = this_build();
    update::start(cfg);
    rt::spawn(async move {
        let me = api::get("/auth/me").await.ok();
        let Some(root) = rt::by_id("app").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) else { return };
        match me {
            Some(me) => {
                let admin = me.get("is_admin").and_then(Value::as_bool).unwrap_or(false);
                leptos::mount::mount_to(root, move || view! { <Shell me=me.clone()/> }).forget();
                // the notice slot is part of the shell: start once it is drawn
                notice::start(pages::data_notice(), admin);
                notice::mount();
                whats_new::start(Rc::new(navigate));
            }
            None => {
                let hash = rt::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
                match media::public_page(&hash) {
                    Some(_) => leptos::mount::mount_to(root, move || media::public_page(&hash)).forget(),
                    None => leptos::mount::mount_to(root, pages::SignIn).forget(),
                }
            }
        }
    });
}

/// Panics go to the browser console with their message.
fn console_error() {
    std::panic::set_hook(Box::new(|p| web_sys::console::error_1(&p.to_string().into())));
}

/// One link of the sidebar.
#[component]
fn NavLink(href: &'static str, label: &'static str, route: ReadSignal<Route>) -> impl IntoView {
    let active = move || route.get().path == href || (href != "/" && route.get().path.starts_with(href));
    view! { <a href=format!("#{href}") data-nav=href class:active=active><span>{label}</span></a> }
}

/// The tabs of the phone's bottom bar: (href, label). More opens the sidebar with everything else.
pub const TABS: [(&str, &str); 4] = [("/movies", "Movies"), ("/tv", "TV"), ("/anime", "Anime"), ("/activity", "Activity")];

/// Whether the page at `path` belongs to the tab `href` (`/anime` for `/anime?v=discover`).
pub fn in_tab(path: &str, href: &str) -> bool {
    path == href || path.starts_with(&format!("{href}/"))
}

/// The bar at the bottom on phones (the sidebar folds away there): the media tabs and More.
#[component]
fn TabBar(route: ReadSignal<Route>, menu: ReadSignal<bool>, set_menu: WriteSignal<bool>) -> impl IntoView {
    view! {
        <nav class="tabbar" aria-label="Sections">
            {TABS.iter().map(|(href, label)| {
                let active = move || !menu.get() && in_tab(&route.get().path, href);
                view! { <a href=format!("#{href}") class:active=active>{*label}</a> }
            }).collect_view()}
            <button type="button" class:active=move || menu.get() aria-expanded=move || menu.get().to_string()
                on:click=move |_| set_menu.update(|m| *m = !*m)>"More"</button>
        </nav>
    }
}

/// The question Sign out asks first (the kit's confirm dialog; the desktop app asks the same).
pub fn sign_out_question() -> kit_rules::dialogs::Confirm {
    kit_rules::dialogs::Confirm::new("Sign out?")
        .text("This browser forgets its sign-in. You'll need your password to sign in again.")
        .ok("Sign out")
        .danger(true)
}

/// The kit's shell: the sidebar with the app's pages, the notice slot and the page.
#[component]
fn Shell(me: Value) -> impl IntoView {
    let (route, set_route) = signal(current_route());
    // the sidebar on phones: open after More, closed again once a page is picked
    let (menu, set_menu) = signal(false);
    rt::on_window("hashchange", move |_| {
        set_route.set(current_route());
        set_menu.set(false);
    });
    let name = app_name();
    let username = me.get("username").and_then(Value::as_str).unwrap_or("").to_string();
    let admin = me.get("is_admin").and_then(Value::as_bool) == Some(true);
    let role = match me.get("role").and_then(Value::as_str) {
        Some("owner") => "Owner",
        Some("admin") => "Admin",
        Some("viewer") => "Read-only",
        _ => "Can request",
    };
    let sign_out = move |_| {
        rt::spawn(async {
            if kit_leptos::modal::confirm(sign_out_question()).await {
                let _ = api::post_empty("/auth/logout").await;
                rt::reload();
            }
        })
    };
    let page = move || {
        let r = route.get();
        match r.path.as_str() {
            "/bug-reports" => view! { <pages::BugReports/> }.into_any(),
            "/movies" => view! { <movies::Movies admin=admin/> }.into_any(),
            "/tv" => view! { <tv::Tv admin=admin/> }.into_any(),
            "/anime" => view! { <anime::Anime admin=admin view=r.param("v")/> }.into_any(),
            "/activity" => view! { <activity::Activity admin=admin/> }.into_any(),
            "/move-in" if admin => view! { <move_in::MoveIn/> }.into_any(),
            "/settings" => view! { <pages::Settings page=r.param("s") admin=admin/> }.into_any(),
            // the tracks' own pages (docs/TRACKS.md), then Home
            p => trash::page(p, &r.query, admin)
                .or_else(|| requests::page(p, &r.query, admin))
                .or_else(|| media::page(p, &r.query, admin))
                .unwrap_or_else(|| view! { <pages::Home/> }.into_any()),
        }
    };
    let track_links = [trash::nav(admin), requests::nav(admin), media::nav(admin)].concat();
    view! {
        <div class="shell">
            <aside class="sidebar" id="sidebar" class:open=move || menu.get()>
                <a class="brand" href="#/">{name}</a>
                <nav class="nav">
                    <NavLink href="/" label="Home" route=route/>
                    <NavLink href="/movies" label="Movies" route=route/>
                    <NavLink href="/tv" label="TV" route=route/>
                    <NavLink href="/anime" label="Anime" route=route/>
                    <NavLink href="/activity" label="Activity" route=route/>
                    {track_links.into_iter().map(|(href, label)| view! { <NavLink href=href label=label route=route/> }).collect_view()}
                </nav>
                <nav class="nav nav-under">
                    {admin.then(|| view! { <NavLink href="/move-in" label="Move in" route=route/> })}
                    <NavLink href="/bug-reports" label="Bug reports" route=route/>
                </nav>
                <div class="sidebar-foot" style="margin-top:0">
                    <div class="grow" style="min-width:0">
                        <div class="ellipsis" style="font-weight:650">{username}</div>
                        <div class="small faint">{role}</div>
                    </div>
                    <a class="btn ghost icon" href="#/settings" title="Settings" aria-label="Settings" inner_html=kit_leptos::icons::icon("settings")></a>
                    <button class="btn ghost" id="logout-btn" title="Sign out" on:click=sign_out>"Sign out"</button>
                </div>
            </aside>
            <main class="main">
                <div id="server-notice"></div>
                <div id="view">{page}</div>
            </main>
            <div class="sidebar-veil" class:open=move || menu.get() on:click=move |_| set_menu.set(false)></div>
            <TabBar route=route menu=menu set_menu=set_menu/>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::Route;

    #[test]
    fn sign_out_asks_first() {
        let q = super::sign_out_question();
        assert_eq!((q.title.as_str(), q.ok.as_str(), q.danger), ("Sign out?", "Sign out", true));
    }

    #[test]
    fn the_phone_tabs_put_anime_next_to_movies_and_tv() {
        let labels: Vec<&str> = super::TABS.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, ["Movies", "TV", "Anime", "Activity"]);
        assert!(super::in_tab("/anime", "/anime"));
        assert!(!super::in_tab("/tv", "/anime"));
        assert!(!super::in_tab("/movies-old", "/movies"));
    }

    #[test]
    fn routes_come_from_the_hash() {
        assert_eq!(Route::parse(""), Route { path: "/".into(), query: String::new() });
        assert_eq!(Route::parse("#/"), Route { path: "/".into(), query: String::new() });
        let r = Route::parse("#/settings?s=diagnostics&x=1");
        assert_eq!(r.path, "/settings");
        assert_eq!(r.param("s"), "diagnostics");
        assert_eq!(r.param("missing"), "");
    }
}
