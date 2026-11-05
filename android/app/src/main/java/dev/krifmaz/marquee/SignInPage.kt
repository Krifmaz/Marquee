package dev.krifmaz.marquee

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.KitApp
import dev.krifmaz.kit.compose.bugs.CrashLog
import dev.krifmaz.kit.compose.settings.TextLink
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.KitButton
import dev.krifmaz.kit.compose.widgets.KitField
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import dev.krifmaz.kit.compose.R as KitR

/**
 * Signing in to the connected server, with the kit's sign-in rules: Username and
 * Password, then the authenticator code when the account has two-step sign-in. The server's answer (for example
 * "Invalid username or password") shows in full with Copy. The app token it gives is kept ([ServerApi.login]);
 * the app comes back here whenever the server stops taking it. When the server offers them
 * (`GET /api/public/sign-in-options`), a Jellyfin name and password, a Plex account (on plex.tv, in the browser)
 * and a reset link by mail work too.
 *
 * @param app the app (its server).
 * @param base the server's address, under the title.
 * @param onBack the back arrow: choose another server.
 */
@Composable
fun SignInPage(app: App, base: String, onBack: () -> Unit) {
    val p = MaterialTheme.palette
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var needsCode by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var options by remember { mutableStateOf(SignInOptions()) }
    // signing in with a Jellyfin name and password instead of a Marquee one
    var jellyfin by remember { mutableStateOf(false) }
    // the Plex sign-in on its way (the browser shows plex.tv), and asking for a reset link
    var plex by remember { mutableStateOf<PlexPin?>(null) }
    var forgot by remember { mutableStateOf(false) }
    LaunchedEffect(base) {
        options = try {
            AccountRules.options(withContext(Dispatchers.IO) { app.server.get("/api/public/sign-in-options") })
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            SignInOptions()
        }
    }
    // while plex.tv shows, ask every 2 seconds whether the person is done there
    LaunchedEffect(plex) {
        val pin = plex ?: return@LaunchedEffect
        val until = System.currentTimeMillis() + AccountRules.PLEX_GIVE_UP_MS
        try {
            while (System.currentTimeMillis() < until) {
                delay(AccountRules.PLEX_POLL_MS)
                when (val r = withContext(Dispatchers.IO) { app.server.plexFinish(pin.id) }) {
                    null -> continue

                    is LoginResult.Success -> {
                        CrashLog.uploadPending(app, app)
                        return@LaunchedEffect
                    }

                    is LoginResult.NeedsCode -> error = r.message

                    is LoginResult.Failed -> error = r.message
                }
                break
            }
            if (error.isEmpty()) error = AccountRules.PLEX_TIMED_OUT
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: SignInRules.FAILED
        }
        plex = null
    }

    fun startPlex() {
        if (busy) return
        busy = true
        error = ""
        scope.launch {
            try {
                val pin = withContext(Dispatchers.IO) { app.server.plexStart() }
                context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(pin.url)))
                plex = pin
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.message ?: SignInRules.FAILED
            } finally {
                busy = false
            }
        }
    }

    fun sendReset() {
        if (busy || username.isBlank()) return
        busy = true
        error = ""
        scope.launch {
            try {
                withContext(Dispatchers.IO) { app.server.forgotPassword(username) }
                Toasts.show(AccountRules.RESET_SENT, "ok")
                forgot = false
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.message ?: SignInRules.FAILED
            } finally {
                busy = false
            }
        }
    }

    fun submit() {
        if (busy || username.isBlank() || password.isEmpty() || (needsCode && code.length < 6)) return
        busy = true
        error = ""
        scope.launch {
            try {
                val call = {
                    if (jellyfin) app.server.loginJellyfin(username, password) else app.server.login(username, password, if (needsCode) code else null)
                }
                when (val r = withContext(Dispatchers.IO) { call() }) {
                    // the shell shows the app now; crash reports kept from before can go
                    is LoginResult.Success -> CrashLog.uploadPending(app, app)

                    is LoginResult.NeedsCode -> {
                        if (needsCode) error = r.message
                        needsCode = true
                    }

                    is LoginResult.Failed -> error = r.message
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.message ?: SignInRules.FAILED
            } finally {
                busy = false
            }
        }
    }

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = Gutter, vertical = 48.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(painterResource(KitR.drawable.kit_ic_arrow_back), contentDescription = "Back") }
            Column {
                Text(SignInRules.title(KitApp.name), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                Text(base, style = MaterialTheme.typography.bodySmall, color = p.muted)
            }
        }
        val pin = plex
        when {
            forgot -> {
                Text("Type your name or mail address. ${KitApp.name} mails a link to pick a new password.", color = p.muted)
                KitField(username, { username = it }, modifier = Modifier.fillMaxWidth(), label = "Name or mail address", imeAction = ImeAction.Done, onAction = ::sendReset)
                if (error.isNotEmpty()) ErrorLine(error)
                KitButton(
                    if (busy) "Sending…" else "Send the link",
                    modifier = Modifier.fillMaxWidth(),
                    primary = true,
                    enabled = !busy && username.isNotBlank(),
                    onClick = ::sendReset,
                )
                TextLink("Back to signing in") {
                    forgot = false
                    error = ""
                }
            }

            pin != null -> {
                Text("Sign in on plex.tv in the browser, then come back here. The code there is ${pin.code}.", color = p.muted)
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Spinner(20.dp)
                    Text("Waiting for Plex…")
                }
                TextLink("Open plex.tv again") { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(pin.url))) }
                TextLink("Stop") { plex = null }
            }

            else -> {
                if (jellyfin) Text("Use the name and password you sign in to Jellyfin with.", color = p.muted)
                KitField(username, { username = it }, modifier = Modifier.fillMaxWidth(), label = "Username", enabled = !needsCode, imeAction = ImeAction.Next)
                KitField(
                    password,
                    { password = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = "Password",
                    enabled = !needsCode,
                    password = true,
                    keyboardType = KeyboardType.Password,
                    imeAction = ImeAction.Done,
                    onAction = ::submit,
                )
                if (needsCode) {
                    Text(SignInRules.CODE_PROMPT, color = p.muted)
                    KitField(
                        code,
                        { v -> code = v.filter { it.isDigit() }.take(8) },
                        modifier = Modifier.fillMaxWidth(),
                        label = "Code",
                        keyboardType = KeyboardType.NumberPassword,
                        imeAction = ImeAction.Done,
                        onAction = ::submit,
                    )
                }
                if (error.isNotEmpty()) ErrorLine(error)
                KitButton(
                    if (busy) {
                        "Signing in…"
                    } else if (jellyfin) {
                        "Sign in with Jellyfin"
                    } else {
                        "Sign in"
                    },
                    modifier = Modifier.fillMaxWidth(),
                    primary = true,
                    enabled = !busy && username.isNotBlank() && password.isNotEmpty() && (!needsCode || code.length >= 6),
                    onClick = ::submit,
                )
                if (needsCode) {
                    TextLink("Use a different account") {
                        needsCode = false
                        code = ""
                        error = ""
                    }
                } else {
                    if (options.jellyfin) {
                        TextLink(if (jellyfin) "Use a ${KitApp.name} account instead" else "Use your Jellyfin account instead") {
                            jellyfin = !jellyfin
                            error = ""
                        }
                    }
                    if (options.reset && !jellyfin) {
                        TextLink("Forgot your password?") {
                            forgot = true
                            error = ""
                        }
                    }
                    if (options.plex) {
                        KitButton("Sign in with Plex", modifier = Modifier.fillMaxWidth().padding(top = 12.dp), enabled = !busy, onClick = ::startPlex)
                    }
                }
            }
        }
    }
}
