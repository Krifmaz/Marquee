# Marquee

One self-hosted app that replaces Seerr, Sonarr and Radarr: requests, TV and movies with one database and one
sign-in, API-compatible with the tools that already talk to them. Downloads go through Lantern. The goal and the
build order are in [docs/GOALS.md](docs/GOALS.md); it is at the start (the kit template), with no release yet.

Made from the [Krifmaz kit](https://github.com/Krifmaz/kit) template (v0.2.1): the kit's look and its
ease-of-use features from the first commit.

| Folder | What it is |
| --- | --- |
| `server/` | Axum server with the kit server crates: health, changelog (What's new), notices, bug reports, diagnostics, settings |
| `web/` | Leptos web app with the kit components and the kit's default theme, served at `/` by the server |
| `desktop/` | Desktop app (Linux, Windows, macOS) on Iced with kit-iced: connect, app-token sign-in, every kit feature, the update notice on the app's own releases |
| `docs/releases/` | one Markdown file per release: the release notes and What's new |

## First steps

1. Make the lock files and commit them (CI and the Docker build use `--locked`):
   `(cd server && cargo generate-lockfile) && (cd web && cargo generate-lockfile)`.
2. Push to GitHub. Every pull request runs the checks; every merge to main is a release.
3. The first start makes the owner account: set `APP_ADMIN_PASSWORD` (and `APP_ADMIN_USER`, default `admin`),
   or read the password it prints (also in `initial_admin_password.txt` in the data folder).

## Run it

```
cd web && trunk build && cd ..
cd server && APP_DATA_DIR=../data APP_STATIC_DIR=../web/dist cargo run
```

Then open <http://localhost:8080>. With Docker:

```
docker build -t marquee .
docker run --read-only -p 8080:8080 -v marquee-data:/data marquee
```

| Variable | Meaning | Default |
| --- | --- | --- |
| `APP_PORT` | the port the server listens on | `8080` |
| `APP_DATA_DIR` | where the database and uploads live (the only folder it writes to) | `/data` |
| `APP_STATIC_DIR` | the built web app | `/app/static` |
| `APP_ADMIN_USER`, `APP_ADMIN_PASSWORD` | the owner made at the first start | `admin`, a printed password |
| `APP_NAME`, `APP_SESSION_DAYS`, `APP_API_KEY` | settings (settings.toml); set here they are locked in Settings | |

## Connecting tools

Marquee speaks the Radarr v3 and Sonarr v3 APIs, so tools made for Radarr and Sonarr work unchanged. Use Marquee's API key (Settings →
General) and these addresses:

| Tool | Address |
| --- | --- |
| Prowlarr (Apps → Radarr), Seerr, Homepage, nzb360, LunaSea | `http://<marquee>:8080/radarr` |
| Prowlarr (Apps → Sonarr), Seerr, Homepage, nzb360, LunaSea | `http://<marquee>:8080/sonarr` |

Downloads go through Lantern. In Settings → Connections, add Lantern's address and API key. Also add this server's
address so Lantern can tell Marquee when a download is done. Mount the downloads folder and the media folders at
the same paths in both containers, on one disk, so finished files are hardlinked and not copied.

Movie and TV information comes from TMDB: add a free API key in Settings → Connections.

## Checks

In `server/`, `web/` and `desktop/`: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
Every merge to main is released as `vX.Y.Z` with the Docker image `ghcr.io/krifmaz/marquee:<version>` and `:latest`.
Desktop installers (Linux tar.gz, Windows zip, macOS dmg) build with Actions → release → "apps". Signing is optional,
from the secrets `WINDOWS_CERT_PFX_BASE64`, `WINDOWS_CERT_PASSWORD`, `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`,
`MACOS_SIGN_IDENTITY`, `APPLE_ID`, `APPLE_TEAM_ID` and `APPLE_APP_PASSWORD`; without them the zip is unsigned and the
dmg is signed ad hoc.

## License and credits

Marquee is licensed under the [GNU General Public License v3.0](LICENSE) (GPL-3.0-only).

Marquee is a Rust port of Sonarr, Radarr and Seerr: it carries their behaviour, API and web UI over faithfully, at the
upstream commits pinned in [UPSTREAM.lock](UPSTREAM.lock), and their forked web UIs are served with the Encore look.
What is ported so far is tracked in [docs/PARITY.md](docs/PARITY.md). All credit for the original design and code
goes to their authors:

| Project | License | Ported for |
| --- | --- | --- |
| [Sonarr](https://github.com/Sonarr/Sonarr) (fork: [Krifmaz/Sonarr](https://github.com/Krifmaz/Sonarr)) | GPL-3.0 | TV: backend behaviour, the Sonarr v3 API and the web UI |
| [Radarr](https://github.com/Radarr/Radarr) (fork: [Krifmaz/Radarr](https://github.com/Krifmaz/Radarr)) | GPL-3.0 | movies: backend behaviour, the Radarr v3 API and the web UI |
| [Seerr](https://github.com/seerr-team/seerr) (Overseerr, Jellyseerr; fork: [Krifmaz/seerr](https://github.com/Krifmaz/seerr)) | MIT | requests: backend behaviour, the Seerr API and the web UI |
| [Prowlarr](https://github.com/Prowlarr/Prowlarr) | GPL-3.0 | reference for how Prowlarr adds Sonarr and Radarr apps and syncs indexers |

Seerr's MIT code is used under its MIT license (copyright notice kept with the ported code); together with the GPL-3.0
code of Sonarr and Radarr the whole is distributed under GPL-3.0-only.

Data used as it is:

| Data | License | Used for |
| --- | --- | --- |
| [TRaSH Guides](https://github.com/TRaSH-Guides/Guides) | MIT | quality profiles, custom formats and scores, quality sizes, naming |
| [Prowlarr indexer definitions](https://github.com/Prowlarr/Indexers) | GPL-3.0 | indexer YAML definitions |
| [TMDB](https://www.themoviedb.org) | API terms | movie and TV metadata. This product uses the TMDB API but is not endorsed or certified by TMDB. |
| [TheTVDB](https://thetvdb.com) | API terms | TV metadata |
| [anime-lists (Fribb)](https://github.com/Fribb/anime-lists) | MIT | anime ids across AniList, AniDB, MAL, TVDB and TMDB |
| [AniList](https://anilist.co) | API terms | anime season lists and details on request pages (GraphQL API) |
| [FFmpeg (ffprobe)](https://ffmpeg.org), static build by [mwader/static-ffmpeg](https://github.com/wader/static-ffmpeg) | LGPL-2.1 / GPL | reads the real audio and subtitle tracks of anime files; shipped in the Docker image |

Built on the [Krifmaz kit](https://github.com/Krifmaz/kit) (MIT).
