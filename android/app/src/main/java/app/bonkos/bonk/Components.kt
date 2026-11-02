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
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/* La charte de l'application, tirée de design/tokens.json : textes, boutons, cadres. */

@Composable
internal fun BonkTheme(content: @Composable () -> Unit) {
    val ink = Color(Tokens.INK)
    val paper = Color(Tokens.PAPER)
    MaterialTheme(
        colorScheme =
            lightColorScheme(
                primary = ink,
                onPrimary = paper,
                background = paper,
                onBackground = ink,
                surface = paper,
                onSurface = ink,
                outline = ink,
                surfaceContainer = paper,
            ),
        content = content,
    )
}

@Composable
internal fun Small(text: String, color: Long = Tokens.SECONDARY) {
    Text(text, fontSize = Tokens.TEXT_SMALL.sp, color = Color(color))
}

@Composable
internal fun Body(text: String) {
    Text(text, fontSize = Tokens.TEXT_BODY.sp)
}

@Composable
internal fun Primary(label: String, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        colors =
            ButtonDefaults.buttonColors(
                containerColor = Color(Tokens.INK),
                contentColor = Color(Tokens.PAPER),
                disabledContainerColor = Color(Tokens.HOVER),
                disabledContentColor = Color(Tokens.SECONDARY),
            ),
    ) {
        Body(label)
    }
}

@Composable
internal fun Secondary(label: String, onClick: () -> Unit) {
    OutlinedButton(
        onClick = onClick,
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        border = BorderStroke(1.dp, Color(Tokens.INK)),
    ) {
        Body(label)
    }
}

@Composable
internal fun Boxed(modifier: Modifier = Modifier, selected: Boolean = false, content: @Composable () -> Unit) {
    OutlinedCard(
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        border = BorderStroke(if (selected) 3.dp else 1.dp, Color(Tokens.INK)),
        colors = CardDefaults.outlinedCardColors(containerColor = Color(Tokens.PAPER)),
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
