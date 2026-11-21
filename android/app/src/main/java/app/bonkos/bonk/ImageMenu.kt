package app.bonkos.bonk

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
internal fun Picker(selected: String, options: List<String>, all: String, filter: (String?) -> ToRust, send: (ToRust) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Secondary(selected.ifEmpty { all }) { open = true }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(text = { Body(all) }, onClick = {
                open = false
                send(filter(null))
            })
            options.forEach { option ->
                DropdownMenuItem(text = { Body(option) }, onClick = {
                    open = false
                    send(filter(option))
                })
            }
        }
    }
}

@Composable
internal fun Menu(menu: MenuView, send: (ToRust) -> Unit) {
    if (!menu.ready) {
        Small(menu.line)
        return
    }
    Secondary(menu.button) { send(ToggleMenu) }
    if (menu.open) MenuSheet(menu, send)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MenuSheet(menu: MenuView, send: (ToRust) -> Unit) {
    ModalBottomSheet(
        onDismissRequest = { send(ToggleMenu) },
        containerColor = Color(LocalPalette.current.paper),
    ) {
        Column(
            Modifier.padding(horizontal = Tokens.SPACE_PAGE.dp),
            verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
        ) {
            TextInput(menu.search, { send(MenuSearch(it)) }, Modifier.fillMaxWidth(), placeholder = menu.searchHint)
            Row(horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
                Picker(menu.category.orEmpty(), menu.categories, menu.allCategories, ::CategoryFilter, send)
                Picker(menu.base.orEmpty(), menu.bases, menu.allBases, ::BaseFilter, send)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
                Picker(menu.system.orEmpty(), menu.systems, menu.allSystems, ::SystemFilter, send)
                Picker(menu.arch.orEmpty(), menu.arches, menu.allArches, ::ArchFilter, send)
            }
            val rows = menu.rows
            LazyColumn(
                Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_SMALL.dp),
                contentPadding = PaddingValues(bottom = Tokens.SPACE_PAGE.dp),
            ) {
                items(rows.size) { position ->
                    val row = rows[position]
                    val index = row.index
                    val modifier =
                        if (index != null) {
                            Modifier.heightIn(min = TOUCH_DP.dp).clickable { send(Choose(index)) }
                        } else {
                            Modifier
                        }
                    Boxed(modifier = modifier) {
                        Small(row.title, if (index != null) LocalPalette.current.ink else LocalPalette.current.muted)
                        row.note?.let { Small(it) }
                    }
                }
            }
        }
    }
}
