package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.parseJson
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Anime tab's rules: the library by season, the countdowns, Discover and the request body. */
class AnimeRulesTest {
    /** Each season is its own entry with its badge; an airing one counts down to its next episode. */
    @Test
    fun library() {
        val answer =
            parseJson(
                """
                [
                  {"key": "s1-2-1", "kind": "season", "title": "Frieren Season 2", "poster": "p", "status": "Airing",
                   "monitored": true, "files": 4, "episodes": 12, "next_episode": {"episode": 5, "airs": 1000000}},
                  {"key": "s1-1-1", "kind": "season", "title": "Frieren", "status": "Finished", "monitored": true, "files": 28, "episodes": 28},
                  {"key": "m3", "kind": "movie", "title": "Suzume", "status": "Finished", "monitored": true, "files": 0, "episodes": 1}
                ]
                """.trimIndent(),
            )
        val got = AnimeRules.library(answer)
        assertEquals(listOf("Airing", "Complete", "Missing"), got.map { it.badge })
        assertEquals("Episode 5 in 2 days 4 hours", AnimeRules.line(got[0], 1_000_000L - 2 * 86_400 - 4 * 3600))
        assertEquals("28 of 28 episodes", AnimeRules.line(got[1], 0))
        assertEquals("Film", AnimeRules.line(got[2], 0))
    }

    /** Countdowns in words. */
    @Test
    fun countdown() {
        assertEquals("now", AnimeRules.countdown(10))
        assertEquals("in 12 minutes", AnimeRules.countdown(12 * 60))
        assertEquals("in 1 hour 1 minute", AnimeRules.countdown(3660))
        assertEquals("in 2 days", AnimeRules.countdown(2 * 86_400))
    }

    /** Discover reads AniList's entries, and a request asks for the entry's TheTVDB season with the language. */
    @Test
    fun discoverAndRequest() {
        val answer =
            parseJson(
                """
                {"results": [
                  {"anilist_id": 182255, "title": "Frieren Season 2", "season": "Winter 2026", "status": "Airing", "media_type": "tv",
                   "tmdb_id": 209867, "tvdb_id": 424536, "tvdb_season": 2, "media_status": null},
                  {"anilist_id": 1, "title": "Unmapped", "media_type": "tv"},
                  {"anilist_id": 2, "title": "Owned", "media_type": "tv", "tmdb_id": 5, "series_id": 9}
                ], "more": false}
                """.trimIndent(),
            )
        val picks = AnimeRules.discover(answer)
        assertEquals("Winter 2026 · Airing", picks[0].line)
        assertTrue(AnimeRules.canRequest(picks[0]))
        assertFalse("not mapped to TMDB yet", AnimeRules.canRequest(picks[1]))
        assertEquals("In the library", picks[2].badge)
        assertEquals("season 2", AnimeRules.what(picks[0]))
        val body = AnimeRules.body(picks[0], "sub", "es")
        assertEquals(JsonPrimitive("sub"), body["animeAudio"])
        assertEquals(JsonPrimitive("es"), body["animeSubtitles"])
        assertEquals(JsonPrimitive(2), body["seasons"]!!.jsonArray[0])
        assertEquals(JsonPrimitive(209867L), body["mediaId"])
    }

    /** The account's defaults come first; an unknown subtitle language is kept as a choice. */
    @Test
    fun defaults() {
        assertEquals("sub" to "de", AnimeRules.defaults(parseJson("""{"animeAudio": "sub", "animeSubtitles": "de"}""")))
        assertEquals("dual" to "en", AnimeRules.defaults(null))
        assertEquals("FI" to "fi", AnimeRules.subtitleChoices("fi").first())
        assertEquals(AnimeRules.SUBTITLES, AnimeRules.subtitleChoices("en"))
    }
}
