package app.bonkos.bonk

import androidx.compose.material3.OutlinedTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue

/**
 * The text typed stays on the screen. Rust answers each key later: taking its echo as the value would put
 * the cursor back before the letters typed meanwhile. Only a value Rust sets itself replaces the text.
 */
@Composable
internal fun TextInput(
    value: String,
    onChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    label: String? = null,
    placeholder: String? = null,
) {
    var shown by remember { mutableStateOf(TextFieldValue(value, TextRange(value.length))) }
    val sent = remember { mutableSetOf<String>() }
    LaunchedEffect(value) {
        if (value != shown.text && value !in sent) {
            shown = TextFieldValue(value, TextRange(value.length))
            sent.clear()
        }
    }
    OutlinedTextField(
        value = shown,
        onValueChange = { typed ->
            val changed = typed.text != shown.text
            shown = typed
            if (changed) {
                sent += typed.text
                onChange(typed.text)
            }
        },
        label = label?.let { { Small(it) } },
        placeholder = placeholder?.let { { Small(it) } },
        singleLine = true,
        modifier = modifier,
    )
}
