package app.bonkos.bonk

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

private const val GLYPH_DP = 22

@Composable
internal fun IncognitoButton(view: IncognitoView, send: (ToRust) -> Unit) {
    val palette = LocalPalette.current
    val glyph = if (!view.available) palette.muted else if (view.on) palette.onAccent else palette.ink
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
    ) {
        OutlinedButton(
            onClick = { send(ToggleIncognito) },
            enabled = view.available,
            shape = CircleShape,
            border = BorderStroke(1.dp, Color(if (view.on) palette.accent else palette.muted)),
            colors =
                ButtonDefaults.outlinedButtonColors(
                    containerColor = Color(if (view.on) palette.accent else palette.surface),
                    disabledContainerColor = Color(palette.sunk),
                ),
            contentPadding = PaddingValues(0.dp),
            modifier =
                Modifier.size(TOUCH_DP.dp).semantics {
                    contentDescription = view.label
                    role = Role.Switch
                },
        ) {
            Image(
                painterResource(R.drawable.incognito),
                contentDescription = null,
                colorFilter = ColorFilter.tint(Color(glyph)),
                modifier = Modifier.size(GLYPH_DP.dp),
            )
        }
        if (!view.available) {
            TextButton(onClick = { send(OpenTorProject) }) {
                Small(view.link, palette.accent)
            }
        }
    }
}
