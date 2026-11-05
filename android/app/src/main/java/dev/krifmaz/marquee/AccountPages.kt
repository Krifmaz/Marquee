package dev.krifmaz.marquee

import android.content.Intent
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.KitApp
import dev.krifmaz.kit.compose.bugs.BugRules
import dev.krifmaz.kit.compose.dialogs.ConfirmDialog
import dev.krifmaz.kit.compose.settings.Muted
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.BackBar
import dev.krifmaz.kit.compose.widgets.Badge
import dev.krifmaz.kit.compose.widgets.CardBox
import dev.krifmaz.kit.compose.widgets.ChoiceRow
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.KitButton
import dev.krifmaz.kit.compose.widgets.KitField
import dev.krifmaz.kit.compose.widgets.MenuEntry
import dev.krifmaz.kit.compose.widgets.MoreButton
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SectionHead
import dev.krifmaz.kit.compose.widgets.SmallButton
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import dev.krifmaz.kit.compose.R as KitR

/** Now, in seconds since 1970, as the server writes times. */
private fun nowSeconds(): Double = System.currentTimeMillis() / 1000.0

/**
 * The More tab's "Your account": the name and role, when the account ends, and its Jellyfin and Plex access
 * (`GET /api/users/{id}/media`, kept for offline mode).
 *
 * @param me who is signed in (nothing shows until the server said).
 */
@Composable
fun AccountSection(app: App, me: Me?) {
    if (me == null || me.id <= 0) return
    val p = MaterialTheme.palette
    var round by remember { mutableIntStateOf(0) }
    val state = rememberLoaded(app, "/api/users/${me.id}/media", round, parse = AccountRules::account)
    SectionHead("Your account", subtitle = me.username.ifEmpty { null })
    Column(Modifier.fillMaxWidth().padding(horizontal = Gutter), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(AccountRules.roleWords(me.role), color = p.text)
        val info = state.value
        when {
            info == null && state.error != null -> Muted("Your media access couldn't be read: ${state.error}")

            info == null -> Spinner(16.dp)

            else -> {
                Muted(AccountRules.expiryWords(info.expiresAt, nowSeconds()))
                if (info.media.isEmpty()) Muted("You have no Jellyfin or Plex access through ${KitApp.name}.")
                info.media.forEach { a ->
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                        val (words, kind) = AccountRules.accessBadge(a)
                        Text(a.server, color = p.text)
                        Badge(words, kind)
                    }
                    Muted(AccountRules.accessWords(a))
                }
            }
        }
    }
    LoadNote(state) { round += 1 }
}

/**
 * The invites page (admins): every invite link with its uses and end, a new invite (who for, how long, which media
 * servers), and per invite Share link (the phone's share sheet), Turn off or on, and Delete after a confirm.
 *
 * @param base the server's address, put in front of a link when the server has no public address.
 * @param onBack goes back to the More tab.
 */
@Composable
fun InvitesPage(app: App, base: String, onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var round by remember { mutableIntStateOf(0) }
    val state = rememberLoaded(app, "/api/invites", round, keep = false, parse = AccountRules::invites)
    var adding by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<Invite?>(null) }

    fun share(i: Invite) {
        val text = AccountRules.shareText(KitApp.name, AccountRules.fullLink(i.link, base))
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
        context.startActivity(Intent.createChooser(send, "Share the invite"))
    }

    Column(Modifier.fillMaxSize()) {
        BackBar("Invites", onBack)
        Row(Modifier.padding(horizontal = Gutter, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SmallButton("New invite", iconRes = KitR.drawable.kit_ic_add, enabled = !adding) { adding = true }
            SmallButton("Refresh", iconRes = KitR.drawable.kit_ic_refresh) { round += 1 }
        }
        if (adding) {
            NewInviteCard(app, onDone = { made ->
                adding = false
                if (made != null) {
                    round += 1
                    share(made)
                }
            })
        }
        val list = state.value
        when {
            list == null && state.error != null -> ErrorBox(state.error) { round += 1 }

            list == null -> Row(Modifier.fillMaxWidth().padding(Gutter), horizontalArrangement = Arrangement.Center) { Spinner(24.dp) }

            list.isEmpty() -> EmptyState(R.drawable.ic_tab_more, "No invites yet", "Make one, then share its link: the person picks a name and password and gets an account.")

            else ->
                LazyColumn(Modifier.fillMaxSize()) {
                    items(list, key = { it.code }) { i ->
                        InviteRow(
                            i,
                            onShare = { share(i) },
                            onToggle = {
                                scope.adminCall(if (i.disabled) "Invite turned on" else "Invite turned off", "The invite couldn't be changed", { round += 1 }) {
                                    app.server.patch("/api/invites/${i.code}", AccountRules.disableBody(!i.disabled))
                                }
                            },
                            onDelete = { deleting = i },
                        )
                        RowDivider()
                    }
                    item { Spacer(Modifier.height(24.dp)) }
                }
        }
    }

    deleting?.let { i ->
        ConfirmDialog(
            title = "Delete this invite?",
            text = "\"${AccountRules.title(i)}\" stops working. Accounts already made with it stay.",
            okLabel = "Delete invite",
            danger = true,
            onDismiss = { deleting = null },
            onConfirm = {
                deleting = null
                scope.adminCall("Invite deleted", "The invite couldn't be deleted", { round += 1 }) { app.server.delete("/api/invites/${i.code}") }
            },
        )
    }
}

/** One invite: its name, badge, the line about uses and end, and its menu. */
@Composable
private fun InviteRow(i: Invite, onShare: () -> Unit, onToggle: () -> Unit, onDelete: () -> Unit) {
    val p = MaterialTheme.palette
    val now = nowSeconds()
    Row(Modifier.fillMaxWidth().padding(start = Gutter, end = 4.dp, top = 10.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(AccountRules.title(i), color = p.text, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                val (words, kind) = AccountRules.badge(i, now)
                Badge(words, kind)
            }
            Text(AccountRules.line(i, now), style = MaterialTheme.typography.bodySmall, color = p.muted)
        }
        MoreButton(
            listOf(
                MenuEntry("Share link", iconRes = KitR.drawable.kit_ic_open_in_new, onClick = onShare).takeIf { i.usable },
                MenuEntry(if (i.disabled) "Turn on" else "Turn off", iconRes = if (i.disabled) KitR.drawable.kit_ic_check else KitR.drawable.kit_ic_close, onClick = onToggle),
                MenuEntry("Delete", iconRes = KitR.drawable.kit_ic_delete, danger = true, onClick = onDelete),
            ),
        )
    }
}

/** The new invite's choices: a note, who it is for, how long the link lasts and which media servers (as set up). */
@Composable
private fun NewInviteCard(app: App, onDone: (Invite?) -> Unit) {
    val scope = rememberCoroutineScope()
    var label by remember { mutableStateOf("") }
    var uses by remember { mutableIntStateOf(0) }
    var days by remember { mutableIntStateOf(1) }
    var servers by remember { mutableStateOf(listOf("Marquee only" to (false to false))) }
    var server by remember { mutableIntStateOf(0) }
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        try {
            val status: JsonElement? = withContext(Dispatchers.IO) { app.server.get("/api/media-servers") }
            servers = AccountRules.serverChoices(status)
            server = servers.lastIndex
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // Marquee only: the invite still works
        }
    }
    CardBox(Modifier.padding(horizontal = Gutter, vertical = 8.dp)) {
        Column(Modifier.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("New invite", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = Gutter))
            KitField(label, { label = it }, modifier = Modifier.fillMaxWidth().padding(horizontal = Gutter), label = "Note (only admins see it)")
            Muted("Who it's for", Modifier.padding(start = Gutter, top = 8.dp))
            ChoiceRow(AccountRules.USES.map { it.first }, uses, { uses = it })
            Muted("The link lasts", Modifier.padding(start = Gutter))
            ChoiceRow(AccountRules.DAYS.map { it.first }, days, { days = it })
            if (servers.size > 1) {
                Muted("New accounts also get (every library)", Modifier.padding(start = Gutter))
                ChoiceRow(servers.map { it.first }, server, { server = it })
            }
            Row(Modifier.padding(horizontal = Gutter, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                KitButton(if (busy) "Making…" else "Make and share", primary = true, enabled = !busy) {
                    busy = true
                    val (jf, px) = servers.getOrNull(server)?.second ?: (false to false)
                    val body = AccountRules.newInviteBody(NewInvite(label, AccountRules.USES[uses].second, AccountRules.DAYS[days].second, jf, px))
                    scope.launch {
                        try {
                            val made = withContext(Dispatchers.IO) { app.server.post("/api/invites", body) }
                            Toasts.show("Invite made", "ok")
                            onDone((made as? JsonObject)?.let(AccountRules::invite))
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            Toasts.show("The invite couldn't be made: ${BugRules.errorWords(e)}", "err")
                        } finally {
                            busy = false
                        }
                    }
                }
                KitButton("Cancel", enabled = !busy) { onDone(null) }
            }
        }
    }
}

/** Runs an admin [call] off the main thread; a toast says [ok] or "[failed]: why", and [after] runs when it worked. */
private fun CoroutineScope.adminCall(ok: String, failed: String, after: () -> Unit, call: () -> Unit) {
    launch {
        try {
            withContext(Dispatchers.IO) { call() }
            Toasts.show(ok, "ok")
            after()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Toasts.show("$failed: ${BugRules.errorWords(e)}", "err")
        }
    }
}
