package dev.krifmaz.marquee

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.KitApp
import dev.krifmaz.kit.compose.bugs.BugReportsHost
import dev.krifmaz.kit.compose.offline.OfflineBanner
import dev.krifmaz.kit.compose.offline.probeHealth
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.whatsnew.WhatsNewOnStart
import dev.krifmaz.kit.compose.widgets.RowDivider
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The tabs at the bottom of the main page: their names and icons. */
enum class Tab(
    /** The name under the icon. */
    val label: String,
    /** The icon. */
    val icon: Int,
) {
    /** What comes out this week ([CalendarPage]). */
    Calendar("Calendar", R.drawable.ic_tab_calendar),

    /** What is downloading ([QueuePage]). */
    Queue("Queue", R.drawable.ic_tab_queue),

    /** Every movie and series ([LibraryPage]). */
    Library("Library", R.drawable.ic_tab_library),

    /** Settings, What's new, bug reports, crashes and updates ([HomePage]). */
    More("More", R.drawable.ic_tab_more),
}

/**
 * The main page once signed in: the offline banner when the server can't be reached, What's new once after an
 * update, the open tab, and the tabs at the bottom. Each list keeps a saved copy, so the tabs still show something
 * offline.
 *
 * @param app the app (its server, settings and calls).
 * @param base the server's address.
 * @param bugs the bug report screens' host.
 * @param open opens another page.
 * @param onChangeServer forgets the server, back to [ConnectPage].
 */
@Composable
fun MainPage(app: App, base: String, bugs: BugReportsHost, open: (Page) -> Unit, onChangeServer: () -> Unit) {
    val context = LocalContext.current
    var tab by rememberSaveable { mutableIntStateOf(0) }
    // who is looking: null until the server says (or when it can't); admin actions show unless it says no
    var me by remember { mutableStateOf<Me?>(null) }
    // offline mode: one /api/health check at start and on every Retry
    var offline by remember { mutableStateOf(false) }
    var checking by remember { mutableStateOf(false) }
    var reason by remember { mutableStateOf("") }
    var round by remember { mutableIntStateOf(0) }
    LaunchedEffect(base, round) {
        checking = true
        val why = withContext(Dispatchers.IO) { probeHealth(app.server.http, base, phoneOnline(context), waitSeconds = 8) }
        offline = why != null
        reason = why.orEmpty()
        checking = false
        if (!offline) {
            try {
                me = withContext(Dispatchers.IO) { app.server.me() }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // the last answer stays (null: admin actions show, and the server says no there)
            }
        }
    }
    // What's new: once after an update, when the server says there is something new for this account
    WhatsNewOnStart(app.changelog, onFullChangelog = { open(Page.Changelog) })

    val p = MaterialTheme.palette
    Column(Modifier.fillMaxSize()) {
        OfflineBanner(
            active = offline,
            trying = checking,
            title = "Offline",
            line = "Can't reach the server · tap for details",
            message = "Offline – ${KitApp.name} can't reach its server at $base. The lists show the copies saved last time.",
            reason = reason,
            status = "",
            onRetry = { round += 1 },
        )
        Box(Modifier.weight(1f).fillMaxWidth()) {
            when (Tab.entries[tab]) {
                Tab.Calendar -> CalendarPage(app)
                Tab.Queue -> QueuePage(app, me?.admin)
                Tab.Library -> LibraryPage(app, me?.admin)
                Tab.More -> HomePage(app, base, me, bugs, open, onChangeServer)
            }
        }
        RowDivider(startIndent = 0.dp)
        NavigationBar(containerColor = p.bg2, windowInsets = WindowInsets(0)) {
            Tab.entries.forEachIndexed { i, t ->
                NavigationBarItem(
                    selected = i == tab,
                    onClick = { tab = i },
                    icon = { Icon(painterResource(t.icon), contentDescription = null) },
                    label = { Text(t.label) },
                    colors =
                    NavigationBarItemDefaults.colors(
                        selectedIconColor = p.text,
                        selectedTextColor = p.text,
                        indicatorColor = p.panel2,
                        unselectedIconColor = p.muted,
                        unselectedTextColor = p.muted,
                    ),
                )
            }
        }
    }
}
