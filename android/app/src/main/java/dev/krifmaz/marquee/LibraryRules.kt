package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.bool
import dev.krifmaz.kit.compose.core.int
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.objects
import dev.krifmaz.kit.compose.core.str
import kotlinx.serialization.json.JsonElement
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.util.Locale

/** One movie or series of the library, as the list shows it. */
data class LibraryItem(
    /** `movie` or `series`. */
    val kind: String,
    /** Marquee's id. */
    val id: Long,
    /** The title. */
    val title: String,
    /** The year it came out (0 when unknown). */
    val year: Int,
    /** The poster's address ("" for none). */
    val poster: String,
    /** The line under the title ("2 of 8 episodes", "Out on disc 12 Dec"). */
    val line: String,
    /** The badge: its words. */
    val badge: String,
    /** The badge: the kit's kind (`ok`, `info`, `warn` or "" for muted). */
    val badgeKind: String,
)

/** How the library reads `GET /api/movies` and `GET /api/series`. Plain Kotlin, unit-tested. */
object LibraryRules {
    /** The movies of a `GET /api/movies` answer, by title. */
    fun movies(answer: JsonElement?, locale: Locale = Locale.getDefault()): List<LibraryItem> = answer.objects().map { o ->
        val (badge, kind) = movieBadge(o.bool("has_file"), o.bool("monitored"), o.bool("available"), o.str("status"))
        val next = listOf(o.str("digital_release"), o.str("physical_release")).mapNotNull(CalendarRules::day).minOrNull()
        val line = when {
            o.bool("has_file") || next == null -> o.str("certification")
            else -> "Out ${day(next, locale)}"
        }
        LibraryItem("movie", o.long("id"), o.str("title"), o.int("year"), o.str("poster"), line, badge, kind)
    }.sortedBy { sortKey(it.title) }

    /** The series of a `GET /api/series` answer, by title. */
    fun series(answer: JsonElement?, locale: Locale = Locale.getDefault()): List<LibraryItem> = answer.objects().map { o ->
        val aired = o.int("aired_episodes")
        val files = o.int("episode_files")
        val next = CalendarRules.day(o.str("next_airing"))
        val (badge, kind) = seriesBadge(aired, files, o.bool("monitored"), o.str("status"), next, locale)
        val line = if (aired > 0) "$files of $aired episodes" else o.str("network")
        LibraryItem("series", o.long("id"), o.str("title"), o.int("year"), o.str("poster"), line, badge, kind)
    }.sortedBy { sortKey(it.title) }

    /** A movie's badge: Downloaded, Missing (out and looked for), In cinemas, Announced, Not announced, or Not looked for. */
    fun movieBadge(hasFile: Boolean, monitored: Boolean, available: Boolean, status: String): Pair<String, String> = when {
        hasFile -> "Downloaded" to "ok"
        !monitored -> "Not looked for" to ""
        available -> "Missing" to "warn"
        status == "inCinemas" -> "In cinemas" to "info"
        status == "announced" -> "Announced" to "info"
        else -> "Not announced" to ""
    }

    /** A series' badge: the next episode's day, else Complete, Missing episodes, Ended, or Not looked for. */
    fun seriesBadge(aired: Int, files: Int, monitored: Boolean, status: String, next: LocalDate?, locale: Locale = Locale.getDefault()): Pair<String, String> = when {
        !monitored -> "Not looked for" to ""
        files < aired -> "Missing ${aired - files}" to "warn"
        next != null -> "Next ${day(next, locale)}" to "info"
        status == "ended" -> "Ended" to "ok"
        else -> "Complete" to "ok"
    }

    /** The items whose title has every word of [query] in it, whatever their case. */
    fun filter(items: List<LibraryItem>, query: String): List<LibraryItem> {
        val words = query.trim().lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (words.isEmpty()) return items
        return items.filter { item ->
            val title = item.title.lowercase()
            words.all { title.contains(it) }
        }
    }

    /** The title as it sorts: without a leading "The", "A" or "An". */
    fun sortKey(title: String): String = title.lowercase().removePrefix("the ").removePrefix("a ").removePrefix("an ")

    private fun day(d: LocalDate, locale: Locale): String = d.format(DateTimeFormatter.ofPattern("d MMM", locale))
}
