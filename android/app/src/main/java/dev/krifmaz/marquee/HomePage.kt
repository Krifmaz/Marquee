package dev.krifmaz.marquee

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.KitApp
import dev.krifmaz.kit.compose.bugs.BugReportsEntry
import dev.krifmaz.kit.compose.bugs.BugReportsHost
import dev.krifmaz.kit.compose.bugs.BugRules
import dev.krifmaz.kit.compose.bugs.CrashLog
import dev.krifmaz.kit.compose.bugs.CrashLogReport
import dev.krifmaz.kit.compose.bugs.NewBugReportSheet
import dev.krifmaz.kit.compose.bugs.saveExport
import dev.krifmaz.kit.compose.dialogs.ConfirmDialog
import dev.krifmaz.kit.compose.settings.Muted
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.update.UpdatesSection
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SectionHead
import dev.krifmaz.kit.compose.widgets.SmallButton
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import dev.krifmaz.kit.compose.R as KitR

/**
 * The More tab: the server, What's new, the admin pages, signing out, bug reports, the crash log and updates.
 *
 * @param app the app (its server, settings and calls).
 * @param base the server's address.
 * @param me who is signed in (null while unknown: the admin pages show, and the server says no there).
 * @param bugs the bug report screens' host.
 * @param open opens another page.
 * @param onChangeServer forgets the server, back to [ConnectPage].
 */
@Composable
fun HomePage(app: App, base: String, me: Me?, bugs: BugReportsHost, open: (Page) -> Unit, onChangeServer: () -> Unit) {
    val scope = rememberCoroutineScope()
    var askSignOut by remember { mutableStateOf(false) }
    var newReport by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        SectionHead(KitApp.name, subtitle = base.substringAfter("://"), action = "Change server", onAction = onChangeServer)

        SectionHead("Settings")
        EntryRow("What's new", "Every release's notes") { open(Page.Changelog) }
        if (me?.admin != false) {
            EntryRow("Server settings", "The server's settings, as the web app has them") { open(Page.ServerSettings) }
            EntryRow("Diagnostics", "Checks of the server and the phone app's crash reports") { open(Page.Diagnostics) }
        }
        EntryRow("Sign out", "Sign in again, as someone else or after a password change") { askSignOut = true }
        BugReportsEntry(bugs)
        Row(Modifier.padding(horizontal = Gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SmallButton("New report", iconRes = KitR.drawable.kit_ic_add) { newReport = true }
        }
        RowDivider(Modifier.padding(top = 12.dp))
        CrashReports(app)
        RowDivider()
        UpdatesSection()
        Box(Modifier.height(32.dp))
    }

    if (askSignOut) {
        ConfirmDialog(
            title = "Sign out?",
            text = "This phone forgets its sign-in and the lists it saved. You'll need your password to sign in again.",
            okLabel = "Sign out",
            danger = true,
            onDismiss = { askSignOut = false },
            onConfirm = { scope.launch { withContext(Dispatchers.IO) { app.server.logout() } } },
        )
    }
    if (newReport) {
        NewBugReportSheet(bugs, page = "More", onDismiss = { newReport = false })
    }
}

/** A row that opens a page: its name and a line about it (the look of the kit's Bug reports entry). */
@Composable
private fun EntryRow(title: String, line: String, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 10.dp)) {
        Text(title)
        Text(line, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
    }
}

/**
 * The crash reports this phone keeps until the server has them (admins read them under Diagnostics → App crashes),
 * Send crash log (a bug report of the kind Crash log) and Save as .txt (straight into Downloads, with Share on the
 * toast).
 */
@Composable
private fun CrashReports(app: App) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var sending by remember { mutableStateOf(false) }
    var version by remember { mutableIntStateOf(0) }
    // the count lists a folder: read off the main thread
    var waiting by remember { mutableIntStateOf(0) }
    LaunchedEffect(version) {
        if (version > 0) delay(2000)
        waiting = withContext(Dispatchers.IO) { CrashLog.pendingCount() }
    }
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text("App crashes", style = MaterialTheme.typography.titleMedium)
        Muted("If ${KitApp.name} closed by itself, send the crash log: it goes to the admin's Bug reports under Crash log.")
        Row(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SmallButton(if (sending) "Sending…" else "Send crash log", iconRes = KitR.drawable.kit_ic_bug, enabled = !sending) {
                sending = true
                scope.launch {
                    try {
                        withContext(Dispatchers.IO) { CrashLogReport.send(app, app.bugReports) }
                        Toasts.show("Crash log sent to the admin")
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Toasts.show(BugRules.errorWords(e), "err")
                    } finally {
                        sending = false
                    }
                }
            }
            SmallButton("Save as .txt", iconRes = KitR.drawable.kit_ic_download) {
                scope.launch {
                    try {
                        saveExport(context, withContext(Dispatchers.IO) { CrashLog.exportText(context) })
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Toasts.show("The crash log couldn't be saved: ${e.message ?: e.javaClass.simpleName}", "err")
                    }
                }
            }
        }
        if (waiting > 0) {
            Row(Modifier.padding(top = 6.dp)) {
                Muted("Waiting to send: $waiting", Modifier.weight(1f), color = MaterialTheme.palette.text)
                SmallButton("Send now", iconRes = KitR.drawable.kit_ic_refresh) {
                    CrashLog.uploadPending(app, app)
                    version += 1
                }
            }
        }
    }
}
