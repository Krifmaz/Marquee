package dev.krifmaz.marquee

import java.io.File
import java.io.IOException

/** A saved answer: its text and when it was fetched (milliseconds since 1970). */
data class SavedAnswer(
    /** The JSON text the server sent. */
    val text: String,
    /** When it came. */
    val at: Long,
)

/**
 * The last answer of each list the app shows (calendar, queue, library), kept in [dir], so the pages still show
 * something when the server can't be reached (offline mode). Forgotten on sign-out and when the server changes,
 * since it belongs to one account on one server. Plain files, unit-tested.
 */
class AnswerCache(private val dir: File) {
    /** Keeps [text] as the answer of [path], fetched [now]. Never fails: a full disk only loses the copy. */
    fun save(path: String, text: String, now: Long = System.currentTimeMillis()) {
        try {
            dir.mkdirs()
            val f = file(path)
            val tmp = File(dir, f.name + ".tmp")
            tmp.writeText(text)
            if (!tmp.renameTo(f)) {
                f.writeText(text)
                tmp.delete()
            }
            f.setLastModified(now)
        } catch (_: IOException) {
            // the copy is only a help when offline
        } catch (_: SecurityException) {
            // same
        }
    }

    /** The saved answer of [path], or null when there is none. */
    fun read(path: String): SavedAnswer? {
        val f = file(path)
        return try {
            if (f.isFile) SavedAnswer(f.readText(), f.lastModified()) else null
        } catch (_: IOException) {
            null
        }
    }

    /** Forgets every saved answer. */
    fun clear() {
        dir.listFiles()?.forEach { it.delete() }
    }

    /** The file of [path]: its letters and digits, the rest as `_` ("/api/calendar?start=…" → "_api_calendar_start_….json"). */
    fun file(path: String): File = File(dir, fileName(path))

    /** Reading the copy's name and age. */
    companion object {
        /** The file name for [path]. */
        fun fileName(path: String): String = path.replace(Regex("[^A-Za-z0-9-]"), "_").take(120) + ".json"

        /** How old a copy is, in words: "Saved copy from just now", "… from 5 min ago", "… from 3 h ago", "… from 2 days ago". */
        fun ageWords(at: Long, now: Long = System.currentTimeMillis()): String {
            val s = ((now - at) / 1000).coerceAtLeast(0)
            val ago = when {
                s < 60 -> "just now"
                s < 3600 -> "${s / 60} min ago"
                s < 86400 -> "${s / 3600} h ago"
                s < 2 * 86400 -> "yesterday"
                else -> "${s / 86400} days ago"
            }
            return "Saved copy from $ago"
        }
    }
}
