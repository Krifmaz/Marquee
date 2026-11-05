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
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.bugs.BugRules
import dev.krifmaz.kit.compose.dialogs.ConfirmDialog
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.Badge
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.MenuEntry
import dev.krifmaz.kit.compose.widgets.MoreButton
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SectionHead
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import dev.krifmaz.kit.compose.R as KitR

/** How often the queue asks the server again while it shows, in seconds. */
private const val QUEUE_REFRESH_SECONDS = 5L

/**
 * The queue: what is downloading and what finished lately, from `GET /api/downloads`, asked again every few
 * seconds while the page shows. Admins can stop a download (`DELETE /api/downloads/{id}`), after a confirm.
 *
 * @param admin whether the signed-in account may stop downloads (null while unknown: the server decides).
 * @param path the list to show (the Anime tab's queue is `GET /api/anime/activity`).
 */
@Composable
fun QueuePage(app: App, admin: Boolean?, path: String = "/api/downloads") {
    var round by remember { mutableIntStateOf(0) }
    val state = rememberLoaded(app, path, round, everySeconds = QUEUE_REFRESH_SECONDS, parse = QueueRules::parse)
    var stopping by remember { mutableStateOf<QueueItem?>(null) }
    val scope = rememberCoroutineScope()

    Column(Modifier.fillMaxSize()) {
        val items = state.value
        val active = items?.count { it.active } ?: 0
        SectionHead(
            "Queue",
            subtitle = if (items == null) {
                null
            } else if (active == 0) {
                "Nothing downloading"
            } else {
                "$active on the way"
            },
        )
        LoadNote(state) { round += 1 }
        when {
            items == null && state.error != null -> ErrorBox(state.error) { round += 1 }

            items == null -> Row(Modifier.fillMaxWidth().padding(Gutter), horizontalArrangement = Arrangement.Center) { Spinner(24.dp) }

            items.isEmpty() -> EmptyState(R.drawable.ic_tab_queue, "Nothing in the queue", "Downloads show here from the moment Marquee grabs them until they are in the library.")

            else ->
                LazyColumn(Modifier.fillMaxSize()) {
                    items(items, key = { it.id }) { item ->
                        QueueRow(item, canStop = admin != false && item.active) { stopping = item }
                        RowDivider()
                    }
                    item { Spacer(Modifier.height(24.dp)) }
                }
        }
    }

    stopping?.let { item ->
        ConfirmDialog(
            title = "Stop this download?",
            text = "\"${item.title}\" is removed from the download client with what it downloaded so far. Marquee may grab another release later.",
            okLabel = "Stop download",
            danger = true,
            onDismiss = { stopping = null },
            onConfirm = {
                stopping = null
                scope.launch {
                    try {
                        withContext(Dispatchers.IO) { app.server.delete("/api/downloads/${item.id}") }
                        Toasts.show("Download stopped", "ok")
                        round += 1
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Toasts.show("The download couldn't be stopped: ${BugRules.errorWords(e)}", "err")
                    }
                }
            },
        )
    }
}

/** One download: its name, status badge, progress bar and the line about size and time left. */
@Composable
private fun QueueRow(item: QueueItem, canStop: Boolean, onStop: () -> Unit) {
    val p = MaterialTheme.palette
    Row(Modifier.fillMaxWidth().padding(start = Gutter, end = 4.dp, top = 10.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(item.title, style = MaterialTheme.typography.bodyMedium, color = p.text, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Badge(QueueRules.statusWords(item.status), QueueRules.badgeKind(item.status))
                Text(QueueRules.detailLine(item), style = MaterialTheme.typography.bodySmall, color = p.muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
            if (QueueRules.showsProgress(item)) {
                LinearProgressIndicator(
                    progress = { item.progress.toFloat() },
                    color = p.accent,
                    trackColor = p.border,
                    modifier = Modifier.fillMaxWidth().padding(top = 2.dp, end = 12.dp),
                )
            }
        }
        if (canStop) {
            MoreButton(listOf(MenuEntry("Stop download", iconRes = KitR.drawable.kit_ic_close, danger = true, onClick = onStop)))
        }
    }
}
