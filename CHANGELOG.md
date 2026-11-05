# Changelog

## Unreleased

### Added
- Moving in from Radarr and Sonarr (the new Move in page, admins; `POST /api/migrate/radarr` and `/api/migrate/sonarr`): with the app's address and API key add every movie and series with its quality profile (by name), root folder, tags, monitored seasons and episodes, and keep the files where they are. A dry run comes first and says what would happen; path rewrites cover folders mounted in other places.
- Android app: sign in with Jellyfin or Plex, and ask for a reset link, when the server offers them; More shows your account (role, when it ends, your Jellyfin and Plex access); admins can make, share, turn off and delete invite links from the phone.
- The sign-in page shows Sign in with Plex, Sign in with Jellyfin and Forgot your password? when they are set up; Jellyfin and Plex scan their libraries after each import; the Docker image has ffprobe, so anime files are checked there too.
- Android app: a Requests tab. See where each request is (waiting, approved, downloading, available), ask for a movie or chosen seasons from Trending or a search, and as an admin approve, decline or retry; remove a request you may remove.
- Moving in from jfa-go: `POST /api/migrate/jfa-go` brings jfa-go's users (matched by Jellyfin user or name, with mail address, end date and turned-off state) and open invites (uses left, end date, account length, their profile's libraries) over, with a dry run first.
- Moving in from Seerr (Overseerr, Jellyseerr): every request with its status, seasons, 4K flag, dates and who asked, and each person's request limits and auto-approve, matched to Marquee accounts by username, email or Plex/Jellyfin name. A dry run shows what would happen first; titles already downloaded come in as available, and running it again adds nothing twice.
- Anime files are checked when they finish: when the audio or subtitles aren't what the request asked for, Marquee turns the release down for good and searches again, or puts it in the Review queue (Settings → Anime). Keep imports it; Reject turns it down. The language choice and the AniList address are now the same ones the request pages use.
- Plex and Jellyfin, and everything jfa-go did: Settings → Media servers (Jellyfin, Plex, sign-up and mail), Media access (whether each server answers, Scan now, each account's Jellyfin user and Plex share with its libraries and end date, linking existing users), Invites (one-time or multi-use links with an end date, role, servers and libraries; sent by mail), a public sign-up page, password reset by mail, accounts that end on a date with a reminder first, sign-in with Plex or Jellyfin, and "available" from either server. Accounts are made only in Marquee; Jellyfin and Plex follow every change (made, password, turned off, removed).
- Android app (android/, on the kit's kit-compose in the Encore look): sign in with your Marquee account, a week-by-week calendar of movies and episodes, the download queue (admins can stop a download) and the library with search; each list keeps a copy for offline use. `GET /api/calendar` gives movies and episodes in one list. An APK is built for every pull request that changes the app.

### Fixed
- On the very first start, adding a movie or series right away could say "There is no such quality profile": the TRaSH defaults are now set up before Marquee answers anyone.

## 0.1.0

### Changed
- First release, v0.1.0: the release hold is lifted, so every merge to main is a release from now on.

### Added
- TRaSH Guides by default: on the first start Marquee adds TRaSH's quality profiles, custom formats and scores, quality sizes and naming for movies, TV and anime (HD Bluray + WEB, WEB-1080p, [Anime] Remux-1080p, Jellyfin naming) and makes them the default. A short wizard on the new Quality page picks 1080p/2160p and WEB/Blu-ray/Remux and the media server. Every 24 hours it looks for TRaSH updates on GitHub, updates what you haven't changed, lists what changed, and asks before touching anything you edited. Custom format scores now choose between releases, and the Radarr and Sonarr APIs list the custom formats and TRaSH naming.
- Requests: Discover (trending and search from TMDB) and Requests pages; ask for a movie or chosen seasons, admins approve or decline, approved titles go into the library and are searched for, and each request shows Waiting for approval, Approved, Downloading or Available. Per-account limits and auto-approve, anime requests with a language choice (Dub only, Dual audio, Sub only, subtitle language) and AniList's seasons, news to Discord and ntfy, and the Seerr API at `/api/v1` (requests, media, users, discover, search) checked against real Jellyseerr 2.7.3 answers.
- Anime as its own kind: a dub, dual-audio or subbed choice per user, the anime-lists mapping (AniList, AniDB, MAL, TVDB, TMDB ids and season offsets), AniList season lists for request pages, an ffprobe check of a file's real audio and subtitle tracks, and a Review queue for files that don't match.
- docs/TRACKS.md: who owns which files in the parallel tracks, the merge order, and hooks so each track adds pages, routes, tables and settings in its own files.
- Downloads through Lantern: Marquee searches Lantern, picks the best release by quality profile and size (each release turned down says why), grabs it, shows it in Activity and in the Radarr and Sonarr queues, and hardlinks the finished files into the library with Radarr's and Sonarr's names. Search buttons on missing movies and series; Radarr and Sonarr search commands; a search for everything missing every 6 hours. The Lantern hub contract is in docs/LANTERN_HUB.md.
- TV: TMDB search, series with seasons and episodes, a TV page with posters and Add, and the Sonarr v3 API at `/sonarr` (series, episodes, season pass, calendar, wanted, quality profiles, root folders for TV and anime, tags, indexers for Prowlarr, commands), checked against real Sonarr 4.0 answers.
- Movies: TMDB search and details, a Movies page with posters and Add, and the Radarr v3 API at `/radarr` (movies, lookup, calendar, wanted, queue, history, quality profiles, root folders, tags, indexers for Prowlarr, commands), checked against real Radarr 6.4 answers.
- The server makes its API key on the first start (Settings → General).
- Accounts shared by requests, movies and TV: roles (owner, admin, can request, read-only), per-account request limits and auto-approve, Settings → Accounts.
- Root folders for movies, TV and anime, with free space, in Settings → Folders.
- Settings → Connections: the Lantern address and API key, and the TMDB key.
- Marquee made from the Krifmaz kit template (v0.2.1): the old Marqueue code is cleared; the goal is in docs/GOALS.md; releases are held until v0.1.0 (docs/RELEASE_HOLD); licensed GPL-3.0 with the original projects and data sources credited in README.md.
