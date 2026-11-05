package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.dbl
import dev.krifmaz.kit.compose.core.formatSize
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.objects
import dev.krifmaz.kit.compose.core.str
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject

/** One download, as `GET /api/downloads` gives it. */
data class QueueItem(
    /** Marquee's id (for `DELETE /api/downloads/{id}`). */
    val id: Long,
    /** The release's name. */
    val title: String,
    /** `movie`, `tv` or `anime`. */
    val kind: String,
    /** `queued`, `downloading`, `paused`, `completed`, `importing`, `imported`, `importBlocked`, `failed` or `removed`. */
    val status: String,
    /** From 0 to 1. */
    val progress: Double,
    /** Bytes in all. */
    val size: Long,
    /** Bytes still to come. */
    val left: Long,
    /** Seconds still to go; null when nobody knows. */
    val eta: Long?,
    /** Its quality ("WEBDL-1080p"). */
    val quality: String,
    /** What happened last, in words (why it failed or can't be imported). */
    val message: String,
) {
    /** Whether it is still on its way (not imported, failed or removed). */
    val active: Boolean get() = status in QueueRules.ACTIVE
}

/** How the queue reads `GET /api/downloads` and words each download. Plain Kotlin, unit-tested. */
object QueueRules {
    /** The states of a download that is still on its way. */
    val ACTIVE: Set<String> = setOf("queued", "downloading", "paused", "completed", "importing", "importBlocked")

    /** How many finished downloads the queue shows under the active ones. */
    const val RECENT = 20

    /** The downloads of an answer, without removed ones: the active ones first, then the [RECENT] last finished. */
    fun parse(answer: JsonElement?): List<QueueItem> {
        val all = answer.objects().map(::item).filter { it.status != "removed" }
        return all.filter { it.active } + all.filterNot { it.active }.take(RECENT)
    }

    private fun item(o: JsonObject): QueueItem = QueueItem(
        id = o.long("id"),
        title = o.str("title"),
        kind = o.str("kind"),
        status = o.str("status"),
        progress = o.dbl("progress").coerceIn(0.0, 1.0),
        size = o.long("size"),
        left = o.long("left"),
        eta = o["eta"].takeUnless { it == null || it is JsonNull }?.let { o.long("eta") },
        quality = o.str("quality"),
        message = o.str("message"),
    )

    /** The status in words, for the badge. */
    fun statusWords(status: String): String = when (status) {
        "queued" -> "Waiting"
        "downloading" -> "Downloading"
        "paused" -> "Paused"
        "completed" -> "Downloaded"
        "importing" -> "Moving into the library"
        "importBlocked" -> "Can't be moved"
        "imported" -> "In the library"
        "failed" -> "Failed"
        else -> status.replaceFirstChar { it.uppercase() }
    }

    /** The kit's badge kind for a status. */
    fun badgeKind(status: String): String = when (status) {
        "downloading", "importing", "completed" -> "info"
        "imported" -> "ok"
        "importBlocked", "paused" -> "warn"
        "failed" -> "err"
        else -> ""
    }

    /** Time left in words: "under a minute left", "12 min left", "2 h 5 min left", "3 days left". */
    fun etaWords(seconds: Long): String = when {
        seconds < 60 -> "under a minute left"

        seconds < 3600 -> "${seconds / 60} min left"

        seconds < 86400 -> {
            val m = (seconds % 3600) / 60
            if (m == 0L) "${seconds / 3600} h left" else "${seconds / 3600} h $m min left"
        }

        else -> if (seconds / 86400 == 1L) "1 day left" else "${seconds / 86400} days left"
    }

    /** "42%". */
    fun percent(progress: Double): String = "${(progress * 100).toInt()}%"

    /**
     * The line under the title: while downloading the percentage, how much of how much and the time left
     * ("42% · 1.2 GB of 3.4 GB · 12 min left"); otherwise the quality and size, and what went wrong.
     */
    fun detailLine(item: QueueItem): String {
        val parts = mutableListOf<String>()
        if (item.status == "downloading" || item.status == "paused") {
            parts += percent(item.progress)
            if (item.size > 0) parts += "${formatSize(item.size - item.left)} of ${formatSize(item.size)}"
            if (item.status == "downloading" && item.eta != null) parts += etaWords(item.eta)
        } else {
            if (item.quality.isNotEmpty()) parts += item.quality
            if (item.size > 0) parts += formatSize(item.size)
        }
        if (item.message.isNotEmpty() && item.status in setOf("failed", "importBlocked")) parts += item.message
        return parts.joinToString(" · ")
    }

    /** Whether the progress bar shows (on its way and not yet done). */
    fun showsProgress(item: QueueItem): Boolean = item.status == "downloading" || item.status == "paused" || item.status == "queued"
}
