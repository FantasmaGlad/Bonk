package app.bonkos.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Language, theme and the expert option: one choice per line, wide enough for a thumb. */
@Composable
internal fun SettingsPage(settings: SettingsView, send: (ToRust) -> Unit) {
    Column(
        Modifier.safeDrawingPadding()
            .padding(Tokens.SPACE_PAGE.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_LARGE.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(settings.title, fontSize = Tokens.TEXT_TITLE.sp, modifier = Modifier.weight(1f))
            Secondary(settings.back) { send(SettingsAction("close")) }
        }
        settings.sections.forEach { section -> Section(section, settings.enabled, send) }
        Column(verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_SMALL.dp)) {
            Body(settings.disksTitle)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(settings.showAll, { send(ShowAll(it)) })
                Small(settings.showAllLabel, LocalPalette.current.ink)
            }
        }
    }
}

@Composable
private fun Section(section: SettingsSection, enabled: Boolean, send: (ToRust) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_SMALL.dp)) {
        Body(section.title)
        Small(section.note)
        section.options.forEach { option ->
            Column(Modifier.fillMaxWidth()) {
                Secondary(option.label, chosen = option.chosen, enabled = enabled) {
                    send(SettingsAction(option.action, option.value))
                }
            }
        }
    }
}
