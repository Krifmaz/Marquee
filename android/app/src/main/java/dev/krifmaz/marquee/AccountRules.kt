package dev.krifmaz.marquee

import dev.krifmaz.kit.compose.core.bool
import dev.krifmaz.kit.compose.core.formatDay
import dev.krifmaz.kit.compose.core.int
import dev.krifmaz.kit.compose.core.obj
import dev.krifmaz.kit.compose.core.objectsOf
import dev.krifmaz.kit.compose.core.str
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonObject

/** Which sign-in buttons the server offers (`GET /api/public/sign-in-options`). */
data class SignInOptions(
    /** Signing in with a Jellyfin name and password. */
    val jellyfin: Boolean = false,
    /** Signing in with a Plex account, on plex.tv. */
    val plex: Boolean = false,
    /** A reset link by mail when the password is forgotten. */
    val reset: Boolean = false,
)

/** A Plex sign-in on its way (`POST /api/auth/plex/start`): open [url], then ask `finish` with [id]. */
data class PlexPin(
    /** The PIN's id, sent to `finish`. */
    val id: String,
    /** The code plex.tv shows. */
    val code: String,
    /** The plex.tv page to open in the browser. */
    val url: String,
)

/** The signed-in account's access on one media server, from `GET /api/users/{id}/media`. */
data class MediaAccess(
    /** "Jellyfin" or "Plex". */
    val server: String,
    /** Whether the access is on. */
    val active: Boolean,
    /** Whether a Jellyfin user or Plex share is linked to the account. */
    val linked: Boolean,
    /** The name on the media server ("" when unknown). */
    val remoteName: String,
    /** Why the last sync failed ("" when it worked). */
    val error: String,
    /** Whether the Jellyfin password changes to the Marquee one at the next sign-in. */
    val passwordPending: Boolean,
)

/** The signed-in account's media access and when the account ends. */
data class AccountInfo(
    /** When the account ends, in seconds since 1970; null when it doesn't. */
    val expiresAt: Double?,
    /** Jellyfin and Plex, the ones the account has. */
    val media: List<MediaAccess>,
)

/** An invite link, as admins see it (`GET /api/invites`). */
data class Invite(
    /** The invite's code. */
    val code: String,
    /** The link to send (`<public address>/#/invite/<code>`, or only the path when the server has no public address). */
    val link: String,
    /** The admins' note ("Family"). */
    val label: String,
    /** How many accounts it can make (0: no limit). */
    val maxUses: Int,
    /** How many it made. */
    val used: Int,
    /** When the link ends, in seconds since 1970; null when it doesn't. */
    val expiresAt: Double?,
    /** Days each new account lasts (0: for ever). */
    val accountDays: Int,
    /** The new accounts' role (`requester` or `viewer`). */
    val role: String,
    /** Whether new accounts get a Jellyfin user. */
    val jellyfin: Boolean,
    /** Whether new accounts get a Plex share. */
    val plex: Boolean,
    /** Whether an admin turned it off. */
    val disabled: Boolean,
    /** Whether it can still make an account. */
    val usable: Boolean,
    /** Why it can't, in the server's words ("" when it can). */
    val whyNot: String,
)

/** The choices of a new invite on the phone. */
data class NewInvite(
    /** The admins' note. */
    val label: String = "",
    /** How many accounts it can make (0: no limit). */
    val maxUses: Int = 1,
    /** Days until the link ends (0: never). */
    val days: Int = 7,
    /** A Jellyfin user for each new account (every library). */
    val jellyfin: Boolean = false,
    /** A Plex share for each new account (every library). */
    val plex: Boolean = false,
)

/** The account screens' rules: sign-in options, Plex, the account's access and invites. Plain Kotlin, unit-tested. */
object AccountRules {
    /** How often the phone asks whether the Plex sign-in is done, in milliseconds. */
    const val PLEX_POLL_MS = 2000L

    /** How long the phone keeps asking before it gives up, in milliseconds. */
    const val PLEX_GIVE_UP_MS = 5 * 60 * 1000L

    /** Shown when the Plex sign-in took too long. */
    const val PLEX_TIMED_OUT = "Plex didn't finish the sign-in in 5 minutes. Tap Sign in with Plex to try again."

    /** Shown after asking for a reset link, whatever the server knows (it doesn't say whether the account exists). */
    const val RESET_SENT = "If that account has a mail address, a link to pick a new password is on its way. It works for an hour."

    /** How many people a new invite is for: the choices and their numbers (0: no limit). */
    val USES = listOf("1 person" to 1, "5 people" to 5, "No limit" to 0)

    /** How long a new invite's link lasts: the choices and their days (0: never). */
    val DAYS = listOf("1 day" to 1, "7 days" to 7, "30 days" to 30, "Never ends" to 0)

    /** The answer of `GET /api/public/sign-in-options` (nothing extra when it can't be read). */
    fun options(answer: JsonElement?): SignInOptions {
        val o = answer as? JsonObject ?: return SignInOptions()
        return SignInOptions(o.bool("jellyfin"), o.bool("plex"), o.bool("reset"))
    }

    /** The answer of `POST /api/auth/plex/start`; null when it has no id or page to open. */
    fun plexPin(answer: JsonElement?): PlexPin? {
        val o = answer as? JsonObject ?: return null
        val pin = PlexPin(o.str("id"), o.str("code"), o.str("url"))
        return if (pin.id.isEmpty() || !pin.url.startsWith("https://")) null else pin
    }

    /** The body of `POST /api/auth/plex/finish`: the PIN and this phone, for an app token. */
    fun plexFinishBody(id: String, deviceId: String, deviceName: String): JsonObject = buildJsonObject {
        put("id", id)
        putJsonObject("app") {
            put("device_id", deviceId)
            put("device_name", deviceName)
        }
    }

    /** Whether a `finish` answer means the person is still signing in on plex.tv (202). */
    fun stillWaiting(status: Int): Boolean = status == 202

    /** The body of `POST /api/public/password/forgot`. */
    fun forgotBody(login: String): JsonObject = buildJsonObject { put("login", login.trim()) }

    /** The role in words, for the account section. */
    fun roleWords(role: String): String = when (role) {
        "owner" -> "Owner of this server"
        "admin" -> "Admin"
        "viewer" -> "Can watch, but not ask for titles"
        else -> "Can ask for movies and series"
    }

    /** The answer of `GET /api/users/{id}/media`. */
    fun account(answer: JsonElement?): AccountInfo {
        val o = answer as? JsonObject ?: return AccountInfo(null, emptyList())
        val media = listOf("jellyfin" to "Jellyfin", "plex" to "Plex").mapNotNull { (key, name) ->
            o.obj(key)?.let { s ->
                MediaAccess(name, s.bool("active"), s.bool("linked"), s.str("remote_name"), s.str("error"), s.bool("password_pending"))
            }
        }
        return AccountInfo(o.seconds("expires_at"), media)
    }

    /** One media server's access in words: on (and the name there), off, not linked yet, or the sync's problem. */
    fun accessWords(a: MediaAccess): String = when {
        a.error.isNotEmpty() -> "Not in step: ${a.error}"
        !a.active -> "Access is off. Ask an admin to turn it on."
        !a.linked -> "Being set up: it shows here once ${a.server} has it."
        a.passwordPending -> "On, as ${a.remoteName.ifEmpty { "you" }}. Your ${a.server} password becomes your Marquee one when you next sign in."
        a.remoteName.isNotEmpty() -> "On, as ${a.remoteName}"
        else -> "On"
    }

    /** The kit's badge kind for one media server's access. */
    fun accessBadge(a: MediaAccess): Pair<String, String> = when {
        a.error.isNotEmpty() -> "Problem" to "err"
        !a.active -> "Off" to "warn"
        !a.linked -> "Waiting" to "info"
        else -> "On" to "ok"
    }

    /** When the account ends, in words; [now] in seconds since 1970. */
    fun expiryWords(expiresAt: Double?, now: Double, day: (Double) -> String = ::formatDay): String = when {
        expiresAt == null -> "Your account doesn't end."
        expiresAt <= now -> "Your account ended on ${day(expiresAt)}. Ask an admin for more time."
        expiresAt - now < 7 * 86400 -> "Your account ends soon, on ${day(expiresAt)}. Ask an admin for more time."
        else -> "Your account ends on ${day(expiresAt)}."
    }

    /** The answer of `GET /api/invites`. */
    fun invites(answer: JsonElement?): List<Invite> = (answer as? JsonArray).objectsOf().map(::invite)

    /** One invite of an answer. */
    fun invite(o: JsonObject): Invite = Invite(
        code = o.str("code"),
        link = o.str("link"),
        label = o.str("label"),
        maxUses = o.int("max_uses"),
        used = o.int("used"),
        expiresAt = o.seconds("expires_at"),
        accountDays = o.int("account_days"),
        role = o.str("role"),
        jellyfin = o.obj("jellyfin") != null,
        plex = o.obj("plex") != null,
        disabled = o.bool("disabled"),
        usable = o.bool("usable"),
        whyNot = o.str("why_not"),
    )

    /** The link to share: the server's address goes in front when the server has no public address set. */
    fun fullLink(link: String, base: String): String = if (link.startsWith("/")) base.trimEnd('/') + link else link

    /** The text the share sheet sends. */
    fun shareText(app: String, link: String): String = "You're invited to $app. Open this link to make your account: $link"

    /** The invite's name in the list: its note, or its code. */
    fun title(i: Invite): String = i.label.ifEmpty { "Invite ${i.code}" }

    /** The line under an invite: uses, when the link ends, what the accounts get. */
    fun line(i: Invite, now: Double, day: (Double) -> String = ::formatDay): String {
        val uses = if (i.maxUses == 0) "${i.used} used, no limit" else "${i.used} of ${i.maxUses} used"
        val ends = when {
            i.expiresAt == null -> "never ends"
            i.expiresAt <= now -> "ended ${day(i.expiresAt)}"
            else -> "ends ${day(i.expiresAt)}"
        }
        val servers = listOfNotNull("Jellyfin".takeIf { i.jellyfin }, "Plex".takeIf { i.plex })
        val gets = if (servers.isEmpty()) "" else " · " + servers.joinToString(" and ")
        val lasts = if (i.accountDays > 0) " · accounts last ${i.accountDays} days" else ""
        return "$uses · $ends$gets$lasts"
    }

    /** The invite's badge: Open, Turned off, Ended or Used up. */
    fun badge(i: Invite, now: Double): Pair<String, String> = when {
        i.disabled -> "Turned off" to "warn"
        i.expiresAt != null && i.expiresAt <= now -> "Ended" to "err"
        i.maxUses > 0 && i.used >= i.maxUses -> "Used up" to ""
        else -> "Open" to "ok"
    }

    /** The media server choices of a new invite, from what the server has set up (`GET /api/media-servers`). */
    fun serverChoices(status: JsonElement?): List<Pair<String, Pair<Boolean, Boolean>>> {
        val o = status as? JsonObject
        val jf = o?.obj("jellyfin")?.bool("configured") == true
        val px = o?.obj("plex")?.bool("configured") == true
        return listOfNotNull(
            "Marquee only" to (false to false),
            ("Jellyfin" to (true to false)).takeIf { jf },
            ("Plex" to (false to true)).takeIf { px },
            ("Jellyfin and Plex" to (true to true)).takeIf { jf && px },
        )
    }

    /** The body of `POST /api/invites` for the phone's choices (every library on the servers chosen). */
    fun newInviteBody(n: NewInvite): JsonObject = buildJsonObject {
        put("label", n.label.trim())
        put("max_uses", n.maxUses)
        put("expires_in_days", n.days)
        if (n.jellyfin) putJsonObject("jellyfin") { put("libraries", "*") }
        if (n.plex) putJsonObject("plex") { put("libraries", "*") }
    }

    /** The body of `PATCH /api/invites/{code}` turning it off or on. */
    fun disableBody(disabled: Boolean): JsonObject = buildJsonObject { put("disabled", disabled) }

    private fun JsonObject.seconds(key: String): Double? = (get(key) as? JsonPrimitive)?.doubleOrNull?.takeIf { it > 0.0 }
}
