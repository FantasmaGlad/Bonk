package app.bonkos.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Below this width the title and the buttons do not fit on one line. */
private const val NARROW_DP = 600

/** Title and page buttons; on a narrow screen the page buttons share a second line, half each. */
@Composable
internal fun Header(state: ScreenState, send: (ToRust) -> Unit) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        if (maxWidth < NARROW_DP.dp) {
            Column(verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Title(Modifier.weight(1f))
                    state.incognito?.let { IncognitoButton(it, send) }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
                    Pages(state, send, Modifier.weight(1f))
                }
            }
        } else {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
            ) {
                Title(Modifier.weight(1f))
                state.incognito?.let { IncognitoButton(it, send) }
                Pages(state, send)
            }
        }
    }
}

@Composable
private fun Title(modifier: Modifier = Modifier) {
    Text("BONK", fontSize = Tokens.TEXT_TITLE.sp, maxLines = 1, softWrap = false, modifier = modifier)
}

@Composable
private fun Pages(state: ScreenState, send: (ToRust) -> Unit, modifier: Modifier = Modifier) {
    state.catalogButton?.let { label -> Secondary(label, modifier = modifier) { send(CatalogAction("open")) } }
    Secondary(state.settingsButton, modifier = modifier) { send(SettingsAction("open")) }
}
