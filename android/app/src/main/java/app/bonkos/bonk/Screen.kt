package app.bonkos.bonk

import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp


private const val MENU_HEIGHT_DP = 360

@Composable
internal fun Screen(
    state: ScreenState,
    usb: UsbManager,
    revision: Int,
    errors: Map<String, String>,
    send: (ToRust) -> Unit,
) {
    Column(
        Modifier.safeDrawingPadding()
            .padding(Tokens.SPACE_PAGE.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_LARGE.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("BONK", fontSize = Tokens.TEXT_TITLE.sp, modifier = Modifier.weight(1f))
            Secondary(state.language) { send(SwitchLanguage) }
        }
        Small(state.intro)
        Small(state.status)
        val waiting = remember(revision, errors) { usb.drives().filterNot(usb::hasPermission) }
        waiting.forEach { device -> AskAccess(usb, device) }
        errors.forEach { (device, error) ->
            Boxed {
                Body(stringResource(R.string.drive_unreadable, device))
                Small(error, Tokens.INK)
            }
        }
        if (state.disks.isEmpty() && waiting.isEmpty() && errors.isEmpty()) Body(state.empty)
        val panel = state.panel
        state.disks.forEach { disk ->
            DiskCard(disk, send)
            if (disk.selected && panel is DraftPanel) Draft(panel, send)
        }
        when (panel) {
            is RunningPanel -> Boxed {
                Body(panel.line)
                Small(stringResource(R.string.do_not_unplug))
                // Without a known total the bar scrolls instead of lying about progress.
                val fraction = panel.fraction
                if (fraction == null) {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth(), color = Color(Tokens.INK))
                } else {
                    LinearProgressIndicator(
                        progress = { fraction },
                        modifier = Modifier.fillMaxWidth(),
                        color = Color(Tokens.INK),
                    )
                }
                Primary(panel.cancel) { send(Cancel) }
            }
            is FinishedPanel -> Boxed {
                Body(panel.message)
                Primary(panel.close) { send(Dismiss) }
            }
            is DraftPanel, null -> {}
        }
        // Absent from the published build: only useful with the test key named at build time.
        if (BuildConfig.BENCH_SERIAL.isNotEmpty()) {
            HorizontalDivider(color = Color(Tokens.INK))
            Tests(usb, revision)
        }
    }
}

@Composable
private fun AskAccess(usb: UsbManager, device: UsbDevice) {
    val context = LocalContext.current
    Boxed {
        Body("${device.manufacturerName.orEmpty()} ${device.productName.orEmpty()}")
        Small(stringResource(R.string.allow_why))
        Primary(stringResource(R.string.allow)) { usb.askPermission(context, device) }
    }
}

@Composable
private fun DiskCard(disk: DiskView, send: (ToRust) -> Unit) {
    val modifier = Modifier.clickable { send(Select(disk.id)) }
    Boxed(modifier, disk.selected) {
        Row {
            Text(disk.model, fontSize = Tokens.TEXT_BODY.sp, modifier = Modifier.weight(1f))
            Small(disk.badge, Tokens.INK)
        }
        Small(disk.facts)
        Small(disk.state)
    }
}

@Composable
private fun Picker(selected: String, options: List<String>, all: String, filter: (String?) -> ToRust, send: (ToRust) -> Unit) {
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
private fun Menu(menu: MenuView, send: (ToRust) -> Unit) {
    if (!menu.ready) {
        Small(menu.line)
        return
    }
    Secondary(menu.button) { send(ToggleMenu) }
    if (!menu.open) return
    Row(horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
        Picker(menu.category.orEmpty(), menu.categories, menu.allCategories, ::CategoryFilter, send)
        Picker(menu.system.orEmpty(), menu.systems, menu.allSystems, ::SystemFilter, send)
    }
    val rows = menu.rows
    // Only visible rows are drawn: the catalog has over 150.
    LazyColumn(
        Modifier.fillMaxWidth().height(MENU_HEIGHT_DP.dp),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_SMALL.dp),
    ) {
        items(rows.size) { position ->
            val row = rows[position]
            val index = row.index
            val modifier = if (index != null) Modifier.clickable { send(Choose(index)) } else Modifier
            Boxed(modifier = modifier) {
                Small(row.title, if (index != null) Tokens.INK else Tokens.SECONDARY)
                row.note?.let { Small(it) }
            }
        }
    }
}

@Composable
private fun Draft(panel: DraftPanel, send: (ToRust) -> Unit) {
    Boxed {
        Body(panel.title)
        panel.summary?.let { Small(it, Tokens.INK) }
        Small(panel.warning)
        Menu(panel.menu, send)
        Secondary(panel.advancedButton) { send(ToggleAdvanced) }
        if (panel.advanced) {
            OutlinedTextField(
                value = panel.image,
                onValueChange = { send(ImageInput(it)) },
                label = { Small(panel.imageHint) },
                modifier = Modifier.fillMaxWidth(),
            )
            Secondary(panel.pick) { send(Browse) }
            OutlinedTextField(
                value = panel.hash,
                onValueChange = { send(HashInput(it)) },
                label = { Small(panel.hashHint) },
                modifier = Modifier.fillMaxWidth(),
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.raw, { send(Raw(it)) })
                Small(panel.rawLabel, Tokens.INK)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.mbr, { send(Mbr(it)) })
                Small(panel.mbrLabel, Tokens.INK)
            }
        }
        Primary(panel.button, panel.enabled) { send(Start) }
    }
}
