# Seerr API parity

Written by `scripts/parity.py`; don't edit by hand. Upstream: Seerr v3.5.1 ([`564bfb08b5`](https://github.com/seerr-team/seerr/tree/564bfb08b54caa374bddaf820abfd226e5e69151)). Marquee serves it under `/api/v1` (no prefix).

**70 done, 0 stub, 147 missing of 217.** Stub means Marquee answers with a fixed empty value so clients don't fail, but does nothing yet.

| Method | Path | Marquee | Upstream summary |
| --- | --- | --- | --- |
| `POST` | `/auth/jellyfin` | ⬜ Missing | Sign in using a Jellyfin username and password |
| `POST` | `/auth/jellyfin/quickconnect/authenticate` | ⬜ Missing | Authenticate with Quick Connect |
| `GET` | `/auth/jellyfin/quickconnect/check` | ⬜ Missing | Check Quick Connect authorization status |
| `POST` | `/auth/jellyfin/quickconnect/initiate` | ⬜ Missing | Initiate Jellyfin Quick Connect |
| `POST` | `/auth/local` | ⬜ Missing | Sign in using a local account |
| `POST` | `/auth/logout` | ⬜ Missing | Sign out and clear session cookie |
| `GET` | `/auth/me` | ✅ Done | Get logged-in user |
| `POST` | `/auth/plex` | ⬜ Missing | Sign in using a Plex token |
| `POST` | `/auth/reset-password` | ⬜ Missing | Send a reset password email |
| `POST` | `/auth/reset-password/{guid}` | ⬜ Missing | Reset the password for a user |
| `GET` | `/backdrops` | ⬜ Missing | Get backdrops of trending items |
| `GET` | `/blacklist` | ⬜ Missing | Returns blocklisted items |
| `POST` | `/blacklist` | ⬜ Missing | Add media to blocklist |
| `DELETE` | `/blacklist/{tmdbId}` | ⬜ Missing | Remove media from blocklist |
| `GET` | `/blacklist/{tmdbId}` | ⬜ Missing | Get media from blocklist |
| `GET` | `/blocklist` | ✅ Done | Returns blocklisted items |
| `POST` | `/blocklist` | ✅ Done | Add media to blocklist |
| `DELETE` | `/blocklist/collection/{collectionId}` | ✅ Done | Remove collection from blocklist |
| `POST` | `/blocklist/collection/{collectionId}` | ✅ Done | Add collection to blocklist |
| `DELETE` | `/blocklist/{tmdbId}` | ✅ Done | Remove media from blocklist |
| `GET` | `/blocklist/{tmdbId}` | ✅ Done | Get media from blocklist |
| `GET` | `/certifications/movie` | ⬜ Missing | Get movie certifications |
| `GET` | `/certifications/tv` | ⬜ Missing | Get TV certifications |
| `GET` | `/collection/{collectionId}` | ⬜ Missing | Get collection details |
| `GET` | `/discover/genreslider/movie` | ✅ Done | Get genre slider data for movies |
| `GET` | `/discover/genreslider/tv` | ✅ Done | Get genre slider data for TV series |
| `GET` | `/discover/keyword/{keywordId}/movies` | ✅ Done | Get movies from keyword |
| `GET` | `/discover/movies` | ✅ Done | Discover movies |
| `GET` | `/discover/movies/genre/{genreId}` | ✅ Done | Discover movies by genre |
| `GET` | `/discover/movies/language/{language}` | ✅ Done | Discover movies by original language |
| `GET` | `/discover/movies/studio/{studioId}` | ✅ Done | Discover movies by studio |
| `GET` | `/discover/movies/upcoming` | ✅ Done | Upcoming movies |
| `GET` | `/discover/trending` | ✅ Done | Trending movies and TV |
| `GET` | `/discover/tv` | ✅ Done | Discover TV shows |
| `GET` | `/discover/tv/genre/{genreId}` | ✅ Done | Discover TV shows by genre |
| `GET` | `/discover/tv/language/{language}` | ✅ Done | Discover TV shows by original language |
| `GET` | `/discover/tv/network/{networkId}` | ✅ Done | Discover TV shows by network |
| `GET` | `/discover/tv/upcoming` | ✅ Done | Discover Upcoming TV shows |
| `GET` | `/discover/watchlist` | ✅ Done | Get the Plex watchlist. |
| `GET` | `/genres/movie` | ⬜ Missing | Get list of official TMDB movie genres |
| `GET` | `/genres/tv` | ⬜ Missing | Get list of official TMDB movie genres |
| `GET` | `/issue` | ⬜ Missing | Get all issues |
| `POST` | `/issue` | ⬜ Missing | Create new issue |
| `GET` | `/issue/count` | ✅ Done | Gets issue counts |
| `DELETE` | `/issue/{issueId}` | ⬜ Missing | Delete issue |
| `GET` | `/issue/{issueId}` | ⬜ Missing | Get issue |
| `POST` | `/issue/{issueId}/comment` | ⬜ Missing | Create a comment |
| `POST` | `/issue/{issueId}/{status}` | ⬜ Missing | Update an issue's status |
| `DELETE` | `/issueComment/{commentId}` | ⬜ Missing | Delete issue comment |
| `GET` | `/issueComment/{commentId}` | ⬜ Missing | Get issue comment |
| `PUT` | `/issueComment/{commentId}` | ⬜ Missing | Update issue comment |
| `GET` | `/keyword/{keywordId}` | ⬜ Missing | Get keyword |
| `GET` | `/languages` | ⬜ Missing | Languages supported by TMDB |
| `GET` | `/media` | ✅ Done | Get media |
| `DELETE` | `/media/{mediaId}` | ✅ Done | Delete media item |
| `DELETE` | `/media/{mediaId}/file` | ✅ Done | Delete media file |
| `GET` | `/media/{mediaId}/watch_data` | ✅ Done | Get watch data |
| `POST` | `/media/{mediaId}/{status}` | ✅ Done | Update media status |
| `GET` | `/movie/{movieId}` | ✅ Done | Get movie details |
| `GET` | `/movie/{movieId}/ratings` | ✅ Done | Get movie ratings |
| `GET` | `/movie/{movieId}/ratingscombined` | ✅ Done | Get RT and IMDB movie ratings combined |
| `GET` | `/movie/{movieId}/recommendations` | ✅ Done | Get recommended movies |
| `GET` | `/movie/{movieId}/similar` | ✅ Done | Get similar movies |
| `GET` | `/network/{networkId}` | ⬜ Missing | Get TV network details |
| `GET` | `/overrideRule` | ⬜ Missing | Get override rules |
| `POST` | `/overrideRule` | ⬜ Missing | Create override rule |
| `POST` | `/overrideRule/advancedRequest` | ⬜ Missing | Advanced override rule request |
| `DELETE` | `/overrideRule/{ruleId}` | ⬜ Missing | Delete override rule by ID |
| `PUT` | `/overrideRule/{ruleId}` | ⬜ Missing | Update override rule |
| `GET` | `/person/{personId}` | ⬜ Missing | Get person details |
| `GET` | `/person/{personId}/combined_credits` | ⬜ Missing | Get combined credits |
| `GET` | `/regions` | ⬜ Missing | Regions supported by TMDB |
| `GET` | `/request` | ✅ Done | Get all requests |
| `POST` | `/request` | ✅ Done | Create new request |
| `GET` | `/request/count` | ✅ Done | Gets request counts |
| `DELETE` | `/request/{requestId}` | ✅ Done | Delete request |
| `GET` | `/request/{requestId}` | ✅ Done | Get MediaRequest |
| `PUT` | `/request/{requestId}` | ✅ Done | Update MediaRequest |
| `POST` | `/request/{requestId}/retry` | ✅ Done | Retry failed request |
| `POST` | `/request/{requestId}/{status}` | ✅ Done | Update a request's status |
| `GET` | `/search` | ✅ Done | Search for movies, TV shows, or people |
| `GET` | `/search/company` | ⬜ Missing | Search for companies |
| `GET` | `/search/keyword` | ⬜ Missing | Search for keywords |
| `GET` | `/service/radarr` | ⬜ Missing | Get non-sensitive Radarr server list |
| `GET` | `/service/radarr/{radarrId}` | ⬜ Missing | Get Radarr server quality profiles and root folders |
| `GET` | `/service/sonarr` | ⬜ Missing | Get non-sensitive Sonarr server list |
| `GET` | `/service/sonarr/lookup/{tmdbId}` | ⬜ Missing | Get series from Sonarr |
| `GET` | `/service/sonarr/{sonarrId}` | ⬜ Missing | Get Sonarr server quality profiles and root folders |
| `GET` | `/settings/about` | ⬜ Missing | Get server stats |
| `GET` | `/settings/cache` | ⬜ Missing | Get a list of active caches |
| `POST` | `/settings/cache/dns/{dnsEntry}/flush` | ⬜ Missing | Flush a specific DNS cache entry |
| `POST` | `/settings/cache/{cacheId}/flush` | ⬜ Missing | Flush a specific cache |
| `GET` | `/settings/discover` | ✅ Done | Get all discover sliders |
| `POST` | `/settings/discover` | ✅ Done | Batch update all sliders. |
| `POST` | `/settings/discover/add` | ✅ Done | Add a new slider |
| `GET` | `/settings/discover/reset` | ✅ Done | Reset all discover sliders |
| `DELETE` | `/settings/discover/{sliderId}` | ✅ Done | Delete slider by ID |
| `PUT` | `/settings/discover/{sliderId}` | ✅ Done | Update a single slider |
| `POST` | `/settings/initialize` | ⬜ Missing | Initialize application |
| `GET` | `/settings/jellyfin` | ⬜ Missing | Get Jellyfin settings |
| `POST` | `/settings/jellyfin` | ⬜ Missing | Update Jellyfin settings |
| `GET` | `/settings/jellyfin/library` | ⬜ Missing | Get Jellyfin libraries |
| `POST` | `/settings/jellyfin/library/sync` | ⬜ Missing | Sync Jellyfin libraries |
| `PUT` | `/settings/jellyfin/library/{libraryId}` | ⬜ Missing | Update a single Jellyfin library |
| `GET` | `/settings/jellyfin/sync` | ⬜ Missing | Get status of full Jellyfin library sync |
| `POST` | `/settings/jellyfin/sync` | ⬜ Missing | Start full Jellyfin library sync |
| `GET` | `/settings/jellyfin/users` | ⬜ Missing | Get Jellyfin Users |
| `GET` | `/settings/jobs` | ⬜ Missing | Get scheduled jobs |
| `POST` | `/settings/jobs/{jobId}/cancel` | ⬜ Missing | Cancel a specific job |
| `POST` | `/settings/jobs/{jobId}/run` | ⬜ Missing | Invoke a specific job |
| `POST` | `/settings/jobs/{jobId}/schedule` | ⬜ Missing | Modify job schedule |
| `GET` | `/settings/logs` | ⬜ Missing | Returns logs |
| `GET` | `/settings/main` | ⬜ Missing | Get main settings |
| `POST` | `/settings/main` | ⬜ Missing | Update main settings |
| `POST` | `/settings/main/regenerate` | ⬜ Missing | Get main settings with newly-generated API key |
| `GET` | `/settings/metadatas` | ⬜ Missing | Get Metadata settings |
| `PUT` | `/settings/metadatas` | ⬜ Missing | Update Metadata settings |
| `POST` | `/settings/metadatas/test` | ⬜ Missing | Test Provider configuration |
| `GET` | `/settings/network` | ⬜ Missing | Get network settings |
| `POST` | `/settings/network` | ⬜ Missing | Update network settings |
| `GET` | `/settings/notifications/discord` | ⬜ Missing | Get Discord notification settings |
| `POST` | `/settings/notifications/discord` | ⬜ Missing | Update Discord notification settings |
| `POST` | `/settings/notifications/discord/test` | ⬜ Missing | Test Discord settings |
| `GET` | `/settings/notifications/email` | ⬜ Missing | Get email notification settings |
| `POST` | `/settings/notifications/email` | ⬜ Missing | Update email notification settings |
| `POST` | `/settings/notifications/email/test` | ⬜ Missing | Test email settings |
| `GET` | `/settings/notifications/gotify` | ⬜ Missing | Get Gotify notification settings |
| `POST` | `/settings/notifications/gotify` | ⬜ Missing | Update Gotify notification settings |
| `POST` | `/settings/notifications/gotify/test` | ⬜ Missing | Test Gotify settings |
| `GET` | `/settings/notifications/ntfy` | ⬜ Missing | Get ntfy.sh notification settings |
| `POST` | `/settings/notifications/ntfy` | ⬜ Missing | Update ntfy.sh notification settings |
| `POST` | `/settings/notifications/ntfy/test` | ⬜ Missing | Test ntfy.sh settings |
| `GET` | `/settings/notifications/pushbullet` | ⬜ Missing | Get Pushbullet notification settings |
| `POST` | `/settings/notifications/pushbullet` | ⬜ Missing | Update Pushbullet notification settings |
| `POST` | `/settings/notifications/pushbullet/test` | ⬜ Missing | Test Pushbullet settings |
| `GET` | `/settings/notifications/pushover` | ⬜ Missing | Get Pushover notification settings |
| `POST` | `/settings/notifications/pushover` | ⬜ Missing | Update Pushover notification settings |
| `GET` | `/settings/notifications/pushover/sounds` | ⬜ Missing | Get Pushover sounds |
| `POST` | `/settings/notifications/pushover/test` | ⬜ Missing | Test Pushover settings |
| `GET` | `/settings/notifications/slack` | ⬜ Missing | Get Slack notification settings |
| `POST` | `/settings/notifications/slack` | ⬜ Missing | Update Slack notification settings |
| `POST` | `/settings/notifications/slack/test` | ⬜ Missing | Test Slack settings |
| `GET` | `/settings/notifications/telegram` | ⬜ Missing | Get Telegram notification settings |
| `POST` | `/settings/notifications/telegram` | ⬜ Missing | Update Telegram notification settings |
| `POST` | `/settings/notifications/telegram/test` | ⬜ Missing | Test Telegram settings |
| `GET` | `/settings/notifications/webhook` | ⬜ Missing | Get webhook notification settings |
| `POST` | `/settings/notifications/webhook` | ⬜ Missing | Update webhook notification settings |
| `POST` | `/settings/notifications/webhook/test` | ⬜ Missing | Test webhook settings |
| `GET` | `/settings/notifications/webpush` | ⬜ Missing | Get Web Push notification settings |
| `POST` | `/settings/notifications/webpush` | ⬜ Missing | Update Web Push notification settings |
| `POST` | `/settings/notifications/webpush/test` | ⬜ Missing | Test Web Push settings |
| `GET` | `/settings/plex` | ⬜ Missing | Get Plex settings |
| `POST` | `/settings/plex` | ⬜ Missing | Update Plex settings |
| `GET` | `/settings/plex/devices/servers` | ⬜ Missing | Gets the user's available Plex servers |
| `GET` | `/settings/plex/library` | ⬜ Missing | Get Plex libraries |
| `POST` | `/settings/plex/library/sync` | ⬜ Missing | Sync Plex libraries |
| `PUT` | `/settings/plex/library/{libraryId}` | ⬜ Missing | Update a single Plex library |
| `GET` | `/settings/plex/sync` | ⬜ Missing | Get status of full Plex library scan |
| `POST` | `/settings/plex/sync` | ⬜ Missing | Start full Plex library scan |
| `GET` | `/settings/plex/users` | ⬜ Missing | Get Plex users |
| `GET` | `/settings/public` | ✅ Done | Get public settings |
| `GET` | `/settings/radarr` | ⬜ Missing | Get Radarr settings |
| `POST` | `/settings/radarr` | ⬜ Missing | Create Radarr instance |
| `POST` | `/settings/radarr/test` | ⬜ Missing | Test Radarr configuration |
| `DELETE` | `/settings/radarr/{radarrId}` | ⬜ Missing | Delete Radarr instance |
| `PUT` | `/settings/radarr/{radarrId}` | ⬜ Missing | Update Radarr instance |
| `GET` | `/settings/radarr/{radarrId}/profiles` | ⬜ Missing | Get available Radarr profiles |
| `GET` | `/settings/sonarr` | ⬜ Missing | Get Sonarr settings |
| `POST` | `/settings/sonarr` | ⬜ Missing | Create Sonarr instance |
| `POST` | `/settings/sonarr/test` | ⬜ Missing | Test Sonarr configuration |
| `DELETE` | `/settings/sonarr/{sonarrId}` | ⬜ Missing | Delete Sonarr instance |
| `PUT` | `/settings/sonarr/{sonarrId}` | ⬜ Missing | Update Sonarr instance |
| `GET` | `/settings/tautulli` | ⬜ Missing | Get Tautulli settings |
| `POST` | `/settings/tautulli` | ⬜ Missing | Update Tautulli settings |
| `GET` | `/status` | ✅ Done | Get Seerr status |
| `GET` | `/status/appdata` | ✅ Done | Get application data volume status |
| `GET` | `/studio/{studioId}` | ⬜ Missing | Get movie studio details |
| `GET` | `/tv/{tvId}` | ✅ Done | Get TV details |
| `GET` | `/tv/{tvId}/ratings` | ✅ Done | Get TV ratings |
| `GET` | `/tv/{tvId}/recommendations` | ✅ Done | Get recommended TV series |
| `GET` | `/tv/{tvId}/season/{seasonNumber}` | ✅ Done | Get season details and episode list |
| `GET` | `/tv/{tvId}/similar` | ✅ Done | Get similar TV series |
| `GET` | `/user` | ✅ Done | Get all users |
| `POST` | `/user` | ✅ Done | Create new user |
| `PUT` | `/user` | ✅ Done | Update batch of users |
| `POST` | `/user/import-from-jellyfin` | ⬜ Missing | Import all users from Jellyfin |
| `POST` | `/user/import-from-plex` | ⬜ Missing | Import all users from Plex |
| `GET` | `/user/jellyfin/{jellyfinUserId}` | ⬜ Missing | Get user by Jellyfin user ID |
| `POST` | `/user/registerPushSubscription` | ⬜ Missing | Register a web push /user/registerPushSubscription |
| `DELETE` | `/user/{userId}` | ✅ Done | Delete user by ID |
| `GET` | `/user/{userId}` | ✅ Done | Get user by ID |
| `PUT` | `/user/{userId}` | ✅ Done | Update a user by user ID |
| `DELETE` | `/user/{userId}/pushSubscription/{endpoint}` | ⬜ Missing | Delete user push subscription by key |
| `GET` | `/user/{userId}/pushSubscription/{endpoint}` | ⬜ Missing | Get web push notification settings for a user |
| `GET` | `/user/{userId}/pushSubscriptions` | ⬜ Missing | Get all web push notification settings for a user |
| `GET` | `/user/{userId}/quota` | ✅ Done | Get quotas for a specific user |
| `GET` | `/user/{userId}/requests` | ✅ Done | Get requests for a specific user |
| `DELETE` | `/user/{userId}/settings/linked-accounts/jellyfin` | ⬜ Missing | Remove the linked Jellyfin account for a user |
| `POST` | `/user/{userId}/settings/linked-accounts/jellyfin` | ⬜ Missing | Link the provided Jellyfin account to the current user |
| `POST` | `/user/{userId}/settings/linked-accounts/jellyfin/quickconnect` | ⬜ Missing | Link Jellyfin/Emby account with Quick Connect |
| `DELETE` | `/user/{userId}/settings/linked-accounts/plex` | ⬜ Missing | Remove the linked Plex account for a user |
| `POST` | `/user/{userId}/settings/linked-accounts/plex` | ⬜ Missing | Link the provided Plex account to the current user |
| `GET` | `/user/{userId}/settings/main` | ✅ Done | Get general settings for a user |
| `POST` | `/user/{userId}/settings/main` | ✅ Done | Update general settings for a user |
| `GET` | `/user/{userId}/settings/notifications` | ⬜ Missing | Get notification settings for a user |
| `POST` | `/user/{userId}/settings/notifications` | ⬜ Missing | Update notification settings for a user |
| `GET` | `/user/{userId}/settings/password` | ⬜ Missing | Get password page informatiom |
| `POST` | `/user/{userId}/settings/password` | ⬜ Missing | Update password for a user |
| `GET` | `/user/{userId}/settings/permissions` | ✅ Done | Get permission settings for a user |
| `POST` | `/user/{userId}/settings/permissions` | ✅ Done | Update permission settings for a user |
| `GET` | `/user/{userId}/watch_data` | ✅ Done | Get watch data |
| `GET` | `/user/{userId}/watchlist` | ✅ Done | Get the Plex watchlist for a specific user |
| `POST` | `/watchlist` | ⬜ Missing | Add media to watchlist |
| `DELETE` | `/watchlist/{tmdbId}` | ⬜ Missing | Delete watchlist item |
| `GET` | `/watchproviders/movies` | ⬜ Missing | Get watch provider movies |
| `GET` | `/watchproviders/regions` | ⬜ Missing | Get watch provider regions |
| `GET` | `/watchproviders/tv` | ⬜ Missing | Get watch provider series |
