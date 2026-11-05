package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.arr
import dev.krifmaz.kit.compose.core.bool
import dev.krifmaz.kit.compose.core.int
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.obj
import dev.krifmaz.kit.compose.core.objectsOf
import dev.krifmaz.kit.compose.core.str
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

/** One request, as `GET /api/v1/request` gives it (Seerr's shape with Marquee's words added). */
data class RequestItem(
    /** The request's id. */
    val id: Long,
    /** `movie` or `tv`. */
    val kind: String,
    /** The title, with the year when known. */
    val title: String,
    /** The poster's address ("" for none). */
    val poster: String,
    /** Seerr's request status: 1 waiting, 2 approved, 3 declined, 4 failed, 5 done. */
    val status: Int,
    /** Seerr's media status: 1 unknown, 2 waiting, 3 being looked for, 4 partly there, 5 there. */
    val mediaStatus: Int,
    /** The server's words for where it is ("Waiting for approval", "Downloading 42%", "Available"). */
    val words: String,
    /** Who asked. */
    val requestedBy: String,
    /** The seasons asked for ("" for a movie). */
    val seasons: String,
    /** Why it failed or was declined, when the server says. */
    val message: String,
    /** Whether the signed-in account may remove it. */
    val canRemove: Boolean,
) {
    /** Whether it waits for an admin's yes or no. */
    val waiting: Boolean get() = status == RequestRules.PENDING
}

/** A movie or series found by the search or on Trending, to ask for. */
data class Discovered(
    /** `movie` or `tv`. */
    val kind: String,
    /** TMDB's id. */
    val tmdbId: Long,
    /** The title. */
    val title: String,
    /** The year it came out (0 when unknown). */
    val year: Int,
    /** The poster's address ("" for none). */
    val poster: String,
    /** Seerr's media status (1 when nobody asked for it). */
    val mediaStatus: Int,
)

/** A season of a series: its number, name and whether it can still be asked for. */
data class SeasonChoice(
    /** The season's number (0 is specials). */
    val number: Int,
    /** Its name ("Season 2"). */
    val name: String,
    /** How many episodes it has. */
    val episodes: Int,
    /** Seerr's media status of this season (1 when nobody asked for it). */
    val status: Int,
) {
    /** Whether it can be asked for (not asked for and not there). */
    val open: Boolean get() = status <= RequestRules.UNKNOWN
}

/** How the Requests tab reads the Seerr API and words what it shows. Plain Kotlin, unit-tested. */
object RequestRules {
    /** Request status: waiting for approval. */
    const val PENDING = 1

    /** Request status: declined. */
    const val DECLINED = 3

    /** Request status: approving it failed. */
    const val FAILED = 4

    /** Media status: nobody asked for it. */
    const val UNKNOWN = 1

    /** Media status: there. */
    const val AVAILABLE = 5

    /** TMDB's pictures, the size the list shows. */
    const val POSTER_BASE = "https://image.tmdb.org/t/p/w185"

    /** The requests list's path: the newest 50 of everyone's (admins) or your own. */
    const val LIST_PATH = "/api/v1/request?take=50&skip=0&sort=added&filter=all"

    /** The requests of a `GET /api/v1/request` answer. */
    fun requests(answer: JsonElement?): List<RequestItem> = ((answer as? JsonObject)?.arr("results")).objectsOf().map { o ->
        val year = o.int("year")
        val title = o.str("title").ifEmpty { "Untitled" }
        val seasons = o.arr("seasons").objectsOf().map { it.int("seasonNumber") }
        RequestItem(
            id = o.long("id"),
            kind = o.str("type"),
            title = if (year > 0) "$title ($year)" else title,
            poster = o.str("posterUrl"),
            status = o.int("status"),
            mediaStatus = o.obj("media")?.int("status") ?: UNKNOWN,
            words = o.str("statusWords").ifEmpty { statusWords(o.int("status")) },
            requestedBy = o.obj("requestedBy")?.let { u -> u.str("displayName").ifEmpty { u.str("username") } }.orEmpty(),
            seasons = seasonWords(seasons),
            message = o.str("message"),
            canRemove = o.bool("canRemove"),
        )
    }

    /** The request status in words, when the server sends none. */
    fun statusWords(status: Int): String = when (status) {
        PENDING -> "Waiting for approval"
        2 -> "Approved"
        DECLINED -> "Declined"
        FAILED -> "Didn't work"
        5 -> "Available"
        else -> "Requested"
    }

    /** The kit's badge kind for a request. */
    fun badgeKind(item: RequestItem): String = when {
        item.mediaStatus == AVAILABLE || item.status == 5 -> "ok"
        item.status == DECLINED || item.status == FAILED -> "err"
        item.waiting -> "warn"
        else -> "info"
    }

    /** "Season 1", "Seasons 1, 2 and 4", "" for none. */
    fun seasonWords(seasons: List<Int>): String {
        val s = seasons.sorted()
        return when (s.size) {
            0 -> ""
            1 -> "Season ${s[0]}"
            else -> "Seasons " + s.dropLast(1).joinToString(", ") + " and " + s.last()
        }
    }

    /** The movies and series of a search or Trending answer (people are left out). */
    fun discovered(answer: JsonElement?): List<Discovered> = ((answer as? JsonObject)?.arr("results")).objectsOf().mapNotNull { o ->
        val kind = o.str("mediaType")
        if (kind != "movie" && kind != "tv") return@mapNotNull null
        val title = if (kind == "movie") o.str("title") else o.str("name")
        val date = if (kind == "movie") o.str("releaseDate") else o.str("firstAirDate")
        Discovered(
            kind = kind,
            tmdbId = o.long("id"),
            title = title,
            year = date.take(4).toIntOrNull() ?: 0,
            poster = posterUrl(o.str("posterPath")),
            mediaStatus = o.obj("mediaInfo")?.int("status") ?: UNKNOWN,
        )
    }

    /** A TMDB poster path's full address ("" for none). */
    fun posterUrl(path: String): String = when {
        path.isEmpty() -> ""
        path.startsWith("http") -> path
        else -> POSTER_BASE + path
    }

    /** A found title's badge: Available, Partly there, Requested, Waiting for approval, or none. */
    fun mediaBadge(status: Int): Pair<String, String> = when (status) {
        AVAILABLE -> "Available" to "ok"
        4 -> "Partly there" to "info"
        3 -> "Requested" to "info"
        2 -> "Waiting for approval" to "warn"
        else -> "" to ""
    }

    /** The seasons of a `GET /api/v1/tv/{id}` answer, with what is already asked for or there; specials go last. */
    fun seasons(details: JsonElement?): List<SeasonChoice> {
        val o = details as? JsonObject ?: return emptyList()
        val known = o.obj("mediaInfo")?.arr("seasons").objectsOf().associate { it.int("seasonNumber") to it.int("status") }
        return o.arr("seasons").objectsOf().map { s ->
            val n = s.int("seasonNumber")
            SeasonChoice(n, s.str("name").ifEmpty { if (n == 0) "Specials" else "Season $n" }, s.int("episodeCount"), known[n] ?: UNKNOWN)
        }.sortedBy { if (it.number == 0) Int.MAX_VALUE else it.number }
    }

    /** The seasons ticked when the sheet opens: every open one but the specials. */
    fun defaultSeasons(seasons: List<SeasonChoice>): Set<Int> = seasons.filter { it.open && it.number > 0 }.map { it.number }.toSet()

    /** The body of `POST /api/v1/request`: the title, and for a series the seasons asked for. */
    fun body(item: Discovered, seasons: Set<Int>): JsonObject = buildJsonObject {
        put("mediaType", item.kind)
        put("mediaId", item.tmdbId)
        if (item.kind == "tv") putJsonArray("seasons") { seasons.sorted().forEach { add(it) } }
    }

    /** Whether the request can be sent: a movie always, a series with at least one season. */
    fun canSend(item: Discovered, seasons: Set<Int>): Boolean = item.kind == "movie" || seasons.isNotEmpty()
}
