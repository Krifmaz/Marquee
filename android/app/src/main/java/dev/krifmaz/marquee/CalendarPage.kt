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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SectionHead
import dev.krifmaz.kit.compose.widgets.SmallButton
import dev.krifmaz.kit.compose.widgets.Spinner
import java.time.LocalDate

/**
 * The calendar: a week of movies coming out and episodes airing, by day, from `GET /api/calendar`; Earlier and
 * Later go a week back or on. The list's resource id ("home-list") is what the benchmarks scroll: this is the
 * first page.
 */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
fun CalendarPage(app: App) {
    val today = remember { LocalDate.now() }
    var week by rememberSaveable { mutableIntStateOf(0) }
    var round by remember { mutableIntStateOf(0) }
    val start = CalendarRules.pageStart(today, week)
    val state = rememberLoaded(app, "/api/calendar" + CalendarRules.query(start), round) { CalendarRules.byDay(CalendarRules.parse(it)) }
    val p = MaterialTheme.palette

    Column(Modifier.fillMaxSize()) {
        SectionHead("Calendar", subtitle = CalendarRules.rangeLabel(start, today))
        Row(Modifier.fillMaxWidth().padding(horizontal = Gutter), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            SmallButton("Earlier") { week -= 1 }
            if (week != 0) SmallButton("This week") { week = 0 }
            SmallButton("Later") { week += 1 }
            Spacer(Modifier.weight(1f))
            if (state.loading) Spinner()
        }
        LoadNote(state) { round += 1 }
        val days = state.value
        when {
            days == null && state.error != null -> ErrorBox(state.error) { round += 1 }

            days == null -> Unit

            days.isEmpty() -> EmptyState(R.drawable.ic_tab_calendar, "Nothing this week", "Movies and episodes Marquee looks for show here on the day they come out.")

            else ->
                LazyColumn(
                    Modifier
                        .fillMaxSize()
                        .semantics { testTagsAsResourceId = true }
                        .testTag("home-list"),
                ) {
                    for (day in days) {
                        item(key = "day-${day.date}") {
                            Text(
                                CalendarRules.dayLabel(day.date, today),
                                style = MaterialTheme.typography.titleSmall,
                                color = if (day.date == today) p.text else p.muted,
                                modifier = Modifier.padding(start = Gutter, end = Gutter, top = 16.dp, bottom = 4.dp),
                            )
                        }
                        items(day.entries, key = { it.key }) { e ->
                            val (badge, kind) = CalendarRules.status(e, today)
                            PosterRow(app, e.poster, e.title, e.line, badge, kind)
                            RowDivider(startIndent = 68.dp)
                        }
                    }
                    item { Spacer(Modifier.height(24.dp)) }
                }
        }
    }
}
