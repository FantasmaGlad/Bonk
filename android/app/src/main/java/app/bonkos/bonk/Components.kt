package app.bonkos.bonk

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp


/** Smallest height of anything that answers a touch. */
internal const val TOUCH_DP = 48

/** Light or dark, chosen once at start: the screen relaunches to change it. */
internal val LocalPalette = staticCompositionLocalOf { Tokens.LIGHT }

@Composable
internal fun BonkTheme(palette: Palette, content: @Composable () -> Unit) {
    val ink = Color(palette.ink)
    val paper = Color(palette.paper)
    val scheme =
        if (palette == Tokens.DARK) {
            darkColorScheme(
                primary = Color(palette.accent),
                onPrimary = Color(palette.onAccent),
                background = paper,
                onBackground = ink,
                surface = Color(palette.surface),
                onSurface = ink,
                outline = Color(palette.muted),
                surfaceContainer = Color(palette.surface),
            )
        } else {
            lightColorScheme(
                primary = Color(palette.accent),
                onPrimary = Color(palette.onAccent),
                background = paper,
                onBackground = ink,
                surface = Color(palette.surface),
                onSurface = ink,
                outline = Color(palette.muted),
                surfaceContainer = Color(palette.surface),
            )
        }
    CompositionLocalProvider(LocalPalette provides palette) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

@Composable
internal fun Small(text: String, color: Long = LocalPalette.current.muted) {
    Text(text, fontSize = Tokens.TEXT_SMALL.sp, color = Color(color))
}

@Composable
internal fun Body(text: String) {
    Text(text, fontSize = Tokens.TEXT_BODY.sp)
}

/** One line; a long path or name loses its middle, never its start or its end. */
@Composable
internal fun OneLine(text: String, color: Long = LocalPalette.current.ink, size: Int = Tokens.TEXT_BODY) {
    Text(
        text,
        fontSize = size.sp,
        color = Color(color),
        maxLines = 1,
        overflow = TextOverflow.MiddleEllipsis,
    )
}

@Composable
internal fun Primary(label: String, enabled: Boolean = true, onClick: () -> Unit) {
    val palette = LocalPalette.current
    Button(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = Color(palette.accent),
                contentColor = Color(palette.onAccent),
                disabledContainerColor = Color(palette.sunk),
                disabledContentColor = Color(palette.muted),
            ),
    ) {
        Body(label)
    }
}

@Composable
internal fun Secondary(label: String, chosen: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    val palette = LocalPalette.current
    OutlinedButton(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        // Its own outline marks the choice: never a second frame around it.
        border = BorderStroke(if (chosen) 2.dp else 1.dp, Color(if (chosen) palette.accent else palette.muted)),
        colors =
            ButtonDefaults.outlinedButtonColors(
                containerColor = Color(if (chosen) palette.accentSoft else palette.surface),
                contentColor = Color(palette.ink),
            ),
    ) {
        Body(label)
    }
}

@Composable
internal fun Boxed(modifier: Modifier = Modifier, selected: Boolean = false, content: @Composable () -> Unit) {
    val palette = LocalPalette.current
    OutlinedCard(
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        border = BorderStroke(if (selected) 2.dp else 1.dp, Color(if (selected) palette.accent else palette.muted)),
        colors =
            CardDefaults.outlinedCardColors(
                containerColor = Color(if (selected) palette.accentSoft else palette.surface),
            ),
        modifier = modifier.fillMaxWidth(),
    ) {
        Column(
            Modifier.padding(Tokens.SPACE_MEDIUM.dp),
            verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
        ) {
            content()
        }
    }
}
