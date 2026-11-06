# Parity with Sonarr, Radarr and Seerr

Marquee ports Sonarr, Radarr and Seerr: the same behaviour, the same API and the same web UI, in one Rust app with
one database and one login. This is the checklist of everything there is to port, module by module, and where
Marquee stands today. The upstream code is pinned in [`UPSTREAM.lock`](../UPSTREAM.lock):

| App | Release | Commit | License |
| --- | --- | --- | --- |
| Sonarr | v4.0.20.3014 | [`cab419ade8`](https://github.com/Sonarr/Sonarr/tree/cab419ade8ac7fcab5bf80394ee492abd35d5f5a) | GPL-3.0 |
| Radarr | v6.4.4.10685 | [`a96bf7c3ec`](https://github.com/Radarr/Radarr/tree/a96bf7c3ecb23cb76993770727285dc17a613fbc) | GPL-3.0 |
| Seerr | v3.5.1 | [`564bfb08b5`](https://github.com/seerr-team/seerr/tree/564bfb08b54caa374bddaf820abfd226e5e69151) | MIT |

**How to port.** Read the upstream module at the pinned commit and carry its behaviour over as it is: the same
rules, defaults, edge cases, names and answers. Differ only where a line in [DECISIONS.md](DECISIONS.md) says so
(for example: downloads go through Lantern; one shared table for root folders, profiles and qualities). Each port
PR ticks its rows here, with the Marquee file it landed in, and adds contract tests against the pinned version.

**Legend.** ✅ ported (behaves like upstream) · 🟨 partly (what is missing is in the note) · ⬜ not started ·
➖ not ported, by decision (the note links the reason).

**Tracks.** *Arr* = Sonarr/Radarr backend port, *Seerr* = Seerr backend port, *UI* = forked web UIs with the Encore
look, *Lead* = this file, `UPSTREAM.lock` and the order of merges. A track sends its ticks to the lead thread when
it doesn't own this file at the time.

## API endpoints

Every path and method in the upstream API specs, checked against Marquee's routes by `scripts/parity.py` (don't tick
these by hand; rerun the script in the PR that adds the route):

| App | Done | Stub | Missing | Table |
| --- | --- | --- | --- | --- |
| Sonarr v3 | 60 | 10 | 164 | [parity/sonarr-api.md](parity/sonarr-api.md) |
| Radarr v3 | 56 | 11 | 170 | [parity/radarr-api.md](parity/radarr-api.md) |
| Seerr v1 | 83 | 0 | 134 | [parity/seerr-api.md](parity/seerr-api.md) |

"Stub" answers a fixed empty value so clients don't fail, but does nothing yet. A PR that adds routes in a new file
adds that file to `ROUTE_FILES` in the script. To rerun:

```sh
# On any machine with git and Python 3.11+ (PyYAML), from the repository root:
scripts/parity-fetch.sh /tmp/upstream && python3 -I scripts/parity.py /tmp/upstream
```

## Sonarr and Radarr backend (`NzbDrone.Core`)

Sonarr and Radarr share most of their core (Radarr began as a Sonarr fork). One row covers both unless it says
otherwise. Track: *Arr*.

| Module | Status | Marquee today | What is left |
| --- | --- | --- | --- |
| Tv (series, seasons, episodes) — Sonarr | 🟨 | `series.rs`, `sonarr.rs` | Episode refresh rules, series types (standard/daily/anime) as Sonarr, `SeriesService` edge cases, season folders |
| Movies (movies, collections, credits, alt titles) — Radarr | 🟨 | `movies.rs`, `radarr.rs` | Collections, credits, alternative titles, movie translations, `MovieService` edge cases |
| MetadataSource (SkyHook / TMDB) | 🟨 | `tmdb.rs` (TMDB for both) | Sonarr reads SkyHook (TVDB); Marquee uses TMDB by [decision](DECISIONS.md). Port search/lookup ranking and refresh rules |
| SeriesStats / MovieStats | 🟨 | computed in `sonarr.rs` / `radarr.rs` | Port the stats queries (episode file counts, size on disk, percent) exactly |
| Parser (release names, languages, scene numbering) | 🟨 | `parser/` (`QualityParser`, `LanguageParser`, both apps' `Parser.cs`, `ReleaseGroupParser` and `SceneChecker` ported with their upstream test cases; `quality_tv.rs`, `quality_movie.rs`, `language*.rs`, `episode.rs`, `movie.rs`, `release_group_movie.rs`, `clr_regex.rs`), `release.rs` | wire the ported parsers into release parsing (`release.rs`) |
| Languages (language list, ISO codes) | ✅ | `parser/` (`Language`, `IsoLanguages`) | — |
| DataAugmentation (scene mappings, XEM, daily series) — Sonarr | ⬜ | — | Scene mapping and XEM numbering, `DailySeries` |
| Qualities (definitions, sizes) | ✅ | `quality.rs` | — |
| Profiles (quality profiles, delay profiles, release profiles) | 🟨 | `quality.rs` | Delay profiles, release profiles (Sonarr), language in profiles |
| CustomFormats | ✅ | `trash/formats.rs` | Custom format CRUD over the API (read-only now) |
| DecisionEngine (specifications) | 🟨 | `decide.rs` | Port every `*Specification.cs` one by one (upgrade allowed, delay, retention, free space, already imported, …) |
| IndexerSearch | 🟨 | `downloads.rs` (search through Lantern) | Season/episode search criteria, interactive search, RSS sync behaviour |
| Indexers (Torznab, Newznab, RSS) | ➖ | stored for Prowlarr, unused | Searching goes through Lantern ([decision](GOALS.md#changes-since)) |
| Download (clients, tracked downloads, failed handling) | 🟨 | `downloads.rs`, `lantern.rs` | Clients are Lantern's job (➖). Port `TrackedDownloadService`, failed-download handling and `CompletedDownloadService` rules |
| Queue | 🟨 | `arr_common.rs` (queue) | Queue item fields, remove/blocklist from queue, grab from pending |
| MediaFiles (import, rename, delete, recycle bin, disk scan) | 🟨 | `import.rs` | Disk scan, manual import, rename preview and execute, `MediaInfo` (ffprobe), file deletion rules, recycle bin cleanup |
| Organizer (naming tokens) | 🟨 | `import.rs` naming | Port `FileNameBuilder` tokens and naming config endpoints in full |
| Extras (subtitles, metadata files, other files) | ⬜ | — | Import and rename extra files, metadata consumers (Kodi, Plex, Roksbox, WDTV) |
| Calendar feed (iCal, `CalendarFeedController`) | ✅ | `calendar_feed.rs` | — |
| History | ⬜ | `/history` is a stub | History table and events (grabbed, imported, failed, deleted, renamed) |
| Blocklisting | 🟨 | releases turned down are remembered | Blocklist table, API and UI as upstream |
| RootFolders | ✅ | `library.rs` | Unmapped folders |
| RemotePathMappings | ⬜ | — | Port (needed when Lantern sees other paths) |
| DiskSpace | ✅ | `arr_common.rs` | — |
| Tags, AutoTagging | 🟨 | `tags.rs` | Auto tagging rules, tag details |
| ImportLists, exclusions | ⬜ | `/importlist` is a stub | Lists (Trakt, Plex watchlist, TMDB, …) and exclusions |
| Notifications (connections) | 🟨 | `requests/notify.rs` (Discord, ntfy for requests) | Upstream's 25+ connections and events (grab, import, upgrade, rename, health) |
| HealthCheck | 🟨 | `checks.rs` (kit diagnostics) | Port the health checks and answer `/health` with them |
| Jobs, Messaging (commands, scheduled tasks) | 🟨 | `/command` (a few commands) | Every command name, task list and intervals, command status |
| Housekeeping | ⬜ | — | Housekeeping tasks |
| Backup | ⬜ | — | Backup and restore (one database for all of Marquee) |
| Configuration (host, media management, UI, indexer config) | 🟨 | `/config/naming`, `/config/ui` | Media management, host, indexer, download client config endpoints |
| Authentication, Security | ✅ | kit sign-in + API key | Forms/basic auth modes answer as Marquee's sign-in |
| Localization | 🟨 | `localization.rs` (English strings) | UI language setting, other languages |
| Update, Lifecycle | ➖ | kit self-update | The kit's release flow replaces upstream's updater |
| Analytics, Instrumentation | ➖ | kit crash logs | No analytics; logs through the kit |
| MediaCover | 🟨 | TMDB image URLs | Local cover cache served at `/MediaCover` |
| CustomFilters | ✅ | `custom_filters.rs` | — |
| ProgressMessaging (SignalR) | 🟨 | `signalr.rs` (MessageHub: negotiate, WebSockets, JSON hub protocol, version and pings) | Push every upstream event (series, episode, queue, command, health) through the hub |

## Seerr backend (`server/`)

Track: *Seerr*.

| Module | Status | Marquee today | What is left |
| --- | --- | --- | --- |
| Requests (`routes/request.ts`, `entity/MediaRequest`) | 🟨 | `requests/seerr.rs`, `requests/mod.rs` (ported from `routes/request.ts`, `MediaRequest.request`, `User.getQuota`) | Override rules, upstream `canRemove` (with `DELETE /media/{id}/file`) |
| Media (`routes/media.ts`, `entity/Media`) | 🟨 | `requests/seerr.rs` (ported from `routes/media.ts`) | Watch data (needs Tautulli; answers "not configured"), `DELETE /media/{id}/file` keeps files until the Arr port follows `deleteFiles`, per-season status body |
| Discover (`routes/discover.ts`) | 🟨 | `requests/discover.rs`, `requests/sliders.rs` (discover rows from `settings/discover`) | Genres, keywords, studios, networks, languages, watchlist |
| Movie, TV, person, collection details | 🟨 | movie and TV done | Person, collection, cast/crew pages, ratings from RT/IMDb as upstream |
| Search (`routes/search.ts`) | 🟨 | `/search` | Keyword and company search |
| Users, permissions, quotas (`routes/user`, `lib/permissions.ts`) | 🟨 | `requests/seerr.rs` (ported from `routes/user/index.ts`), `requests/user_settings.rs` (general, password, permissions and notification settings from `usersettings.ts`); roles map to permission bits ([decision](DECISIONS.md)) | Linked accounts (`usersettings.ts`), sending to each account's own notification settings, watchlist |
| Auth (`routes/auth.ts`: local, Plex, Jellyfin) | 🟨 | Marquee sign-in, Plex/Jellyfin sign-in in `media_servers/` | Answer Seerr's `/auth/*` paths for clients that sign in |
| Settings (`routes/settings/*`) | ⬜ | Marquee's own settings | Seerr's settings endpoints (main, Plex, Jellyfin, Radarr/Sonarr = Marquee itself, notifications, jobs, cache, logs) |
| Notifications (`lib/notifications/agents`) | 🟨 | Discord, ntfy | Email (have `mail.rs`), Gotify, Pushbullet, Pushover, Slack, Telegram, webhook, web push |
| Issues (`routes/issue.ts`, `issueComment.ts`) | 🟨 | `requests/issues.rs` (ported from `routes/issue.ts`, `routes/issueComment.ts`) | Issue notifications; reporting for requesters once per-account permissions are stored ([decision](DECISIONS.md)) |
| Blocklist (`routes/blocklist.ts`, `job/blocklistedTagsProcessor.ts`) | 🟨 | `requests/blocklist.rs` (ported from `routes/blocklist.ts`) | Blocklisted tags (`blocklistedTagsProcessor.ts`) |
| Watchlist sync (`lib/watchlistsync.ts`) | ⬜ | — | Plex watchlist auto-requests |
| Availability sync, scanners (`lib/availabilitySync.ts`, `lib/scanners`) | 🟨 | `media_available` from Plex/Jellyfin | Sonarr/Radarr scanners are Marquee's own library (➖); port availability rules |
| Download tracker (`lib/downloadtracker.ts`) | 🟨 | downloads are Marquee's own | Show progress on requests as upstream |
| Override rules (`routes/overrideRule.ts`) | ⬜ | — | Port |
| Jobs and cache (`job/schedule.ts`, `lib/cache.ts`) | ⬜ | — | Job list, run now, cache stats |
| Image and avatar proxy | ⬜ | — | `/imageproxy`, `/avatarproxy` |
| Service (`routes/service.ts`) | ⬜ | — | Radarr/Sonarr server lists (Marquee answers itself) |
| Status (`/status`, `/status/appdata`) | ✅ | `requests/seerr.rs` | — |
| Overseerr merge, migrations | ➖ | — | Not needed: moving in from Seerr is `/api/migrate/seerr` |

## Web UI

The forked upstream frontends are served by Marquee with the Encore look (track *UI*); the kit's Leptos pages
stay for what upstream doesn't have (setup, TRaSH, accounts and invites, moving in, Anime). A page is ✅ when the
forked page is served, styled and working against Marquee's API.

**Forked apps served and styled (#26); pages turn ✅ as their API lands.** Sonarr's and Radarr's library index and
Seerr's home and discover already load against Marquee.

### Sonarr (served at `/sonarr/`) and Radarr (served at `/radarr/`)

| Page | Sonarr | Radarr | Note |
| --- | --- | --- | --- |
| Library index (posters, overview, table) | ⬜ | ⬜ | Leptos `web/src/tv.rs` / `movies.rs` stand in today |
| Add new / import existing / discover | ⬜ | ⬜ | Radarr adds Discover and Collections |
| Series or movie details, season and episode details | ⬜ | ⬜ | |
| Calendar (and iCal feed) | ⬜ | ⬜ | iCal feed done (`calendar_feed.rs`) |
| Activity: queue, history, blocklist | ⬜ | ⬜ | Leptos `web/src/activity.rs` stands in |
| Wanted: missing, cutoff unmet | ⬜ | ⬜ | |
| Interactive search, interactive import, organize (rename) | ⬜ | ⬜ | |
| Settings: media management, profiles, quality, custom formats | ⬜ | ⬜ | Leptos `web/src/trash.rs` for TRaSH |
| Settings: indexers, download clients | ➖ | ➖ | Lantern only; show a Lantern card instead |
| Settings: import lists, connect, metadata, tags, general, UI | ⬜ | ⬜ | |
| System: status, tasks, backup, updates, events, logs | ⬜ | ⬜ | Updates are the kit's |
| Login, first run | ➖ | ➖ | Marquee's sign-in |

### Seerr (served at `/requests`)

| Page | Status | Note |
| --- | --- | --- |
| Home (discover sliders), discover movies/TV, trending, upcoming | ⬜ | Leptos `web/src/requests.rs` stands in |
| Discover by genre, keyword, language, studio, network | ⬜ | |
| Movie, TV, person, collection; cast, crew, similar, recommendations | ⬜ | |
| Search | ⬜ | |
| Requests list | ⬜ | |
| Issues | ⬜ | |
| Blocklist, watchlist | ⬜ | |
| Users list, user profile, user settings (main, password, permissions, notifications, linked accounts) | ⬜ | Accounts are made in Marquee (`web/src/media.rs`) |
| Settings: main, users, Plex, Jellyfin, services, network, metadata, notifications, jobs, logs, about | ⬜ | Plex/Jellyfin settings are Marquee's |
| Login, setup, reset password | ➖ | Marquee's sign-in and reset links |
