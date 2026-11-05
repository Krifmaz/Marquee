# The forked web apps

Marquee serves the real web apps of Sonarr, Radarr and Seerr, built from the commits pinned in `UPSTREAM.lock`, with
a few patches so they look like the rest of Marquee (the kit's Encore theme) and live under Marquee's addresses:

| App | Address | Talks to | Who |
| --- | --- | --- | --- |
| Sonarr | `/sonarr/` | `/sonarr/api/v3` | admins |
| Radarr | `/radarr/` | `/radarr/api/v3` | admins |
| Seerr | `/requests/` | `/api/v1` | everyone signed in |

- `build-all.sh <UPSTREAM.lock> <tokens.css> <work> <out>` fetches each pinned commit (the Krifmaz fork first, else
  upstream) and runs `build.sh` for it; the Dockerfile's `upstream-ui` stage runs it, and the image keeps the result
  in `/app/upstream-ui`.
- `arr/patch.mjs` + `arr/encore.js`: every Sonarr/Radarr theme becomes Encore (the upstream dark theme with every
  colour taken from the kit's tokens), the kit's font, and `index.html` loads the kit's `tokens.css`.
- `seerr/patch.mjs`: a static build (`output: 'export'`) at `/requests`, the sign-in read in the browser (signed-out
  visitors go to Marquee's sign-in and come back), Marquee's CSRF header on every change, and Tailwind's grays and
  indigos from the kit's tokens.
- `server/src/upstream_ui.rs` serves the builds: `index.html` and `initialize.json` (with the API key) only for
  admins on Sonarr/Radarr, and Seerr's pages found by their address (`/requests/movie/603` →
  `movie/[movieId].html`).

Every patch checks that the upstream text it changes is still there, so moving `UPSTREAM.lock` to a newer release
fails the build loudly instead of quietly losing the look. Run one build by hand with Node 22, yarn and pnpm:

```
ui/build-all.sh UPSTREAM.lock web/static/css/tokens.css /tmp/upstream /tmp/upstream-ui
APP_UPSTREAM_UI_DIR=/tmp/upstream-ui cargo run --manifest-path server/Cargo.toml
```
