package app.bonkos.bonk

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Checkbox
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun Draft(panel: DraftPanel, send: (ToRust) -> Unit) {
    Boxed {
        Body(panel.title)
        Menu(panel.menu, send)
        Secondary(panel.advancedButton) { send(ToggleAdvanced) }
        if (panel.advanced) {
            TextInput(panel.image, { send(ImageInput(it)) }, Modifier.fillMaxWidth(), label = panel.imageHint)
            Secondary(panel.pick) { send(Browse) }
            TextInput(panel.hash, { send(HashInput(it)) }, Modifier.fillMaxWidth(), label = panel.hashHint)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.raw, { send(Raw(it)) })
                Small(panel.rawLabel, LocalPalette.current.ink)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.mbr, { send(Mbr(it)) })
                Small(panel.mbrLabel, LocalPalette.current.ink)
            }
        }
        OneLine(panel.imageLine)
        panel.summary?.let { Body(it) }
        Text(
            panel.warning,
            fontSize = Tokens.TEXT_SMALL.sp,
            color = Color(LocalPalette.current.warn),
            modifier =
                Modifier.fillMaxWidth()
                    .background(Color(LocalPalette.current.warnSoft))
                    .padding(Tokens.SPACE_MEDIUM.dp),
        )
        Primary(panel.button, enabled = panel.enabled) { send(Start) }
        Secondary(panel.vaultButton) { send(VaultButton("open")) }
    }
}

@Composable
internal fun Running(panel: RunningPanel, send: (ToRust) -> Unit) {
    Boxed {
        Body(panel.line)
        Small(stringResource(R.string.do_not_unplug))
        ProgressBar(panel.fraction)
        Primary(panel.cancel) { send(Cancel) }
    }
}

/** Without a known total the bar scrolls instead of lying about progress. */
@Composable
internal fun ProgressBar(fraction: Float?) {
    if (fraction == null) {
        LinearProgressIndicator(modifier = Modifier.fillMaxWidth(), color = Color(LocalPalette.current.ink))
    } else {
        LinearProgressIndicator(
            progress = { fraction },
            modifier = Modifier.fillMaxWidth(),
            color = Color(LocalPalette.current.ink),
        )
    }
}
