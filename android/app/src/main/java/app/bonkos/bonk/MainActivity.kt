package app.bonkos.bonk

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.net.Uri
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.util.Log
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
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
import androidx.core.content.ContextCompat
import java.util.Locale
import kotlin.concurrent.thread
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject

private const val TAG = "Bonk"
private const val MEBIBYTE = 1_048_576
private const val CLOCK_MS = 500L
private const val TICKS_PER_REFRESH = 4

class MainActivity : ComponentActivity() {
    private var revision by mutableIntStateOf(0)
    private var screen by mutableStateOf(JSONObject())
    private var screenText = ""
    private var keepingOn = false
    private var driveErrors by mutableStateOf(mapOf<String, String>())
    private var picked: ParcelFileDescriptor? = null
    private lateinit var actions: Actions

    private val usbEvents =
        object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                revision++
                send(JSONObject().put("type", "tick"))
                actions.enumerate()
            }
        }

    private val pickFile =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            uri?.let {
                // Le chemin du descripteur se lit comme un fichier : le moteur reste celui de l'ordinateur.
                picked?.close()
                picked = contentResolver.openFileDescriptor(it, "r")
                picked?.let { file ->
                    send(JSONObject().put("type", "picked").put("value", "/proc/self/fd/${file.fd}"))
                }
            }
        }

    private fun send(message: JSONObject) {
        runOnUiThread {
            if (message.optString("type") == "cancel") actions.cancelled = true
            val answer = JSONObject(Native.dispatch(message.toString()))
            // Redessiner ou toucher à la fenêtre sans changement fait tressauter l'écran à chaque battement.
            val state = answer.optJSONObject("state") ?: JSONObject()
            val text = state.toString()
            if (text != screenText) {
                screenText = text
                screen = state
                keepScreenOn(state.optBoolean("running"))
            }
            val effects = answer.optJSONArray("effects") ?: JSONArray()
            for (index in 0 until effects.length()) accomplish(effects.getJSONObject(index))
        }
    }

    private fun keepScreenOn(on: Boolean) {
        if (on == keepingOn) return
        keepingOn = on
        if (on) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    private fun accomplish(effect: JSONObject) {
        when (effect.getString("do")) {
            "enumerate" -> actions.enumerate()
            "catalog" -> actions.catalog()
            "pick" -> pickFile.launch(arrayOf("*/*"))
            "write" -> actions.write(effect)
            "save_language" ->
                getPreferences(MODE_PRIVATE).edit().putString("lang", effect.getString("value")).apply()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Fond clair : les icônes du système doivent rester sombres, quel que soit le thème du téléphone.
        val bars = SystemBarStyle.light(Tokens.PAPER.toInt(), Tokens.INK.toInt())
        enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
        actions =
            Actions(this, ::send) { device, error ->
                runOnUiThread {
                    driveErrors = if (error == null) driveErrors - device else driveErrors + (device to error)
                }
            }
        val saved = getPreferences(MODE_PRIVATE).getString("lang", null)
        Native.setLanguage((saved ?: Locale.getDefault().language) == "fr")
        val filter =
            IntentFilter().apply {
                addAction(ACTION_PERMISSION)
                addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
                addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
            }
        ContextCompat.registerReceiver(this, usbEvents, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        val usb = getSystemService(UsbManager::class.java)
        actions.enumerate()
        setContent {
            BonkTheme {
                Surface(Modifier.fillMaxSize(), color = Color(Tokens.PAPER)) {
                    LaunchedEffect(Unit) {
                        var ticks = 0
                        while (true) {
                            delay(CLOCK_MS)
                            ticks = (ticks + 1) % TICKS_PER_REFRESH
                            send(JSONObject().put("type", if (ticks == 0) "tick" else "clock"))
                        }
                    }
                    Screen(screen, usb, revision, driveErrors, ::send)
                }
            }
        }
    }

    override fun onDestroy() {
        unregisterReceiver(usbEvents)
        picked?.close()
        super.onDestroy()
    }
}

@Composable
private fun BonkTheme(content: @Composable () -> Unit) {
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
private fun Small(text: String, color: Long = Tokens.SECONDARY) {
    Text(text, fontSize = Tokens.TEXT_SMALL.sp, color = Color(color))
}

@Composable
private fun Body(text: String) {
    Text(text, fontSize = Tokens.TEXT_BODY.sp)
}

@Composable
private fun Primary(label: String, enabled: Boolean = true, onClick: () -> Unit) {
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
private fun Secondary(label: String, onClick: () -> Unit) {
    OutlinedButton(
        onClick = onClick,
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        border = BorderStroke(1.dp, Color(Tokens.INK)),
    ) {
        Body(label)
    }
}

@Composable
private fun Boxed(selected: Boolean = false, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
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

private fun message(type: String) = JSONObject().put("type", type)

@Composable
private fun Screen(
    state: JSONObject,
    usb: UsbManager,
    revision: Int,
    errors: Map<String, String>,
    send: (JSONObject) -> Unit,
) {
    Column(
        Modifier.safeDrawingPadding()
            .padding(Tokens.SPACE_PAGE.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_LARGE.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("BONK", fontSize = Tokens.TEXT_TITLE.sp, modifier = Modifier.weight(1f))
            Secondary(state.optString("language")) { send(message("language")) }
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
                LinearProgressIndicator(
                    progress = { panel.optDouble("fraction").toFloat() },
                    modifier = Modifier.fillMaxWidth(),
                    color = Color(Tokens.INK),
                )
                Primary(panel.optString("cancel")) { send(message("cancel")) }
            }
            "finished" -> Boxed {
                Body(panel.optString("message"))
                Primary(panel.optString("close")) { send(message("dismiss")) }
            }
        }
        HorizontalDivider(color = Color(Tokens.INK))
        Tests(usb, revision)
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
private fun DiskCard(disk: JSONObject, send: (JSONObject) -> Unit) {
    val modifier = Modifier.clickable { send(message("select").put("id", disk.getString("id"))) }
    Boxed(disk.optBoolean("selected"), modifier) {
        Row {
            Text(disk.optString("model"), fontSize = Tokens.TEXT_BODY.sp, modifier = Modifier.weight(1f))
            Small(disk.optString("badge"), Tokens.INK)
        }
        Small(disk.optString("facts"))
        Small(disk.optString("state"))
    }
}

@Composable
private fun Picker(selected: String, options: List<String>, all: String, type: String, send: (JSONObject) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Secondary(selected.ifEmpty { all }) { open = true }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(text = { Body(all) }, onClick = {
                open = false
                send(message(type))
            })
            options.forEach { option ->
                DropdownMenuItem(text = { Body(option) }, onClick = {
                    open = false
                    send(message(type).put("value", option))
                })
            }
        }
    }
}

private fun JSONArray?.strings(): List<String> =
    if (this == null) emptyList() else List(length()) { getString(it) }

@Composable
private fun Menu(menu: JSONObject, send: (JSONObject) -> Unit) {
    if (!menu.optBoolean("ready")) {
        Small(menu.optString("line"))
        return
    }
    Secondary(menu.optString("button")) { send(message("menu")) }
    if (!menu.optBoolean("open")) return
    Row(horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
        Picker(
            menu.optString("category").takeUnless { menu.isNull("category") }.orEmpty(),
            menu.optJSONArray("categories").strings(),
            menu.optString("all_categories"),
            "category",
            send,
        )
        Picker(
            menu.optString("system").takeUnless { menu.isNull("system") }.orEmpty(),
            menu.optJSONArray("systems").strings(),
            menu.optString("all_systems"),
            "system",
            send,
        )
    }
    val rows = menu.optJSONArray("rows") ?: JSONArray()
    for (position in 0 until rows.length()) {
        val row = rows.getJSONObject(position)
        val usable = !row.isNull("index")
        val modifier =
            if (usable) {
                Modifier.clickable { send(message("choose").put("index", row.getInt("index"))) }
            } else {
                Modifier
            }
        Boxed(modifier = modifier) {
            Small(row.optString("title"), if (usable) Tokens.INK else Tokens.SECONDARY)
            if (!row.isNull("note")) Small(row.optString("note"))
        }
    }
}

@Composable
private fun DraftPanel(panel: JSONObject, send: (JSONObject) -> Unit) {
    Boxed {
        Body(panel.optString("title"))
        Small(panel.optString("summary"), Tokens.INK)
        Small(panel.optString("warning"))
        panel.optJSONObject("menu")?.let { Menu(it, send) }
        Secondary(panel.optString("advanced_button")) { send(message("advanced")) }
        if (panel.optBoolean("advanced")) {
            OutlinedTextField(
                value = panel.optString("image"),
                onValueChange = { send(message("image").put("value", it)) },
                label = { Small(panel.optString("image_hint")) },
                modifier = Modifier.fillMaxWidth(),
            )
            Secondary(panel.optString("pick")) { send(message("browse")) }
            OutlinedTextField(
                value = panel.optString("hash"),
                onValueChange = { send(message("hash").put("value", it)) },
                label = { Small(panel.optString("hash_hint")) },
                modifier = Modifier.fillMaxWidth(),
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.optBoolean("raw"), { send(message("raw").put("on", it)) })
                Small(panel.optString("raw_label"), Tokens.INK)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(panel.optBoolean("mbr"), { send(message("mbr").put("on", it)) })
                Small(panel.optString("mbr_label"), Tokens.INK)
            }
        }
        Primary(panel.optString("button"), panel.optBoolean("enabled")) { send(message("start")) }
    }
}

/** Essais réservés au développement : ils décident si l'écriture marche sur cet appareil. */
@Composable
private fun Tests(usb: UsbManager, revision: Int) {
    val drives = remember(revision) { usb.drives().filter(usb::hasPermission) }
    Body(stringResource(R.string.tests_title))
    Small(stringResource(R.string.tests_how))
    drives.forEach { device -> DeviceTests(usb, device) }
}

@Composable
private fun DeviceTests(usb: UsbManager, device: UsbDevice) {
    var report by remember { mutableStateOf("") }
    val working = stringResource(R.string.working)
    fun run(work: (Int, ByteArray) -> String) {
        report = working
        thread {
            val result = runCatching { usb.withDrive(device, work) }.getOrElse { "error: ${it.message}" }
            Log.i(TAG, result)
            report = result
        }
    }
    Boxed {
        val serial = device.serialNumber.orEmpty()
        Small(stringResource(R.string.serial_read, serial))
        Primary(stringResource(R.string.read_test)) {
            run { fd, raw -> Native.probe(fd, raw) + "\n" + Native.readHash(fd, raw, 0, MEBIBYTE) }
        }
        if (serial == BuildConfig.BENCH_SERIAL && serial.isNotEmpty()) {
            Small(stringResource(R.string.write_warning))
            Primary(stringResource(R.string.write_test)) { run { fd, raw -> Native.writeBench(fd, raw) } }
        } else {
            Small(stringResource(R.string.write_reserved, BuildConfig.BENCH_SERIAL))
        }
        if (report.isNotEmpty()) Small(report, Tokens.INK)
    }
}
