# Tracks

Marquee is built in parallel tracks. Each track has its own thread, its own branch and one PR at a time. No two
tracks edit the same file. The core thread owns the shared files and is the merge gate: it watches CI and
squash-merges track PRs in the order below.

## Who owns what

| Track | Owns (only this track edits these) |
| --- | --- |
| **Core** (users, movies, TV, downloads through Lantern, anime, migration) | every file not listed below, including `server/src/migrate/**` (Radarr, Sonarr and the shared `Report`) and `web/src/move_in.rs`; the Seerr and jfa-go imports live in the requests and media servers tracks under `/api/migrate/seerr` and `/api/migrate/jfa-go`, including the shared ones: `server/src/lib.rs`, `server/Cargo.toml`, `server/Cargo.lock`, `server/settings.toml` (outside the track blocks), `server/src/{arr,arr_common,radarr,sonarr,movies,series,tmdb,library,tags,roles,release,lantern,downloads,import}.rs`, `web/src/main.rs`, `web/src/{movies,tv,activity,pages}.rs`, `web/Cargo.toml`, `desktop/**`, `.github/workflows/{ci,release}.yml`, `Dockerfile`, `docs/GOALS.md`, `docs/LANTERN_HUB.md`, this file. Later: `server/src/anime/**`, `server/src/migrate/**` |
| **TRaSH** (profiles, custom formats, sync) | `server/src/trash/**`, `server/data/trash/**`, `server/tests/trash.rs`, `server/examples/trash_bundle.rs`, `web/src/trash.rs` (and `web/src/trash/**`). Until its PR merges it also owns `server/src/quality.rs` and `server/src/decide.rs`; then they go back to core (anime needs them) |
| **Requests** (Seerr API) | `server/src/requests/**` (`mod.rs`, `seerr.rs`, `discover.rs`, …), `server/tests/seerr.rs`, `server/tests/contracts/seerr/**`, `web/src/requests.rs` (and `web/src/requests/**`) |
| **Media servers** (Plex, Jellyfin, accounts that replace jfa-go) | `server/src/media_servers/**` (`mod.rs`, `jellyfin.rs`, `plex.rs`, `account_sync.rs`, `invites.rs`, `recovery.rs`, `expiry.rs`, `external_login.rs`), `server/src/mail.rs` (every track may call it), `server/tests/media_accounts.rs`, `web/src/media.rs` (and `web/src/media/**`). It also owns `server/src/users.rs` and `server/src/auth.rs` from now on |
| **Android** | `android/**`, `.github/workflows/android.yml`, `server/src/calendar.rs`, `server/tests/calendar.rs` |

## Hooks already in place

The shared files already call each track's module, so a track adds code only in its own files.

- **Server:** `lib.rs` declares `trash`, `requests`, `media_servers`, `mail` and `calendar`. At start-up it calls
  `<track>::migrate(&db)` after the core tables and `<track>::spawn(st)` once. It merges `<track>::router()`
  (`calendar::router()` for Android). Put submodules inside your folder (`mod seerr;` in `requests/mod.rs`).
- **TRaSH hooks:** `trash::custom_formats` answers `customformat` in both APIs. `trash::naming` answers
  `config/naming` when it returns `Some`. `trash::default_profile` comes first in `quality::default_profile`.
- **TMDB:** `tmdb::Tmdb::get(path, query)` is public, so the requests track can make discover and trending calls
  from its own `discover.rs`. Get the client with `movies::tmdb(st)`.
- **Web:** `main.rs` asks `trash::page`, `requests::page` and `media::page` for every route it doesn't know. It
  adds their `nav(admin)` links to the sidebar. When nobody is signed in, it shows `media::public_page(hash)` for
  invite and reset links.
- **Settings:** `server/settings.toml` has one block per track at the end. Add sections, groups and keys only
  inside your block, with your prefix: `trash_`, `request_`/`seerr_`, or
  `jellyfin_`/`plex_`/`smtp_`/`signup_`/`invite_`.
- **Dependencies:** `server/Cargo.toml` has one comment line per track under `[dependencies]`. Add your crates
  right under your own line, and commit the `Cargo.lock` that results.

## The only shared edits

- `CHANGELOG.md` (one line under Unreleased) and `docs/DECISIONS.md` (lines at the end) are append-only for every
  track. If two PRs both append, the one that merges second rebases. That is the only allowed overlap.
- Anything else in a core file goes through core. Message the core thread
  ([Build Marquee from the kit template](#cmsg_01LSdTGGYRCMZeBKR63nVcvN51SFHYejnStRbVA4NsnCW5)) with the exact
  change. Core lands it in a small PR, and you rebase.

## Merge order

1. Core: downloads through Lantern (PR #5, with these hooks).
2. TRaSH. After it merges, core ships **v0.1.0**: it removes `docs/RELEASE_HOLD` and writes
   `docs/releases/0.1.0.md`.
3. Requests, media servers and Android's calendar PR are independent of each other. Each merges once it is green
   and rebased on main.
4. Core: anime, after TRaSH (it needs `quality.rs`, `decide.rs` and TRaSH's anime formats).
5. Android's request and account screens, after requests and media servers.
6. Core: migration from Sonarr, Radarr, Seerr and jfa-go, last (it needs requests, media servers and TRaSH).

Every PR rebases on the newest main before it merges and must be green. Core merges with squash and deletes the
branch. While `docs/RELEASE_HOLD` exists, merges are not released.
