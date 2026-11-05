package dev.krifmaz.marquee

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.widgets.Badge
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.SmallButton
import dev.krifmaz.kit.compose.R as KitR

/**
 * A list row with a poster: the title, a line under it, a badge, and [trailing] at the end (a menu). The poster is
 * a small 2:3 picture; a film icon stands in when there is none.
 */
@Composable
fun PosterRow(app: App, poster: String, title: String, line: String, badge: String, badgeKind: String, trailing: @Composable () -> Unit = {}) {
    val p = MaterialTheme.palette
    Row(
        Modifier.fillMaxWidth().padding(horizontal = Gutter, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        val shape = RoundedCornerShape(6.dp)
        Box(Modifier.size(width = 40.dp, height = 60.dp).clip(shape).background(p.panel2), contentAlignment = Alignment.Center) {
            if (poster.isNotEmpty()) {
                AsyncImage(
                    model = poster,
                    contentDescription = null,
                    imageLoader = app.imageLoader,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.size(width = 40.dp, height = 60.dp),
                )
            } else {
                Icon(painterResource(R.drawable.ic_film), contentDescription = null, tint = p.faint, modifier = Modifier.size(20.dp))
            }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, color = p.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (line.isNotEmpty()) Text(line, style = MaterialTheme.typography.bodySmall, color = p.muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (badge.isNotEmpty()) Badge(badge, badgeKind, Modifier.padding(top = 2.dp))
        }
        trailing()
    }
}

/**
 * The line under a page's head about what is shown: a saved copy and how old it is (offline mode), why the last
 * try failed, with Refresh. Nothing while the server's latest answer shows.
 */
@Composable
fun <T> LoadNote(state: Loaded<T>, onRefresh: () -> Unit) {
    val p = MaterialTheme.palette
    val words = when {
        state.savedAt != null && state.error != null -> AnswerCache.ageWords(state.savedAt) + " · " + state.error
        state.savedAt != null -> AnswerCache.ageWords(state.savedAt)
        state.error != null && state.value != null -> "Not up to date · " + state.error
        else -> return
    }
    Row(Modifier.fillMaxWidth().padding(horizontal = Gutter, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(words, style = MaterialTheme.typography.bodySmall, color = p.muted, modifier = Modifier.weight(1f))
        if (!state.loading) SmallButton("Refresh", iconRes = KitR.drawable.kit_ic_refresh, onClick = onRefresh)
    }
}
