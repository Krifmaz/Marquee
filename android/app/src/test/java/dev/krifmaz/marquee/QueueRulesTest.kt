package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.parseJson
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The queue's rules: reading `GET /api/downloads` and the words for each download. */
class QueueRulesTest {
    private fun download(id: Int, status: String, extra: String = "") = """{"id": $id, "kind": "movie", "title": "Release $id", "quality": "WEBDL-1080p", "size": 4294967296,
            "left": 1073741824, "eta": 754, "status": "$status", "progress": 0.75, "message": "" $extra}"""

    /** Removed ones go; active ones come first, then the finished ones. */
    @Test
    fun activeFirst() {
        val answer = parseJson("[${download(1, "imported")}, ${download(2, "downloading")}, ${download(3, "removed")}, ${download(4, "failed")}]")
        val items = QueueRules.parse(answer)
        assertEquals(listOf(2L, 1L, 4L), items.map { it.id })
        assertTrue(items[0].active)
        assertFalse(items[1].active)
    }

    /** Only the last finished ones are kept. */
    @Test
    fun keepsRecentFinished() {
        val many = (1..30).joinToString(",") { download(it, "imported") }
        assertEquals(QueueRules.RECENT, QueueRules.parse(parseJson("[$many]")).size)
    }

    /** A null eta is unknown. */
    @Test
    fun unknownEta() {
        val one = QueueRules.parse(parseJson("[${download(1, "queued").replace("\"eta\": 754", "\"eta\": null")}]"))[0]
        assertNull(one.eta)
    }

    /** While downloading: the percentage, how much of how much and the time left. */
    @Test
    fun downloadingLine() {
        val d = QueueRules.parse(parseJson("[${download(1, "downloading")}]"))[0]
        assertEquals("75% · 3.0 GB of 4.0 GB · 12 min left", QueueRules.detailLine(d))
        assertTrue(QueueRules.showsProgress(d))
    }

    /** A failed one says why. */
    @Test
    fun failedLine() {
        val d = QueueRules.parse(parseJson("[${download(1, "failed").replace("\"message\": \"\"", "\"message\": \"Not enough space\"")}]"))[0]
        assertEquals("WEBDL-1080p · 4.0 GB · Not enough space", QueueRules.detailLine(d))
        assertEquals("Failed", QueueRules.statusWords("failed"))
        assertEquals("err", QueueRules.badgeKind("failed"))
        assertFalse(QueueRules.showsProgress(d))
    }

    /** Time left in words. */
    @Test
    fun etaWords() {
        assertEquals("under a minute left", QueueRules.etaWords(30))
        assertEquals("5 min left", QueueRules.etaWords(300))
        assertEquals("2 h left", QueueRules.etaWords(7200))
        assertEquals("2 h 5 min left", QueueRules.etaWords(7500))
        assertEquals("1 day left", QueueRules.etaWords(90000))
        assertEquals("3 days left", QueueRules.etaWords(3 * 86400))
    }
}
