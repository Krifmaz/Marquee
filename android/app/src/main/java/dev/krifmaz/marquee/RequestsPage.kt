package dev.krifmaz.marquee

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.bugs.BugRules
import dev.krifmaz.kit.compose.core.str
import dev.krifmaz.kit.compose.dialogs.ConfirmDialog
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.ChoiceRow
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.KitField
import dev.krifmaz.kit.compose.widgets.MenuEntry
import dev.krifmaz.kit.compose.widgets.MoreButton
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SectionHead
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import java.net.URLEncoder
import dev.krifmaz.kit.compose.R as KitR

/** How often the requests list asks the server again while it shows, in seconds. */
private const val REQUESTS_REFRESH_SECONDS = 30L

/** How long the search waits after the last key before it asks, in milliseconds. */
private const val SEARCH_WAIT_MS = 400L

/**
 * The Requests tab, on the Seerr API (`/api/v1`): the requests (yours, or everyone's for admins) with where each
 * one is, and Ask for something: Trending, or a search, then a movie or the seasons of a series to ask for.
 * Admins approve, decline and retry; anyone removes what they may.
 *
 * @param me who is signed in (null while unknown: the server decides).
 */
@Composable
fun RequestsPage(app: App, me: Me?) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var round by remember { mutableIntStateOf(0) }
    val canAsk = me?.canRequest != false
    Column(Modifier.fillMaxSize()) {
        if (canAsk) ChoiceRow(listOf("Requests", "Ask for something"), tab, onSelect = { tab = it })
        if (tab == 0 || !canAsk) {
            RequestList(app, me?.admin, round, onChanged = { round += 1 })
        } else {
            AskFor(app, onAsked = {
                round += 1
                tab = 0
            })
        }
    }
}

/** The requests, newest first, every 30 s; each with its menu of what the account may do. */
@Composable
private fun RequestList(app: App, admin: Boolean?, round: Int, onChanged: () -> Unit) {
    var again by remember { mutableIntStateOf(0) }
    val state = rememberLoaded(app, RequestRules.LIST_PATH, round + again, everySeconds = REQUESTS_REFRESH_SECONDS, parse = RequestRules::requests)
    val scope = rememberCoroutineScope()
    var removing by remember { mutableStateOf<RequestItem?>(null) }
    val act: (RequestItem, String, String, String) -> Unit = { item, method, path, done ->
        scope.launch {
            try {
                withContext(Dispatchers.IO) {
                    if (method == "DELETE") app.server.delete(path) else app.server.post(path, JsonObject(emptyMap()))
                }
                Toasts.show(done, "ok")
                onChanged()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Toasts.show("That didn't work for ${item.title}: ${BugRules.errorWords(e)}", "err")
            }
        }
    }
    LoadNote(state) { again += 1 }
    val items = state.value
    when {
        items == null && state.error != null -> ErrorBox(state.error) { again += 1 }

        items == null -> Row(Modifier.fillMaxWidth().padding(Gutter), horizontalArrangement = Arrangement.Center) { Spinner(24.dp) }

        items.isEmpty() -> EmptyState(R.drawable.ic_tab_requests, "No requests yet", "Ask for a movie or a series and follow it here until it's ready to watch.")

        else ->
            LazyColumn(Modifier.fillMaxSize()) {
                items(items, key = { it.id }) { item ->
                    val line = listOf(item.seasons, if (item.requestedBy.isNotEmpty()) "by ${item.requestedBy}" else "", item.message)
                        .filter { it.isNotEmpty() }
                        .joinToString(" · ")
                    PosterRow(app, item.poster, item.title, line, item.words, RequestRules.badgeKind(item)) {
                        val menu = buildList {
                            if (admin != false && item.waiting) {
                                add(MenuEntry("Approve", iconRes = KitR.drawable.kit_ic_check, onClick = { act(item, "POST", "/api/v1/request/${item.id}/approve", "Approved") }))
                                add(MenuEntry("Decline", iconRes = KitR.drawable.kit_ic_close, onClick = { act(item, "POST", "/api/v1/request/${item.id}/decline", "Declined") }))
                            }
                            if (admin != false && item.status == RequestRules.FAILED) {
                                add(
                                    MenuEntry("Try again", iconRes = KitR.drawable.kit_ic_refresh, onClick = {
                                        act(item, "POST", "/api/v1/request/${item.id}/retry", "Trying again")
                                    }),
                                )
                            }
                            if (item.canRemove) add(MenuEntry("Remove request", iconRes = KitR.drawable.kit_ic_delete, danger = true, onClick = { removing = item }))
                        }
                        if (menu.isNotEmpty()) MoreButton(menu)
                    }
                    RowDivider(startIndent = 68.dp)
                }
                item { Spacer(Modifier.height(24.dp)) }
            }
    }
    removing?.let { item ->
        ConfirmDialog(
            title = "Remove this request?",
            text = "The request for ${item.title} goes. What is already in the library stays.",
            okLabel = "Remove",
            danger = true,
            onDismiss = { removing = null },
            onConfirm = {
                removing = null
                act(item, "DELETE", "/api/v1/request/${item.id}", "Request removed")
            },
        )
    }
}

/** Trending, or what a search finds; a tap opens the request dialog. */
@Composable
private fun AskFor(app: App, onAsked: () -> Unit) {
    var typed by rememberSaveable { mutableStateOf("") }
    var query by rememberSaveable { mutableStateOf("") }
    LaunchedEffect(typed) {
        delay(SEARCH_WAIT_MS)
        query = typed.trim()
    }
    val path = if (query.isEmpty()) "/api/v1/discover/trending?page=1" else "/api/v1/search?query=${URLEncoder.encode(query, "UTF-8")}&page=1"
    var again by remember { mutableIntStateOf(0) }
    val state = rememberLoaded(app, path, again, keep = query.isEmpty(), parse = RequestRules::discovered)
    var asking by remember { mutableStateOf<Discovered?>(null) }

    KitField(typed, { typed = it }, Modifier.fillMaxWidth().padding(horizontal = Gutter), placeholder = "Search movies and series", leadingIcon = KitR.drawable.kit_ic_search)
    SectionHead(if (query.isEmpty()) "Trending" else "Found")
    val found = state.value
    when {
        found == null && state.error != null -> ErrorBox(state.error) { again += 1 }

        found == null -> Row(Modifier.fillMaxWidth().padding(Gutter), horizontalArrangement = Arrangement.Center) { Spinner(24.dp) }

        found.isEmpty() -> EmptyState(KitR.drawable.kit_ic_search, "Nothing found", "Try other words, or the original title.")

        else ->
            LazyColumn(Modifier.fillMaxSize()) {
                items(found, key = { "${it.kind}-${it.tmdbId}" }) { d ->
                    val (badge, kind) = RequestRules.mediaBadge(d.mediaStatus)
                    val what = if (d.kind == "movie") "Movie" else "Series"
                    Column(Modifier.fillMaxWidth().clickable { asking = d }) {
                        PosterRow(app, d.poster, if (d.year > 0) "${d.title} (${d.year})" else d.title, what, badge, kind)
                    }
                    RowDivider(startIndent = 68.dp)
                }
                item { Spacer(Modifier.height(24.dp)) }
            }
    }
    asking?.let { d ->
        AskDialog(app, d, onDismiss = { asking = null }, onAsked = {
            asking = null
            onAsked()
        })
    }
}

/**
 * Asks for a movie, or for the ticked seasons of a series (those not asked for yet are ticked, specials aside), with
 * `POST /api/v1/request`; the toast says what the server made of it (auto-approved, or waiting for an admin).
 */
@Composable
private fun AskDialog(app: App, item: Discovered, onDismiss: () -> Unit, onAsked: () -> Unit) {
    val p = MaterialTheme.palette
    val scope = rememberCoroutineScope()
    var seasons by remember { mutableStateOf<List<SeasonChoice>?>(if (item.kind == "movie") emptyList() else null) }
    var ticked by remember { mutableStateOf(setOf<Int>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var sending by remember { mutableStateOf(false) }
    if (item.kind == "tv") {
        LaunchedEffect(item.tmdbId) {
            try {
                val list = RequestRules.seasons(withContext(Dispatchers.IO) { app.server.get("/api/v1/tv/${item.tmdbId}") })
                seasons = list
                ticked = RequestRules.defaultSeasons(list)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = BugRules.errorWords(e)
            }
        }
    }
    val there = item.mediaStatus == RequestRules.AVAILABLE
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = p.panel,
        titleContentColor = p.text,
        textContentColor = p.muted,
        title = { Text(if (item.year > 0) "${item.title} (${item.year})" else item.title) },
        text = {
            Column(Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                val list = seasons
                when {
                    error != null -> Text("The seasons couldn't be loaded: $error")

                    there -> Text("This is already in the library.")

                    item.kind == "movie" -> Text("Ask for this movie? You'll see here when it's being downloaded and when it's ready.")

                    list == null -> Spinner()

                    list.none { it.open } -> Text("Every season is already asked for or in the library.")

                    else -> {
                        Text("Which seasons?")
                        for (s in list) {
                            Row(
                                Modifier.fillMaxWidth().clickable(enabled = s.open) { ticked = if (s.number in ticked) ticked - s.number else ticked + s.number },
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Checkbox(
                                    checked = s.number in ticked || !s.open,
                                    onCheckedChange = null,
                                    enabled = s.open,
                                    colors = CheckboxDefaults.colors(checkedColor = p.accent, checkmarkColor = p.onAccent, uncheckedColor = p.muted),
                                )
                                val note = if (s.open) "${s.episodes} episodes" else RequestRules.mediaBadge(s.status).first.ifEmpty { "Asked for" }
                                Text("${s.name} · $note", color = if (s.open) p.text else p.muted)
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            val ok = !there && error == null && !sending && seasons != null && RequestRules.canSend(item, ticked) && (item.kind == "movie" || seasons?.any { it.open } == true)
            TextButton(
                enabled = ok,
                onClick = {
                    sending = true
                    scope.launch {
                        try {
                            val answer = withContext(Dispatchers.IO) { app.server.post("/api/v1/request", RequestRules.body(item, ticked)) }
                            Toasts.show(askedWords(item, answer), "ok")
                            onAsked()
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            Toasts.show("The request didn't go through: ${BugRules.errorWords(e)}", "err")
                            sending = false
                        }
                    }
                },
            ) { Text(if (sending) "Asking…" else "Request", color = if (ok) p.text else p.faint) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel", color = p.muted) } },
    )
}

/** What the toast says after a request: the server's words for it ("Approved", "Waiting for approval"). */
private fun askedWords(item: Discovered, answer: JsonElement?): String {
    val words = (answer as? JsonObject)?.str("statusWords").orEmpty()
    return if (words.isEmpty()) "Asked for ${item.title}" else "Asked for ${item.title} · $words"
}
