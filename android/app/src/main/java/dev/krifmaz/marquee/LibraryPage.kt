package dev.krifmaz.marquee

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.bugs.BugRules
import dev.krifmaz.kit.compose.core.objects
import dev.krifmaz.kit.compose.core.str
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.ChoiceRow
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.KitField
import dev.krifmaz.kit.compose.widgets.MenuEntry
import dev.krifmaz.kit.compose.widgets.MoreButton
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import dev.krifmaz.kit.compose.R as KitR

/**
 * The library: every movie (`GET /api/movies`) or series (`GET /api/series`) with whether it is downloaded,
 * missing or still to come, and a search over the titles. Admins can search for one now (`POST
 * /api/movies/{id}/search`, `POST /api/series/{id}/search`).
 *
 * @param admin whether the signed-in account may search (null while unknown: the server decides).
 */
@Composable
fun LibraryPage(app: App, admin: Boolean?) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var query by rememberSaveable { mutableStateOf("") }
    var round by remember { mutableIntStateOf(0) }
    val movies = tab == 0
    val state = if (movies) {
        rememberLoaded(app, "/api/movies", round) { LibraryRules.movies(it) }
    } else {
        rememberLoaded(app, "/api/series", round) { LibraryRules.series(it) }
    }
    val scope = rememberCoroutineScope()

    Column(Modifier.fillMaxSize()) {
        ChoiceRow(listOf("Movies", "TV"), tab, onSelect = { tab = it })
        KitField(
            query,
            { query = it },
            Modifier.fillMaxWidth().padding(horizontal = Gutter),
            placeholder = if (movies) "Search your movies" else "Search your series",
            leadingIcon = KitR.drawable.kit_ic_search,
        )
        LoadNote(state) { round += 1 }
        val all = state.value
        when {
            all == null && state.error != null -> ErrorBox(state.error) { round += 1 }

            all == null -> Row(Modifier.fillMaxWidth().padding(Gutter), horizontalArrangement = Arrangement.Center) { Spinner(24.dp) }

            all.isEmpty() ->
                EmptyState(
                    R.drawable.ic_tab_library,
                    if (movies) "No movies yet" else "No series yet",
                    "Add them in Marquee's web app, or request them; they show here.",
                )

            else -> {
                val shown = LibraryRules.filter(all, query)
                if (shown.isEmpty()) {
                    EmptyState(KitR.drawable.kit_ic_search, "Nothing matches \"${query.trim()}\"")
                } else {
                    LazyColumn(Modifier.fillMaxSize()) {
                        items(shown, key = { "${it.kind}-${it.id}" }) { item ->
                            val title = if (item.year > 0) "${item.title} (${item.year})" else item.title
                            PosterRow(app, item.poster, title, item.line, item.badge, item.badgeKind) {
                                if (admin != false) {
                                    MoreButton(listOf(MenuEntry("Search now", iconRes = KitR.drawable.kit_ic_search, onClick = { scope.launch { searchNow(app, item) } })))
                                }
                            }
                            RowDivider(startIndent = 68.dp)
                        }
                        item { Spacer(Modifier.height(24.dp)) }
                    }
                }
            }
        }
    }
}

/** Asks the server to search for [item] now, and says in a toast what it grabbed or why nothing. */
private suspend fun searchNow(app: App, item: LibraryItem) {
    val path = if (item.kind == "movie") "/api/movies/${item.id}/search" else "/api/series/${item.id}/search"
    Toasts.show("Searching for ${item.title}…")
    try {
        val answer = withContext(Dispatchers.IO) { app.server.post(path, JsonObject(emptyMap())) }
        Toasts.show(searchWords(answer), "ok")
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        Toasts.show("The search didn't work: ${BugRules.errorWords(e)}", "err")
    }
}

/** A search's answer in words: what was grabbed, else the server's reason. */
private fun searchWords(answer: JsonElement?): String {
    val o = answer as? JsonObject ?: return "Searched"
    val grabbed = when (val g = o["grabbed"]) {
        is JsonObject -> listOf(g)
        else -> g.objects()
    }
    return when {
        grabbed.size == 1 -> "Grabbed ${grabbed[0].str("title")}"
        grabbed.size > 1 -> "Grabbed ${grabbed.size} releases"
        o.str("reason").isNotEmpty() -> o.str("reason")
        else -> "Nothing suitable found"
    }
}
