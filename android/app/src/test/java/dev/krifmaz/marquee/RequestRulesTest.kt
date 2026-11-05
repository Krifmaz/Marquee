package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.parseJson
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Requests tab's rules: reading the Seerr API's requests, search results and seasons, and the request body. */
class RequestRulesTest {
    /** Requests with Marquee's words, who asked, the seasons and whether they wait. */
    @Test
    fun readsRequests() {
        val answer =
            parseJson(
                """
                {"pageInfo": {"pages": 1, "pageSize": 50, "results": 2, "page": 1}, "results": [
                  {"id": 2, "status": 1, "type": "tv", "media": {"status": 2}, "seasons": [{"seasonNumber": 2}, {"seasonNumber": 1}],
                   "requestedBy": {"username": "sam", "displayName": "Sam"}, "canRemove": true,
                   "statusWords": "Waiting for approval", "title": "Game of Thrones", "year": 2011, "posterUrl": "https://img/got.jpg", "message": ""},
                  {"id": 1, "status": 4, "type": "movie", "media": {"status": 3}, "seasons": [], "requestedBy": {"username": "ana"},
                   "canRemove": false, "title": "Dune", "year": 0, "message": "Lantern isn't set up"}
                ]}
                """.trimIndent(),
            )
        val r = RequestRules.requests(answer)
        assertEquals(2, r.size)
        assertEquals("Game of Thrones (2011)", r[0].title)
        assertEquals("Seasons 1 and 2", r[0].seasons)
        assertEquals("Sam", r[0].requestedBy)
        assertTrue(r[0].waiting)
        assertEquals("warn", RequestRules.badgeKind(r[0]))
        assertEquals("Dune", r[1].title)
        assertEquals("the server's words, else Marquee's own", "Didn't work", r[1].words)
        assertEquals("ana", r[1].requestedBy)
        assertEquals("err", RequestRules.badgeKind(r[1]))
        assertTrue(RequestRules.requests(parseJson("[]")).isEmpty())
    }

    /** Season lists in words. */
    @Test
    fun seasonWords() {
        assertEquals("", RequestRules.seasonWords(emptyList()))
        assertEquals("Season 3", RequestRules.seasonWords(listOf(3)))
        assertEquals("Seasons 1, 2 and 4", RequestRules.seasonWords(listOf(4, 1, 2)))
    }

    /** Search results: movies and series with their year, poster and status; people are left out. */
    @Test
    fun readsSearch() {
        val answer =
            parseJson(
                """
                {"page": 1, "totalPages": 1, "results": [
                  {"id": 603, "mediaType": "movie", "title": "The Matrix", "releaseDate": "1999-03-30", "posterPath": "/m.jpg",
                   "mediaInfo": {"status": 5}},
                  {"id": 1399, "mediaType": "tv", "name": "Game of Thrones", "firstAirDate": "2011-04-17", "posterPath": null},
                  {"id": 6384, "mediaType": "person", "name": "Keanu Reeves"}
                ]}
                """.trimIndent(),
            )
        val d = RequestRules.discovered(answer)
        assertEquals(listOf("The Matrix", "Game of Thrones"), d.map { it.title })
        assertEquals(1999, d[0].year)
        assertEquals("https://image.tmdb.org/t/p/w185/m.jpg", d[0].poster)
        assertEquals("Available" to "ok", RequestRules.mediaBadge(d[0].mediaStatus))
        assertEquals("", d[1].poster)
        assertEquals("" to "", RequestRules.mediaBadge(d[1].mediaStatus))
    }

    /** Seasons with what is already asked for; specials last and not ticked at first. */
    @Test
    fun readsSeasons() {
        val details =
            parseJson(
                """
                {"id": 1399, "seasons": [
                  {"seasonNumber": 0, "name": "Specials", "episodeCount": 3},
                  {"seasonNumber": 1, "name": "Season 1", "episodeCount": 10},
                  {"seasonNumber": 2, "name": "", "episodeCount": 10}
                ], "mediaInfo": {"status": 4, "seasons": [{"seasonNumber": 1, "status": 5}]}}
                """.trimIndent(),
            )
        val s = RequestRules.seasons(details)
        assertEquals(listOf(1, 2, 0), s.map { it.number })
        assertEquals("Season 2", s[1].name)
        assertFalse(s[0].open)
        assertTrue(s[1].open)
        assertEquals(setOf(2), RequestRules.defaultSeasons(s))
    }

    /** The request body: a movie by id, a series with its seasons in order. */
    @Test
    fun body() {
        val movie = Discovered("movie", 603, "The Matrix", 1999, "", 1)
        val tv = Discovered("tv", 1399, "Game of Thrones", 2011, "", 1)
        val m = RequestRules.body(movie, setOf(1))
        assertEquals(JsonPrimitive("movie"), m["mediaType"])
        assertEquals(JsonPrimitive(603), m["mediaId"])
        assertNull(m["seasons"])
        assertEquals(JsonArray(listOf(JsonPrimitive(1), JsonPrimitive(3))), RequestRules.body(tv, setOf(3, 1))["seasons"])
        assertTrue(RequestRules.canSend(movie, emptySet()))
        assertFalse(RequestRules.canSend(tv, emptySet()))
    }
}
