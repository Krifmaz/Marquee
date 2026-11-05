# Lantern hub contract

Marquee searches and downloads only through [Lantern](https://github.com/Krifmaz/lantern). This page is what
Marquee sends and expects. Lantern's download hub (its build step 6) answers it. Every call carries Lantern's API
key in `X-Api-Key`. Errors answer `{"detail": "<plain sentence>"}`.

## Searching (Lantern's Prowlarr API, there today)

`GET /api/v1/search?type=movie|tvsearch&query=…&imdbid=…&tmdbid=…&tvdbid=…&season=…&ep=…&categories=2000&limit=100`

The answer is a list of releases:
`[{guid, title, size, protocol, indexer, indexerId, seeders, leechers, downloadUrl, magnetUrl, infoUrl, publishDate, age}]`.
Marquee uses categories 2000 for movies, 5000 for TV, and 5070 then 5000 for anime.

## Registering

`POST /api/v1/hub/apps` `{name: "Marquee", kinds: ["movie", "tv", "anime"], callback, callbackKey}`

Marquee sends this after start-up, and again whenever the Lantern address or key changes. It is safe to repeat.
`callback` is `<Marquee address>/api/lantern/events`. It is empty when Settings → Connections has no address for
Marquee. Lantern makes the app's categories and folders. The answer is any JSON.

## Grabbing

`POST /api/v1/hub/grabs` `{app: "Marquee", kind: "movie"|"tv"|"anime", ref, release}`

- `ref` is `movie:<id>` or `episodes:<id>,<id>`. Marquee reads it back.
- `release` is a release exactly as the search sent it.

The answer is the new grab (201):

`{id, ref, title, status, progress, size, left, eta, error, files, protocol}`

- `status` is one of `queued`, `downloading`, `paused`, `completed`, `failed` or `removed`.
- `progress` goes from 0 to 1.
- `eta` is in seconds, or null.
- `error` says in words what went wrong, or is null.
- `files` holds the finished files or folders once the grab is `completed`. The paths are as Marquee sees them, so
  Lantern applies its path mappings. They are on the same disk as the media folders, so they can be hardlinked.

## Following downloads

- `GET /api/v1/hub/grabs?app=Marquee` returns every grab Lantern keeps for Marquee, in the shape above. Marquee
  asks every minute. A grab that is missing from the answer is shown as stopped.
- `POST <callback>` with Marquee's API key in `X-Api-Key` and `{event, grab}` gives Marquee the news at once.
  `event` is `grab.progress`, `grab.completed`, `grab.failed` or `grab.removed`. Marquee answers
  `{ok, known}`.
- `POST /api/v1/hub/grabs/{id}/imported` comes from Marquee once the files are in the library. From then on,
  Lantern's seeding and clean-up rules apply.
- `DELETE /api/v1/hub/grabs/{id}?removeData=true|false` comes from Marquee when someone stops a download.

## Importing (Marquee's side)

For a movie, Marquee takes the biggest video in `files`. Samples are left out. The video is hardlinked as
`<movie folder>/<Title> (<Year>) <Quality>.<ext>`, the way Radarr names files.

For TV, each video's name says its episodes. Each one is hardlinked as
`<series folder>/Season <n>/<Series> - S01E02 - <Episode title> <Quality>.<ext>`, the way Sonarr names files.

Subtitles that sit next to a video come along with it. Marquee copies a file only when the download is on another
disk. A file that gets replaced is moved to `<root folder>/.marquee-recycle/` and is never deleted.
