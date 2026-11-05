package dev.krifmaz.marquee

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.nio.file.Files

/** The saved copies of the lists (offline mode): kept, read back, forgotten, and their age in words. */
class AnswerCacheTest {
    /** A saved answer comes back with its time; clearing forgets it. */
    @Test
    fun savesAndClears() {
        val dir = Files.createTempDirectory("answers").toFile()
        val cache = AnswerCache(dir)
        assertNull(cache.read("/api/downloads"))
        cache.save("/api/downloads", "[1]", now = 1_700_000_000_000)
        cache.save("/api/downloads", "[1,2]", now = 1_700_000_001_000)
        val saved = cache.read("/api/downloads")!!
        assertEquals("[1,2]", saved.text)
        assertEquals(1_700_000_001_000, saved.at)
        cache.clear()
        assertNull(cache.read("/api/downloads"))
        dir.deleteRecursively()
    }

    /** Paths become plain file names. */
    @Test
    fun fileNames() {
        assertEquals("_api_calendar_start_2026-11-05_end_2026-11-11.json", AnswerCache.fileName("/api/calendar?start=2026-11-05&end=2026-11-11"))
    }

    /** How old a copy is. */
    @Test
    fun ageWords() {
        val now = 10_000_000_000L
        assertEquals("Saved copy from just now", AnswerCache.ageWords(now - 5_000, now))
        assertEquals("Saved copy from 5 min ago", AnswerCache.ageWords(now - 300_000, now))
        assertEquals("Saved copy from 3 h ago", AnswerCache.ageWords(now - 3 * 3_600_000, now))
        assertEquals("Saved copy from yesterday", AnswerCache.ageWords(now - 30 * 3_600_000L, now))
        assertEquals("Saved copy from 4 days ago", AnswerCache.ageWords(now - 4 * 86_400_000L, now))
    }
}
