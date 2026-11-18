package app.bonkos.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

@Composable
internal fun Vault(vault: VaultView, send: (ToRust) -> Unit) {
    Boxed {
        Body(vault.title)
        vault.lines.forEach { Small(it, LocalPalette.current.ink) }
        if (vault.busy) {
            Small(stringResource(R.string.do_not_unplug))
            ProgressBar(vault.fraction)
        }
        vault.images.forEach { image ->
            Row(horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
                Small(image.size)
                OneLine(image.name, size = Tokens.TEXT_SMALL)
            }
        }
        vault.buttons.forEach { button ->
            if (button.primary) {
                Primary(button.label) { send(VaultButton(button.action)) }
            } else {
                Secondary(button.label) { send(VaultButton(button.action)) }
            }
        }
    }
}
