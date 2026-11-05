package dev.krifmaz.marquee

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The account screens' rules: sign-in options, Plex, media access, expiry and invites. */
class AccountRulesTest {
    private fun json(s: String) = Json.parseToJsonElement(s)

    private val day: (Double) -> String = { "day ${it.toLong()}" }

    @Test
    fun signInOptionsAndPlex() {
        assertEquals(SignInOptions(jellyfin = true, plex = false, reset = true), AccountRules.options(json("""{"jellyfin":true,"plex":false,"reset":true}""")))
        assertEquals(SignInOptions(), AccountRules.options(null))
        val pin = AccountRules.plexPin(json("""{"id":"42","code":"ABCD","url":"https://app.plex.tv/auth#?code=ABCD"}"""))
        assertEquals("42", pin?.id)
        assertNull("not a plex.tv page", AccountRules.plexPin(json("""{"id":"42","code":"A","url":"javascript:x"}""")))
        val body = AccountRules.plexFinishBody("42", "phone-1", "Pixel 8")
        assertEquals("42", body["id"]!!.jsonPrimitive.content)
        assertEquals("phone-1", body["app"]!!.jsonObject["device_id"]!!.jsonPrimitive.content)
        assertTrue(AccountRules.stillWaiting(202))
        assertFalse(AccountRules.stillWaiting(200))
        assertEquals("sam", AccountRules.forgotBody("  sam ")["login"]!!.jsonPrimitive.content)
    }

    @Test
    fun mediaAccessInWords() {
        val info = AccountRules.account(
            json(
                """{"expires_at":null,"jellyfin":{"active":true,"linked":true,"remote_name":"sam","error":"","password_pending":false},
                "plex":{"active":false,"linked":true,"remote_name":"","error":""}}""",
            ),
        )
        assertNull(info.expiresAt)
        assertEquals(listOf("Jellyfin", "Plex"), info.media.map { it.server })
        assertEquals("On, as sam", AccountRules.accessWords(info.media[0]))
        assertEquals("On" to "ok", AccountRules.accessBadge(info.media[0]))
        assertEquals("Off" to "warn", AccountRules.accessBadge(info.media[1]))
        val broken = info.media[0].copy(error = "Jellyfin can't be reached")
        assertEquals("Not in step: Jellyfin can't be reached", AccountRules.accessWords(broken))
        assertTrue(AccountRules.account(json("""{"expires_at":null,"jellyfin":null,"plex":null}""")).media.isEmpty())
    }

    @Test
    fun whenTheAccountEnds() {
        val now = 1_000_000.0
        assertEquals("Your account doesn't end.", AccountRules.expiryWords(null, now, day))
        assertEquals("Your account ends on day 2000000.", AccountRules.expiryWords(2_000_000.0, now, day))
        assertTrue(AccountRules.expiryWords(now + 86400, now, day).startsWith("Your account ends soon"))
        assertTrue(AccountRules.expiryWords(now - 1, now, day).startsWith("Your account ended"))
        assertEquals(1_700_000_000.0, AccountRules.account(json("""{"expires_at":1700000000.0}""")).expiresAt)
    }

    @Test
    fun invitesListedAndShared() {
        val list = AccountRules.invites(
            json(
                """[{"code":"abc","link":"/#/invite/abc","label":"","max_uses":1,"used":1,"expires_at":null,"account_days":30,
                "role":"requester","jellyfin":{"libraries":"*"},"plex":null,"disabled":false,"usable":false,"why_not":"used"},
                {"code":"def","link":"https://m.example.com/#/invite/def","label":"Family","max_uses":0,"used":2,"expires_at":500.0,
                "account_days":0,"role":"viewer","jellyfin":null,"plex":{"libraries":["1"]},"disabled":true,"usable":false,"why_not":"off"}]""",
            ),
        )
        assertEquals(2, list.size)
        val (a, b) = list
        assertEquals("Invite abc", AccountRules.title(a))
        assertEquals("Family", AccountRules.title(b))
        assertEquals("https://m.example.com/#/invite/abc", AccountRules.fullLink(a.link, "https://m.example.com/"))
        assertEquals(b.link, AccountRules.fullLink(b.link, "https://other"))
        assertEquals("1 of 1 used · never ends · Jellyfin · accounts last 30 days", AccountRules.line(a, 1000.0, day))
        assertEquals("2 used, no limit · ended day 500 · Plex", AccountRules.line(b, 1000.0, day))
        assertEquals("Used up" to "", AccountRules.badge(a, 1000.0))
        assertEquals("Turned off" to "warn", AccountRules.badge(b, 1000.0))
        assertEquals("Open" to "ok", AccountRules.badge(a.copy(used = 0), 1000.0))
        assertTrue(AccountRules.shareText("Marquee", "L").endsWith(": L"))
    }

    @Test
    fun aNewInvite() {
        val choices = AccountRules.serverChoices(json("""{"jellyfin":{"configured":true},"plex":{"configured":false}}"""))
        assertEquals(listOf("Marquee only", "Jellyfin"), choices.map { it.first })
        assertEquals(4, AccountRules.serverChoices(json("""{"jellyfin":{"configured":true},"plex":{"configured":true}}""")).size)
        val body: JsonObject = AccountRules.newInviteBody(NewInvite(" Family ", 5, 0, jellyfin = true, plex = false))
        assertEquals("Family", body["label"]!!.jsonPrimitive.content)
        assertEquals("5", body["max_uses"]!!.jsonPrimitive.content)
        assertEquals("0", body["expires_in_days"]!!.jsonPrimitive.content)
        assertEquals("*", body["jellyfin"]!!.jsonObject["libraries"]!!.jsonPrimitive.content)
        assertFalse(body.containsKey("plex"))
        assertEquals("true", AccountRules.disableBody(true)["disabled"]!!.jsonPrimitive.content)
    }
}
