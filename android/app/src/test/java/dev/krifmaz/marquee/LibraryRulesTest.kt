package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.parseJson
import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.LocalDate
import java.util.Locale

/** The library's rules: reading the movie and series lists, their badges and the title search. */
class LibraryRulesTest {
    /** Movies by title (without "The"), with their badge and when they come out. */
    @Test
    fun movies() {
        val answer =
            parseJson(
                """
                [
                  {"id": 1, "title": "The Matrix", "year": 1999, "poster": "p", "status": "released", "available": true,
                   "has_file": true, "monitored": true, "certification": "R"},
                  {"id": 2, "title": "Dune", "year": 2026, "status": "announced", "available": false, "has_file": false,
                   "monitored": true, "digital_release": "2026-12-20", "physical_release": "2027-01-10"}
                ]
                """.trimIndent(),
            )
        val m = LibraryRules.movies(answer, Locale.UK)
        assertEquals(listOf("Dune", "The Matrix"), m.map { it.title })
        assertEquals("Out 20 Dec", m[0].line)
        assertEquals("Announced", m[0].badge)
        assertEquals("R", m[1].line)
        assertEquals("Downloaded" to "ok", m[1].badge to m[1].badgeKind)
    }

    /** Every movie badge. */
    @Test
    fun movieBadges() {
        assertEquals("Not looked for" to "", LibraryRules.movieBadge(hasFile = false, monitored = false, available = true, status = "released"))
        assertEquals("Missing" to "warn", LibraryRules.movieBadge(hasFile = false, monitored = true, available = true, status = "released"))
        assertEquals("In cinemas" to "info", LibraryRules.movieBadge(hasFile = false, monitored = true, available = false, status = "inCinemas"))
        assertEquals("Not announced" to "", LibraryRules.movieBadge(hasFile = false, monitored = true, available = false, status = "tba"))
    }

    /** Series: how many episodes are downloaded and what airs next. */
    @Test
    fun series() {
        val answer =
            parseJson(
                """
                [{"id": 5, "title": "The Bear", "year": 2022, "network": "FX", "status": "continuing", "monitored": true,
                  "aired_episodes": 4, "episode_files": 4, "next_airing": "2026-11-12"}]
                """.trimIndent(),
            )
        val s = LibraryRules.series(answer, Locale.UK)[0]
        assertEquals("4 of 4 episodes", s.line)
        assertEquals("Next 12 Nov", s.badge)
        val next = LocalDate.of(2026, 11, 12)
        assertEquals("Missing 2" to "warn", LibraryRules.seriesBadge(4, 2, true, "continuing", next, Locale.UK))
        assertEquals("Ended" to "ok", LibraryRules.seriesBadge(4, 4, true, "ended", null))
        assertEquals("Complete" to "ok", LibraryRules.seriesBadge(4, 4, true, "continuing", null))
        assertEquals("Not looked for" to "", LibraryRules.seriesBadge(4, 0, false, "continuing", null))
    }

    /** Every typed word must be in the title, whatever the case. */
    @Test
    fun search() {
        val items = listOf("The Matrix", "The Matrix Reloaded", "Dune").map { LibraryItem("movie", 0, it, 0, "", "", "", "") }
        assertEquals(3, LibraryRules.filter(items, "  ").size)
        assertEquals(listOf("The Matrix", "The Matrix Reloaded"), LibraryRules.filter(items, "matrix").map { it.title })
        assertEquals(listOf("The Matrix Reloaded"), LibraryRules.filter(items, "RELOADED matrix").map { it.title })
    }
}
