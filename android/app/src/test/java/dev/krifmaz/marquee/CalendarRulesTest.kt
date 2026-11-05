package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.parseJson
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.LocalDate
import java.util.Locale

/** The calendar's rules: reading `GET /api/calendar`, grouping by day and the words for days and entries. */
class CalendarRulesTest {
    private val today = LocalDate.of(2026, 11, 5)

    private val answer =
        parseJson(
            """
            [
              {"kind": "episode", "date": "2026-11-05", "series_id": 10, "episode_id": 100, "series_title": "The Bear",
               "series_kind": "tv", "season": 2, "episode": 1, "title": "Beef", "poster": "https://img/bear.jpg",
               "has_file": true, "monitored": true},
              {"kind": "movie", "date": "2026-11-06", "release": "digital", "movie_id": 2, "title": "Apple", "year": 2026,
               "poster": "", "has_file": false, "monitored": true},
              {"kind": "movie", "date": "2026-11-06", "release": "cinemas", "movie_id": 1, "title": "Zebra", "year": 0,
               "poster": "", "has_file": false, "monitored": false},
              {"kind": "movie", "date": "soon", "movie_id": 3, "title": "No day"}
            ]
            """.trimIndent(),
        )

    /** Each entry's lines; one without a readable day is left out. */
    @Test
    fun readsEntries() {
        val e = CalendarRules.parse(answer)
        assertEquals(3, e.size)
        assertEquals("The Bear", e[0].title)
        assertEquals("S02E01 · Beef", e[0].line)
        assertEquals(10L, e[0].id)
        assertEquals("Out online · 2026", e[1].line)
        assertEquals("In cinemas", e[2].line)
        assertTrue(CalendarRules.parse(parseJson("{\"detail\": \"no\"}")).isEmpty())
    }

    /** Entries go under their day, earliest first. */
    @Test
    fun groupsByDay() {
        val days = CalendarRules.byDay(CalendarRules.parse(answer))
        assertEquals(listOf(today, today.plusDays(1)), days.map { it.date })
        assertEquals(listOf("Apple", "Zebra"), days[1].entries.map { it.title })
    }

    /** Downloaded, Coming, Not looked for and Missing. */
    @Test
    fun badges() {
        val e = CalendarRules.parse(answer)
        assertEquals("Downloaded" to "ok", CalendarRules.status(e[0], today))
        assertEquals("Coming" to "info", CalendarRules.status(e[1], today))
        assertEquals("Not looked for" to "", CalendarRules.status(e[2], today.plusDays(1)))
        assertEquals("Missing" to "warn", CalendarRules.status(e[1], today.plusDays(2)))
    }

    /** Pages are weeks from today; the query names both days. */
    @Test
    fun pages() {
        assertEquals(LocalDate.of(2026, 11, 12), CalendarRules.pageStart(today, 1))
        assertEquals(LocalDate.of(2026, 10, 29), CalendarRules.pageStart(today, -1))
        assertEquals("?start=2026-11-05&end=2026-11-11", CalendarRules.query(today))
    }

    /** Today, Tomorrow, Yesterday, else the weekday; ranges in one or two months, with the year when it isn't this one. */
    @Test
    fun dayWords() {
        val en = Locale.UK
        assertEquals("Today", CalendarRules.dayLabel(today, today, en))
        assertEquals("Tomorrow", CalendarRules.dayLabel(today.plusDays(1), today, en))
        assertEquals("Yesterday", CalendarRules.dayLabel(today.minusDays(1), today, en))
        assertEquals("Mon 9 Nov", CalendarRules.dayLabel(today.plusDays(4), today, en))
        assertEquals("5 – 11 Nov", CalendarRules.rangeLabel(today, today, en))
        assertEquals("26 Nov – 2 Dec", CalendarRules.rangeLabel(LocalDate.of(2026, 11, 26), today, en))
        assertEquals("29 Dec – 4 Jan 2027", CalendarRules.rangeLabel(LocalDate.of(2026, 12, 29), today, en))
    }
}
