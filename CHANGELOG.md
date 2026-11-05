# Changelog

## Unreleased

### Added
- TV: TMDB search, series with seasons and episodes, a TV page with posters and Add, and the Sonarr v3 API at `/sonarr` (series, episodes, season pass, calendar, wanted, quality profiles, root folders for TV and anime, tags, indexers for Prowlarr, commands), checked against real Sonarr 4.0 answers.
- Movies: TMDB search and details, a Movies page with posters and Add, and the Radarr v3 API at `/radarr` (movies, lookup, calendar, wanted, queue, history, quality profiles, root folders, tags, indexers for Prowlarr, commands), checked against real Radarr 6.4 answers.
- The server makes its API key on the first start (Settings → General).
- Accounts shared by requests, movies and TV: roles (owner, admin, can request, read-only), per-account request limits and auto-approve, Settings → Accounts.
- Root folders for movies, TV and anime, with free space, in Settings → Folders.
- Settings → Connections: the Lantern address and API key, and the TMDB key.
- Marquee made from the Krifmaz kit template (v0.2.1): the old Marqueue code is cleared; the goal is in docs/GOALS.md; releases are held until v0.1.0 (docs/RELEASE_HOLD); licensed GPL-3.0 with the original projects and data sources credited in README.md.
