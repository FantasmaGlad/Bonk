package app.bonkos.bonk

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun Disks(state: ScreenState, send: (ToRust) -> Unit) {
    val panel = state.panel
    val vault = state.vault
    state.disks.forEach { disk ->
        DiskCard(disk, send)
        if (disk.selected && vault != null) {
            Vault(vault, send)
        } else if (disk.selected && panel is DraftPanel) {
            Draft(panel, send)
        }
    }
}

@Composable
private fun DiskCard(disk: DiskView, send: (ToRust) -> Unit) {
    val modifier = Modifier.heightIn(min = TOUCH_DP.dp).clickable { send(Select(disk.id)) }
    Boxed(modifier, disk.selected) {
        Row {
            Text(
                disk.model,
                fontSize = Tokens.TEXT_BODY.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Small(disk.badge, LocalPalette.current.ink)
        }
        Small(disk.facts)
        Small(disk.state)
        disk.details.forEach { Small(it) }
    }
    // Outside the card: one control never sits inside another.
    disk.detailsButton?.let { label -> Secondary(label) { send(ToggleDetails) } }
}
