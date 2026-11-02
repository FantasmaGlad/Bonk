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
import org.json.JSONArray
import org.json.JSONObject

/* L'écran, dessiné depuis l'état que rend Rust ; chaque geste devient un message typé (Protocol.kt). */

private const val MENU_HEIGHT_DP = 360

@Composable
internal fun Screen(
    state: JSONObject,
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
            Secondary(state.optString("language")) { send(SwitchLanguage) }
        }
        Small(state.optString("intro"))
        Small(state.optString("status"))
        val waiting = remember(revision, errors) { usb.drives().filterNot(usb::hasPermission) }
        waiting.forEach { device -> AskAccess(usb, device) }
        errors.forEach { (device, error) ->
            Boxed {
                Body(stringResource(R.string.drive_unreadable, device))
                Small(error, Tokens.INK)
            }
        }
        val disks = state.optJSONArray("disks") ?: JSONArray()
        if (disks.length() == 0 && waiting.isEmpty() && errors.isEmpty()) Body(state.optString("empty"))
        val panel = state.optJSONObject("panel")
        for (index in 0 until disks.length()) {
            val disk = disks.getJSONObject(index)
            DiskCard(disk, send)
            if (disk.optBoolean("selected") && panel?.optString("kind") == "draft") {
                DraftPanel(panel, send)
            }
        }
        when (panel?.optString("kind")) {
            "running" -> Boxed {
                Body(panel.optString("line"))
                Small(stringResource(R.string.do_not_unplug))
                LinearProgressIndicator(
                    progress = { panel.optDouble("fraction").toFloat() },
                    modifier = Modifier.fillMaxWidth(),
                    color = Color(Tokens.INK),
                )
                Primary(panel.optString("cancel")) { send(Cancel) }
            }
            "finished" -> Boxed {
                Body(panel.optString("message"))
                Primary(panel.optString("close")) { send(Dismiss) }
            }
        }
        // Absents de la version publiée : ils ne servent qu'avec la clé de test désignée à la construction.
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
private fun DiskCard(disk: JSONObject, send: (ToRust) -> Unit) {
    val modifier = Modifier.clickable { send(Select(disk.getString("id"))) }
    Boxed(modifier, disk.optBoolean("selected")) {
        Row {
            Text(disk.optString("model"), fontSize = Tokens.TEXT_BODY.sp, modifier = Modifier.weight(1f))
            Small(disk.optString("badge"), Tokens.INK)
        }
        Small(disk.optString("facts"))
        Small(disk.optString("state"))
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

private fun JSONArray?.strings(): List<String> =
    if (this == null) emptyList() else List(length()) { getString(it) }

@Composable
private fun Menu(menu: JSONObject, send: (ToRust) -> Unit) {
    if (!menu.optBoolean("ready")) {
        Small(menu.optString("line"))
        return
    }
    Secondary(menu.optString("button")) { send(ToggleMenu) }
    if (!menu.optBoolean("open")) return
    Row(horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
        Picker(
            menu.optString("category").takeUnless { menu.isNull("category") }.orEmpty(),
            menu.optJSONArray("categories").strings(),
            menu.optString("all_categories"),
            ::CategoryFilter,
            send,
        )
        Picker(
            menu.optString("system").takeUnless { menu.isNull("system") }.orEmpty(),
            menu.optJSONArray("systems").strings(),
            menu.optString("all_systems"),
            ::SystemFilter,
            send,
        )
    }
    val rows = menu.optJSONArray("rows") ?: JSONArray()
    // Seules les lignes visibles sont dessinées : le catalogue en compte plus de cent cinquante.
    LazyColumn(
        Modifier.fillMaxWidth().height(MENU_HEIGHT_DP.dp),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_SMALL.dp),
    ) {
        items(rows.length()) { position ->
            val row = rows.getJSONObject(position)
            val usable = !row.isNull("index")
            val modifier =
                if (usable) {
                    Modifier.clickable { send(Choose(row.getInt("index"))) }
                } else {
                    Modifier
                }
            Boxed(modifier = modifier) {
                Small(row.optString("title"), if (usable) Tokens.INK else Tokens.SECONDARY)
                if (!row.isNull("note")) Small(row.optString("note"))
            }
        }
    }
}

@Composable
private fun DraftPanel(panel: JSONObject, send: (ToRust) -> Unit) {
    Boxed {
        Body(panel.optString("title"))
        Small(panel.optString("summary"), Tokens.INK)
        Small(panel.optString("warning"))
        panel.optJSONObject("menu")?.let { Menu(it, send) }
        Secondary(panel.optString("advanced_button")) { send(ToggleAdvanced) }
        if (panel.optBoolean("advanced")) {
            OutlinedTextField(
                value = panel.optString("image"),
                onValueChange = { send(ImageInput(it)) },
                label = { Small(panel.optString("image_hint")) },
                modifier = Modifier.fillMaxWidth(),
            )
            Secondary(panel.optString("pick")) { send(Browse) }
            OutlinedTextField(
                value = panel.optString("hash"),
                onValueChange = { send(HashInput(it)) },
                label = { Small(panel.optString("hash_hint")) },
                modifier = Modifier.fillMaxWidth(),
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.optBoolean("raw"), { send(Raw(it)) })
                Small(panel.optString("raw_label"), Tokens.INK)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.optBoolean("mbr"), { send(Mbr(it)) })
                Small(panel.optString("mbr_label"), Tokens.INK)
            }
        }
        Primary(panel.optString("button"), panel.optBoolean("enabled")) { send(Start) }
    }
}
