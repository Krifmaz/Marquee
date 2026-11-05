# Decisions

One line per call that was not obvious.

- 2026-11-05 · The repository was cleared and regenerated from the kit template (kit v0.2.1, the latest tag) as the owner asked; the old Marqueue code (playbill, Scoutline) stays in git history.
- 2026-11-05 · `main` did not exist (the default branch was a session branch); it was made from that branch so the first pull request has a base, and is the branch releases run on.
- 2026-11-05 · Releases are held until v0.1.0: `docs/RELEASE_HOLD` makes the release workflow skip, because the owner wants v0.1.0 only when movies, TV, downloads and TRaSH defaults work end to end. The rule "every merge ships a release" applies from v0.1.0 on.
- 2026-11-05 · The template's desktop app is kept (the kit's proof app, free CI coverage); the Android app is added in its own step at the end, as the owner ordered.
- 2026-11-05 · Prowlarr still adds Marquee as Sonarr and Radarr (goal 2, the API stays compatible), but Marquee keeps no indexers of its own: indexers Prowlarr pushes are accepted and handed to Lantern, or answered as an empty list, decided in the downloads step.
- 2026-11-05 · The template's .gitignore only ignores /target, but each of server/, web/ and desktop/ is its own workspace with its own target/; Marquee ignores target/ everywhere. To fix in the kit (not in this session's reach).
