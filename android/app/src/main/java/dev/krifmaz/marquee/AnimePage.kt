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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.bugs.BugRules
import dev.krifmaz.kit.compose.core.str
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
import kotlinx.serialization.json.JsonObject
import dev.krifmaz.kit.compose.R as KitR

/** How long after the last key the studio filter is used, so a word isn't asked for at every letter. */
private const val STUDIO_WAIT_MS = 400L

/**
 * The Anime tab, its own section next to the library's Movies and TV: the anime library (each season its own
 * entry, with AniList's posters and a countdown to the next episode), AniList's Discover with Request (Dub only,
 * Dual audio or Sub only and the subtitle language), and the calendar and queue narrowed to anime.
 *
 * @param me who is looking (null while unknown); read-only accounts can't request.
 */
@Composable
fun AnimePage(app: App, me: Me?) {
    var view by rememberSaveable { mutableIntStateOf(0) }
    Column(Modifier.fillMaxSize()) {
        ChoiceRow(AnimeRules.VIEWS, view, onSelect = { view = it })
        when (view) {
            0 -> AnimeLibrary(app)
            1 -> AnimeDiscover(app, canAsk = me?.canRequest != false)
            2 -> CalendarPage(app, path = "/api/anime/calendar", title = "Anime calendar")
            else -> QueuePage(app, me?.admin, path = "/api/anime/activity")
        }
    }
}

/** Every season of every anime series, and anime films; the countdowns move on every minute. */
@Composable
private fun AnimeLibrary(app: App) {
    var round by remember { mutableIntStateOf(0) }
    val state = rememberLoaded(app, "/api/anime/library", round, parse = AnimeRules::library)
    var now by remember { mutableLongStateOf(System.currentTimeMillis() / 1000) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(60_000)
            now = System.currentTimeMillis() / 1000
        }
    }
    LoadNote(state) { round += 1 }
    val all = state.value
    when {
        all == null && state.error != null -> ErrorBox(state.error) { round += 1 }

        all == null -> Row(Modifier.fillMaxWidth().padding(Gutter), horizontalArrangement = Arrangement.Center) { Spinner(24.dp) }

        all.isEmpty() -> EmptyState(R.drawable.ic_tab_anime, "No anime yet", "Find some in Discover and request it; it shows here.")

        else ->
            LazyColumn(Modifier.fillMaxSize()) {
                items(all, key = { it.key }) { e ->
                    PosterRow(app, e.poster, e.title, AnimeRules.line(e, now), e.badge, e.badgeKind)
                    RowDivider(startIndent = 68.dp)
                }
                item { Spacer(Modifier.height(24.dp)) }
            }
    }
}

/** AniList's lists with a genre and studio filter; a tap opens the request dialog. */
@Composable
private fun AnimeDiscover(app: App, canAsk: Boolean) {
    var list by rememberSaveable { mutableIntStateOf(0) }
    var genre by rememberSaveable { mutableStateOf("") }
    var typed by rememberSaveable { mutableStateOf("") }
    var studio by rememberSaveable { mutableStateOf("") }
    LaunchedEffect(typed) {
        delay(STUDIO_WAIT_MS)
        studio = typed.trim()
    }
    var again by remember { mutableIntStateOf(0) }
    val genres = rememberLoaded(app, "/api/anime/genres", 0, parse = AnimeRules::genres)
    val path = AnimeRules.discoverPath(AnimeRules.LISTS[list].second, genre, studio)
    val state = rememberLoaded(app, path, again, keep = genre.isEmpty() && studio.isEmpty(), parse = AnimeRules::discover)
    var asking by remember { mutableStateOf<AnimePick?>(null) }

    ChoiceRow(AnimeRules.LISTS.map { it.first }, list, onSelect = { list = it })
    Row(Modifier.fillMaxWidth().padding(horizontal = Gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        KitField(typed, { typed = it }, Modifier.weight(1f), placeholder = "Studio, like MAPPA", leadingIcon = KitR.drawable.kit_ic_search)
        val entries = listOf(MenuEntry("Every genre", onClick = { genre = "" })) + genres.value.orEmpty().map { g -> MenuEntry(g, onClick = { genre = g }) }
        MoreButton(entries)
    }
    SectionHead(if (genre.isEmpty()) AnimeRules.LISTS[list].first else "${AnimeRules.LISTS[list].first} · $genre")
    val found = state.value
    when {
        found == null && state.error != null -> ErrorBox(state.error) { again += 1 }

        found == null -> Row(Modifier.fillMaxWidth().padding(Gutter), horizontalArrangement = Arrangement.Center) { Spinner(24.dp) }

        found.isEmpty() -> EmptyState(KitR.drawable.kit_ic_search, "Nothing here", "Try another list, genre or studio.")

        else ->
            LazyColumn(Modifier.fillMaxSize()) {
                items(found, key = { it.anilistId }) { d ->
                    Column(Modifier.fillMaxWidth().clickable(enabled = canAsk) { asking = d }) {
                        PosterRow(app, d.poster, d.title, d.line, d.badge, d.badgeKind)
                    }
                    RowDivider(startIndent = 68.dp)
                }
                item { Spacer(Modifier.height(24.dp)) }
            }
    }
    asking?.let { d ->
        AnimeAskDialog(app, d, onDismiss = { asking = null }, onAsked = {
            asking = null
            again += 1
        })
    }
}

/** Asks for an AniList entry with the language choice; the account's default is picked first. */
@Composable
private fun AnimeAskDialog(app: App, pick: AnimePick, onDismiss: () -> Unit, onAsked: () -> Unit) {
    val p = MaterialTheme.palette
    val scope = rememberCoroutineScope()
    var audio by remember { mutableStateOf("dual") }
    var subs by remember { mutableStateOf("en") }
    var ready by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        try {
            val (a, s) = AnimeRules.defaults(withContext(Dispatchers.IO) { app.server.get("/api/v1/auth/me") })
            audio = a
            subs = s
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // the server's own default applies
        }
        ready = true
    }
    val open = AnimeRules.canRequest(pick)
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = p.panel,
        titleContentColor = p.text,
        textContentColor = p.muted,
        title = { Text(pick.title) },
        text = {
            Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                when {
                    pick.badge.isNotEmpty() -> Text("This is already ${pick.badge.lowercase()}.")

                    pick.tmdbId <= 0 -> Text("Marquee can't match this one to TMDB yet, so it can't be requested. The anime list is updated every day.")

                    !ready -> Spinner()

                    else -> {
                        Text("This asks for ${AnimeRules.what(pick)}.")
                        Text("Audio", color = p.text)
                        val picked = AnimeRules.AUDIO.indexOfFirst { it.second == audio }.coerceAtLeast(0)
                        ChoiceRow(AnimeRules.AUDIO.map { it.first }, picked, onSelect = { audio = AnimeRules.AUDIO[it].second })
                        val langs = AnimeRules.subtitleChoices(subs)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text("Subtitles: ${langs.first { it.second == subs }.first}", color = p.text, modifier = Modifier.weight(1f).padding(top = 12.dp))
                            MoreButton(langs.map { (name, code) -> MenuEntry(name, onClick = { subs = code }) })
                        }
                        Text("Your default is picked first. A change here is for this request only.")
                    }
                }
            }
        },
        confirmButton = {
            val ok = open && ready && !sending
            TextButton(
                enabled = ok,
                onClick = {
                    sending = true
                    scope.launch {
                        try {
                            val answer = withContext(Dispatchers.IO) { app.server.post("/api/v1/request", AnimeRules.body(pick, audio, subs)) }
                            val words = (answer as? JsonObject)?.str("statusWords").orEmpty()
                            Toasts.show(if (words.isEmpty()) "Asked for ${pick.title}" else "Asked for ${pick.title} · $words", "ok")
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
