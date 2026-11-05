package dev.krifmaz.marquee

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import dev.krifmaz.kit.compose.bugs.BugRules
import dev.krifmaz.kit.compose.core.parseJson
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement

/** What a list page has to show. */
data class Loaded<T>(
    /** The list: the server's latest answer or the saved copy; null before either came. */
    val value: T? = null,
    /** When the saved copy shown was fetched (ms); null while [value] is the server's latest answer. */
    val savedAt: Long? = null,
    /** Why the last try failed, in the kit's words; null when it worked. */
    val error: String? = null,
    /** Whether a call is on its way. */
    val loading: Boolean = true,
)

/**
 * GETs [path] from the server and keeps it as [Loaded]: the saved copy first (offline mode), then the server's
 * answer, kept for next time. Again every [everySeconds] while the page shows (null: once), and whenever [round]
 * changes (a Refresh). A failure keeps what is shown and says why.
 */
@Composable
fun <T> rememberLoaded(app: App, path: String, round: Int, everySeconds: Long? = null, parse: (JsonElement?) -> T): Loaded<T> {
    var state by remember(path) { mutableStateOf(Loaded<T>()) }
    LaunchedEffect(path, round) {
        if (state.value == null) {
            val saved = withContext(Dispatchers.IO) { app.settings.answers.read(path) }
            if (saved != null) state = Loaded(parse(parseJson(saved.text)), saved.at, null, loading = true)
        }
        while (true) {
            state = state.copy(loading = true)
            state = try {
                val text = withContext(Dispatchers.IO) { app.server.getText(path).also { app.settings.answers.save(path, it) } }
                Loaded(parse(parseJson(text)), null, null, loading = false)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state.copy(error = BugRules.errorWords(e), loading = false)
            }
            if (everySeconds == null) break
            delay(everySeconds * 1000)
        }
    }
    return state
}
