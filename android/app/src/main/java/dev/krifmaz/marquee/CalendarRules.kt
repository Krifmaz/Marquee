package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.bool
import dev.krifmaz.kit.compose.core.int
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.objects
import dev.krifmaz.kit.compose.core.str
import kotlinx.serialization.json.JsonElement
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeParseException
import java.util.Locale

/** One line of the calendar: a movie coming out, or an episode airing. */
data class CalendarEntry(
    /** `movie` or `episode`. */
    val kind: String,
    /** The day it comes out. */
    val date: LocalDate,
    /** The first line: the movie's or the series' title. */
    val title: String,
    /** The second line: how the movie comes out ("In cinemas · 2026"), or the episode ("S02E01 · Beef"). */
    val line: String,
    /** The poster's address ("" for none). */
    val poster: String,
    /** Whether it is downloaded. */
    val hasFile: Boolean,
    /** Whether Marquee looks for it. */
    val monitored: Boolean,
    /** The movie's id (movies) or the series' id (episodes). */
    val id: Long,
) {
    /** A key that is the same for the same entry in every answer (for the list). */
    val key: String get() = "$kind-$id-$date-$line"
}

/** A day of the calendar and what comes out on it. */
data class CalendarDay(
    /** The day. */
    val date: LocalDate,
    /** What comes out, in the server's order. */
    val entries: List<CalendarEntry>,
)

/** How the calendar reads `GET /api/calendar` and words its days. Plain Kotlin, unit-tested. */
object CalendarRules {
    /** The days one page of the calendar shows. */
    const val DAYS = 7L

    /** The entries of a `GET /api/calendar` answer; an entry without a readable day is left out. */
    fun parse(answer: JsonElement?): List<CalendarEntry> = answer.objects().mapNotNull { o ->
        val date = day(o.str("date")) ?: return@mapNotNull null
        val episode = o.str("kind") == "episode"
        CalendarEntry(
            kind = if (episode) "episode" else "movie",
            date = date,
            title = if (episode) o.str("series_title") else o.str("title"),
            line = if (episode) episodeLine(o.int("season"), o.int("episode"), o.str("title")) else movieLine(o.str("release"), o.int("year")),
            poster = o.str("poster"),
            hasFile = o.bool("has_file"),
            monitored = o.bool("monitored"),
            id = if (episode) o.long("series_id") else o.long("movie_id"),
        )
    }

    /** The entries grouped by day, earliest first. */
    fun byDay(entries: List<CalendarEntry>): List<CalendarDay> = entries.groupBy { it.date }.toSortedMap().map { (d, list) -> CalendarDay(d, list) }

    /** The first day of the page [offset] weeks from [today] (0: today's page). */
    fun pageStart(today: LocalDate, offset: Int): LocalDate = today.plusDays(DAYS * offset)

    /** The query of the page that starts on [start]: `?start=…&end=…`, both days included. */
    fun query(start: LocalDate): String = "?start=$start&end=${start.plusDays(DAYS - 1)}"

    /** "Today", "Tomorrow", "Yesterday", else the weekday and the day ("Thu 12 Nov"). */
    fun dayLabel(date: LocalDate, today: LocalDate, locale: Locale = Locale.getDefault()): String = when (date) {
        today -> "Today"
        today.plusDays(1) -> "Tomorrow"
        today.minusDays(1) -> "Yesterday"
        else -> date.format(DateTimeFormatter.ofPattern("EEE d MMM", locale))
    }

    /** The page's days as one line: "5 – 11 Nov", or "28 Nov – 4 Dec" across two months; the year when it isn't [today]'s. */
    fun rangeLabel(start: LocalDate, today: LocalDate, locale: Locale = Locale.getDefault()): String {
        val end = start.plusDays(DAYS - 1)
        val month = DateTimeFormatter.ofPattern("MMM", locale)
        val yearPart = if (end.year != today.year) " ${end.year}" else ""
        return if (start.month == end.month) {
            "${start.dayOfMonth} – ${end.dayOfMonth} ${end.format(month)}$yearPart"
        } else {
            "${start.dayOfMonth} ${start.format(month)} – ${end.dayOfMonth} ${end.format(month)}$yearPart"
        }
    }

    /** The entry's badge: its words and the kit's badge kind (`ok`, `info`, `warn` or "" for muted). */
    fun status(entry: CalendarEntry, today: LocalDate): Pair<String, String> = when {
        entry.hasFile -> "Downloaded" to "ok"
        entry.date.isAfter(today) -> "Coming" to "info"
        !entry.monitored -> "Not looked for" to ""
        else -> "Missing" to "warn"
    }

    /** "S02E01 · Beef" (just "S02E01" without a title). */
    fun episodeLine(season: Int, episode: Int, title: String): String {
        val code = "S%02dE%02d".format(Locale.US, season, episode)
        return if (title.isBlank()) code else "$code · $title"
    }

    /** How a movie comes out on its day, with its year: "In cinemas · 2026". */
    fun movieLine(release: String, year: Int): String {
        val how = when (release) {
            "cinemas" -> "In cinemas"
            "physical" -> "Out on disc"
            "digital" -> "Out online"
            else -> "Out"
        }
        return if (year > 0) "$how · $year" else how
    }

    /** A `YYYY-MM-DD` day (longer dates are cut to their day); null when it isn't one. */
    fun day(text: String): LocalDate? = try {
        if (text.length < 10) null else LocalDate.parse(text.substring(0, 10))
    } catch (_: DateTimeParseException) {
        null
    }
}
