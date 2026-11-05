# Goals

The owner's goal for Marquee, saved as given on 2026-11-05.

Goal: build Marquee, one Rust app that replaces Seerr, Sonarr and Radarr (Docker, Encore look), started from the Krifmaz/kit template.

1. One app, one database, one login: requests (Seerr), TV (Sonarr) and movies (Radarr) share users, roles, libraries, root folders, download clients, indexers and notifications. Users and roles: admin, can-request with per-user quotas and auto-approve, read-only.
2. API-compatible so existing tools keep working unchanged: Sonarr v3 API (/api/v3 series, episodes, queue, calendar, wanted), Radarr v3 API (movies, queue, calendar, wanted), Seerr API (requests, media, users, discover). Prowlarr must be able to add Marquee as both Sonarr and Radarr and sync indexers; Homedit widgets, nzb360 and LunaSea must work. Contract tests against recorded real Sonarr/Radarr/Seerr responses.
3. Downloads: qBittorrent (and SABnzbd/NZBGet) clients, Prowlarr/Torznab/Newznab indexers, hardlinks/atomic moves on the same filesystem (TRaSH folder layout /data/torrents, /data/media), import, rename, upgrade, blocklist, failed-download handling.
4. TRaSH Guides by default: on first run, create TRaSH quality profiles, custom formats and scores, quality sizes and naming for TV and movies (1080p and 2160p, WEB/remux choices in a short setup wizard). Sync them from the TRaSH Guides repo on a schedule, show what changed, and never overwrite a user's own edits without asking.
5. Requests: discover/trending/search (TMDB/TVDB), request seasons or movies, approval flow, status from request → downloading → available, notifications (Discord, email, ntfy, web push).
6. Media servers: connect Plex and Jellyfin. Sign in with Plex or Jellyfin accounts; account sync keeps users in Marquee, Plex (friends/shared libraries) and Jellyfin (users/library access) matched: create, disable and library access follow Marquee's roles. Trigger library scans on import; show "available" from either server.
7. Migration: import an existing Sonarr, Radarr and Seerr (via their APIs or backups): series, movies, files, history, profiles, requests and users, with a dry run first.
8. Android app on kit-compose for requests, calendar and queue.

Separate small deliverable in its own repo afterwards: a Kodi add-on (Python) for Jellyfin that beats Jellyfin for Kodi: native Kodi library sync (fast incremental via Jellyfin's API and websocket), direct play/path substitution by default, resume/watched sync both ways, multiple users, and a settings screen with plain words.

Build on Krifmaz/kit (pin the latest kit tag; read its docs/DESIGN.md and CLAUDE.md first). Use the kit for the look (Encore theme), What's new and changelog, self-updating web app, notice banners, smart toasts with Copy, confirm dialogs, bug reports and crash logs, diagnostics, settings from a schema, offline mode and the shared release workflows. Don't rebuild what the kit has; if something is missing or generic, add it to the kit instead of the app.

Work hands-off: make your own calls (log non-obvious ones in docs/DECISIONS.md), one PR per change, fix CI until green, squash-merge yourself, delete the branch. Every merge ships a normal vX.Y.Z release (no pre-releases). Never skip tests or force-push main. Only stop to ask about deleting user data, a needed secret or paid service. One short summary per finished goal.

## Build order

From the owner's message of 2026-11-05, one pull request each:

1. The repository made from the kit template (cargo generate), this file, CI and releases through the kit's shared workflows.
2. Shared users, roles and settings.
3. Movies (Radarr API).
4. TV (Sonarr API).
5. Downloads through Lantern and import with hardlinks.
6. TRaSH profiles and sync.
7. Requests (Seerr API).
8. Plex/Jellyfin connection and account sync.
9. Migration from the owner's existing Sonarr, Radarr and Seerr.
10. Android app.

## Changes since

- 2026-11-05 · **Downloads go through Lantern only.** Marquee has no download-client or indexer settings of its own,
  just a Lantern URL and API key. It searches and grabs through Lantern and imports when Lantern reports a download
  finished. This replaces the download-client and indexer parts of goal 3 (qBittorrent, SABnzbd/NZBGet,
  Prowlarr/Torznab/Newznab are Lantern's job); hardlinks/atomic moves, import, rename, upgrade, blocklist and
  failed-download handling stay in Marquee. It also replaces the earlier "use my current Prowlarr as the indexer
  until Lantern is ready".
- 2026-11-05 · **Fresh Rust, original apps as reference only.** Sonarr, Radarr, Seerr and Prowlarr (source, docs, real
  API responses) are reference only; everything is written fresh in Rust. Data files are reused directly (indexer
  YAML definitions, TRaSH Guides profiles). The repository is GPL-3.0, with the original projects and data sources
  credited in README.md.

**v0.1.0** ships when movies, TV, downloads and the TRaSH defaults work end to end. Until then `docs/RELEASE_HOLD`
keeps the release workflow from publishing; the v0.1.0 pull request deletes it.
