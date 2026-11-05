# app

Marquee's Android app (`dev.krifmaz.marquee`; debug builds install beside it as `dev.krifmaz.marquee.debug`).
One activity in the kit's default theme; every page is a kit-compose part, wired to this repository's server.

| File | What it is |
| --- | --- |
| `App.kt` | The process: crash log, `KitApp.init`, where updates come from, the server and its calls |
| `MainActivity.kt` | The theme, the pages and the toasts; the daily update check |
| `AppShell.kt` | The pages as a stack (Back goes down), and the bug report screens' host |
| `ConnectPage.kt` | First start: the server's address |
| `SignInPage.kt` | Username and password (and the authenticator code), the kit's phone sign-in |
| `MainPage.kt` | Signed in: the offline banner, What's new, and the tabs Calendar, Queue, Library and More |
| `CalendarPage.kt` | A week of movies coming out and episodes airing, by day (`GET /api/calendar`); Earlier and Later |
| `RequestsPage.kt` | Requests on the Seerr API (`/api/v1/request`): where each one is; admins approve, decline, retry. Ask for something: Trending or a search, then a movie or seasons |
| `QueuePage.kt` | What is downloading and what finished lately (`GET /api/downloads`, every 5 s); admins can stop one |
| `LibraryPage.kt` | Every movie (`GET /api/movies`) and series (`GET /api/series`), with a title search; admins can search now |
| `HomePage.kt` | The More tab: the server, What's new, admin pages, sign out, bug reports, crash log, updates |
| `Rows.kt`, `Loaded.kt` | The poster row; loading a list with its saved copy (offline mode) and the line about it |
| `CalendarRules.kt`, `RequestRules.kt`, `QueueRules.kt`, `LibraryRules.kt` | Reading the answers and their words (unit-tested in `src/test`) |
| `AnswerCache.kt` | The saved copy of each list, forgotten on sign-out and on a server change (unit-tested) |
| `AdminPages.kt` | Server settings (forms from the server's schema) and Diagnostics (with the app's crash reports) |
| `Server.kt` | The address and token, the server's API (`KitApi`), What's new calls, the network check |
| `ServerAddress.kt` | What is tried for a typed address (unit-tested in `src/test`) |
| `SignInRules.kt` | The sign-in's body, its answer and words (unit-tested in `src/test`) |

| kit-compose part | Where | Server |
| --- | --- | --- |
| Theme, toasts, details | `MainActivity`, `HomePage` | — |
| What's new popup and changelog | `MainPage` (`WhatsNewOnStart`), `AppShell` (`ChangelogScreen`) | `/api/changelog…`, `PUT /api/auth/prefs` |
| In-app updates | `App` (`UpdateRules.source`), `MainActivity`, `HomePage` (`UpdatesSection`) | GitHub releases of Krifmaz/marquee |
| Bug reports and crash logs | `App` (`CrashLog`), `AppShell`, `HomePage` | `/api/bug-reports…`, `/api/crash-reports` |
| Offline banner | `MainPage` (`probeHealth`, `OfflineBanner`) | `/api/health` |
| Settings forms | `AdminPages` (`SchemaPage`) | `/api/settings…`, `/api/diagnostics/…/test` |
| Diagnostics | `AdminPages` (`DiagnosticsScreen`, `AppCrashesSection`) | `/api/diagnostics` |

Signing in is the kit's phone sign-in: `POST /api/auth/login` with `{username, password, app: {device_id,
device_name}}` answers `{user, token}`; the token goes with every request as `Authorization: Bearer <token>` (only
to the server's own address), with the User-Agent `<app name>-Android/<version>`. A 401 from any later call goes
back to the sign-in. Admin pages and actions show unless `GET /api/auth/me` says the account isn't one.

The app uses Marquee's own API (the web app's), not the Radarr and Sonarr APIs: those need the server's API key,
which a phone shouldn't hold. Requests use Marquee's Seerr API (`/api/v1`), which takes the same app token.
