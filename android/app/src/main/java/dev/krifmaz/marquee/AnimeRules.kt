package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.arr
import dev.krifmaz.kit.compose.core.bool
import dev.krifmaz.kit.compose.core.int
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.obj
import dev.krifmaz.kit.compose.core.objects
import dev.krifmaz.kit.compose.core.objectsOf
import dev.krifmaz.kit.compose.core.str
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

/** One entry of the anime library: a season of a series, or a film. */
data class AnimeEntry(
    /** The server's key for it (unique in the list). */
    val key: String,
    /** The title, AniList's when it knows it. */
    val title: String,
    /** The poster's address ("" for none). */
    val poster: String,
    /** The next episode's number, 0 when none is due. */
    val nextEpisode: Int,
    /** When the next episode airs, Unix seconds (0 when none is due). */
    val nextAirs: Long,
    /** The line under the title when no episode is due ("4 of 12 episodes", "Film"). */
    val line: String,
    /** The badge: its words. */
    val badge: String,
    /** The badge: the kit's kind. */
    val badgeKind: String,
)

/** One AniList entry of Discover, with what a request for it needs. */
data class AnimePick(
    /** AniList's id. */
    val anilistId: Long,
    /** The title. */
    val title: String,
    /** The poster's address. */
    val poster: String,
    /** "Fall 2026 · Airing". */
    val line: String,
    /** `tv` or `movie`. */
    val mediaType: String,
    /** TMDB's id, 0 when the mapping list doesn't know it yet. */
    val tmdbId: Long,
    /** TheTVDB's id, 0 when unknown. */
    val tvdbId: Long,
    /** The TheTVDB season this entry is, 0 when unknown (then every season is asked for). */
    val season: Int,
    /** Where it stands ("In the library", "Requested"), "" when nobody asked. */
    val badge: String,
    /** The badge's kind. */
    val badgeKind: String,
)

/** How the Anime tab reads the `/api/anime` calls and writes anime requests. Plain Kotlin, unit-tested. */
object AnimeRules {
    /** The views of the tab. */
    val VIEWS = listOf("Library", "Discover", "Calendar", "Queue")

    /** Discover's lists: (name, `list=` word). */
    val LISTS = listOf("Trending" to "trending", "This season" to "season", "Upcoming" to "upcoming", "Top rated" to "top")

    /** The audio choices: (name, word). */
    val AUDIO = listOf("Dub only" to "dub", "Dual audio" to "dual", "Sub only" to "sub")

    /** The subtitle languages a request can pick: (name, ISO 639-1). */
    val SUBTITLES =
        listOf(
            "English" to "en", "Spanish" to "es", "Portuguese" to "pt", "French" to "fr", "German" to "de", "Italian" to "it",
            "Russian" to "ru", "Arabic" to "ar", "Polish" to "pl", "Dutch" to "nl", "Turkish" to "tr", "Indonesian" to "id",
            "Chinese" to "zh", "Korean" to "ko", "Japanese" to "ja",
        )

    /** The entries of `GET /api/anime/library`, in the server's order. */
    fun library(answer: JsonElement?): List<AnimeEntry> = answer.objects().map { o ->
        val next = o.obj("next_episode")
        val film = o.str("kind") == "movie"
        val (badge, kind) = badge(film, o.bool("monitored"), o.str("status"), o.int("files"), o.int("episodes"))
        AnimeEntry(
            key = o.str("key"),
            title = o.str("title"),
            poster = o.str("poster"),
            nextEpisode = next?.int("episode") ?: 0,
            nextAirs = next?.long("airs") ?: 0,
            line = if (film) "Film" else "${o.int("files")} of ${o.int("episodes")} episodes",
            badge = badge,
            badgeKind = kind,
        )
    }

    /** A library entry's badge: Not looking, Downloaded or Missing for a film, else Airing, Not yet aired or Complete. */
    fun badge(film: Boolean, monitored: Boolean, status: String, files: Int, episodes: Int): Pair<String, String> = when {
        !monitored -> "Not looking" to ""
        film -> if (files > 0) "Downloaded" to "ok" else "Missing" to "warn"
        status == "Airing" || status == "Not yet aired" -> status to "info"
        episodes in 1..files -> "Complete" to "ok"
        else -> status to ""
    }

    /** The line under a library entry at [nowSeconds]: the next episode's countdown, else [AnimeEntry.line]. */
    fun line(e: AnimeEntry, nowSeconds: Long): String = if (e.nextAirs > 0) "Episode ${e.nextEpisode} ${countdown(e.nextAirs - nowSeconds)}" else e.line

    /** The time until an episode airs in words: "in 2 days 4 hours", "in 3 hours 5 minutes", "in 12 minutes", "now". */
    fun countdown(seconds: Long): String {
        if (seconds <= 60) return "now"
        val d = seconds / 86_400
        val h = seconds % 86_400 / 3600
        val m = seconds % 3600 / 60
        fun unit(n: Long, one: String) = if (n == 1L) "1 $one" else "$n ${one}s"
        return when {
            d > 0 && h > 0 -> "in ${unit(d, "day")} ${unit(h, "hour")}"
            d > 0 -> "in ${unit(d, "day")}"
            h > 0 -> "in ${unit(h, "hour")} ${unit(m, "minute")}"
            else -> "in ${unit(m, "minute")}"
        }
    }

    /** The entries of `GET /api/anime/discover`. */
    fun discover(answer: JsonElement?): List<AnimePick> = ((answer as? JsonObject)?.arr("results")).objectsOf().map { o ->
        val status = o.int("media_status")
        val (badge, kind) = when {
            status > 1 -> RequestRules.mediaBadge(status)
            o.long("series_id") > 0 || o.long("movie_id") > 0 -> "In the library" to "ok"
            else -> "" to ""
        }
        AnimePick(
            anilistId = o.long("anilist_id"),
            title = o.str("title"),
            poster = o.str("poster"),
            line = listOf(o.str("season"), o.str("status")).filter { it.isNotEmpty() }.joinToString(" · "),
            mediaType = o.str("media_type").ifEmpty { "tv" },
            tmdbId = o.long("tmdb_id"),
            tvdbId = o.long("tvdb_id"),
            season = o.int("tvdb_season"),
            badge = badge,
            badgeKind = kind,
        )
    }

    /** Whether Discover's [pick] can be requested: nobody asked yet, and the mapping list knows its TMDB id. */
    fun canRequest(pick: AnimePick): Boolean = pick.badge.isEmpty() && pick.tmdbId > 0

    /** What the request asks for, in words: "season 2", "every season" or "the film". */
    fun what(pick: AnimePick): String = when {
        pick.mediaType == "movie" -> "the film"
        pick.season > 0 -> "season ${pick.season}"
        else -> "every season"
    }

    /** The body of `POST /api/v1/request` for [pick] with the language choice. */
    fun body(pick: AnimePick, audio: String, subtitles: String): JsonObject = buildJsonObject {
        put("mediaType", pick.mediaType)
        put("mediaId", pick.tmdbId)
        put("animeAudio", audio)
        put("animeSubtitles", subtitles)
        if (pick.mediaType == "tv") {
            if (pick.tvdbId > 0) put("tvdbId", pick.tvdbId)
            if (pick.season > 0) putJsonArray("seasons") { add(pick.season) } else put("seasons", "all")
        }
    }

    /** The account's default (audio, subtitles) from `GET /api/v1/auth/me`: dual audio with English subtitles when unset. */
    fun defaults(me: JsonElement?): Pair<String, String> {
        val o = me as? JsonObject
        val audio = o?.str("animeAudio").orEmpty().takeIf { a -> AUDIO.any { it.second == a } } ?: "dual"
        val subs = o?.str("animeSubtitles").orEmpty().ifEmpty { "en" }
        return audio to subs
    }

    /** The subtitle choices with [current] in them (it goes first when it isn't one of [SUBTITLES]). */
    fun subtitleChoices(current: String): List<Pair<String, String>> = if (SUBTITLES.any { it.second == current }) SUBTITLES else listOf(current.uppercase() to current) + SUBTITLES

    /** Discover's address for a list, genre and studio. */
    fun discoverPath(list: String, genre: String, studio: String): String {
        fun enc(s: String) = java.net.URLEncoder.encode(s.trim(), "UTF-8")
        return "/api/anime/discover?list=$list&genre=${enc(genre)}&studio=${enc(studio)}&page=1"
    }

    /** AniList's genres from `GET /api/anime/genres`. */
    fun genres(answer: JsonElement?): List<String> = (answer as? kotlinx.serialization.json.JsonArray).orEmpty().mapNotNull {
        (it as? kotlinx.serialization.json.JsonPrimitive)?.content
    }
}
