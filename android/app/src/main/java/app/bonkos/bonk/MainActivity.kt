package app.bonkos.bonk

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.net.Uri
import android.os.Bundle
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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.lazy.LazyColumn
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
import androidx.core.content.edit
import java.util.Locale
import kotlin.concurrent.thread
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject

private const val TAG = "Bonk"
private const val MEBIBYTE = 1_048_576
private const val CLOCK_MS = 500L
private const val TICKS_PER_REFRESH = 4
private const val NO_EFFECT = "\"effects\":[]"
private const val MENU_HEIGHT_DP = 360

class MainActivity : ComponentActivity() {
    private var revision by mutableIntStateOf(0)
    private var screen by mutableStateOf(JSONObject())
    private var screenText = ""
    private var lastAnswer = ""
    private var keepingOn = false
    private var driveErrors by mutableStateOf(mapOf<String, String>())
    private lateinit var actions: Actions

    private val usbEvents =
        object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                revision++
                send(Tick)
                actions.enumerate()
            }
        }

    private val pickFile =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            uri?.let {
                // Le chemin du descripteur se lit comme un fichier : le moteur reste celui de l'ordinateur.
                Bridge.pick(contentResolver.openFileDescriptor(it, "r"))?.let { path ->
                    send(Picked(path))
                }
            }
        }

    private fun send(message: ToRust) = Bridge.send(message)

    /** Une réponse de Rust, sur le fil du pont : seul l'écran vivant la reçoit. */
    private val answered: (String) -> Unit = { raw -> show(raw) }

    private fun show(raw: String) {
        if (raw == lastAnswer && raw.contains(NO_EFFECT)) return
        lastAnswer = raw
        // Frontière avec Rust : une réponse illisible serait un défaut de protocole, pas une raison de tuer
        // l'application ; elle est journalisée et l'écran garde l'état précédent.
        val answer =
            runCatching { Protocol.answer(raw) }
                .onFailure { Log.e(TAG, "unreadable answer from the native library", it) }
                .getOrElse { return }
        val stateText = answer.state.toString()
        val effects = answer.effects
        runOnUiThread {
            // Redessiner ou toucher à la fenêtre sans changement fait tressauter l'écran.
            if (stateText != screenText) {
                screenText = stateText
                // L'écran lit encore l'état champ par champ : sa forme suit les règles d'écran de Rust.
                val state = JSONObject(stateText)
                screen = state
                keepScreenOn(state.optBoolean("running"))
                // Le résultat reste lisible après coup, même une fois le panneau refermé.
                state.optJSONObject("panel")?.takeIf { it.optString("kind") == "finished" }?.let {
                    Log.i(TAG, it.optString("message"))
                }
            }
            effects.forEach(::accomplish)
        }
    }

    /** La clé branchée rouvre cette même fenêtre (`singleTask`) : seule la liste change. */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        revision++
        actions.enumerate()
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

    private fun accomplish(effect: Effect) {
        when (effect) {
            Enumerate -> actions.enumerate()
            LoadCatalog -> actions.catalog()
            PickImage -> pickFile.launch(arrayOf("*/*"))
            is Write -> actions.write(effect)
            is SaveLanguage -> getPreferences(MODE_PRIVATE).edit { putString("lang", effect.value) }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Fond clair : les icônes du système doivent rester sombres, quel que soit le thème du téléphone.
        val bars = SystemBarStyle.light(Tokens.PAPER.toInt(), Tokens.INK.toInt())
        enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
        actions =
            Actions(this, Bridge::send) { device, error ->
                runOnUiThread {
                    driveErrors = if (error == null) driveErrors - device else driveErrors + (device to error)
                }
            }
        Bridge.listener = answered
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
                            send(if (ticks == 0) Tick else Clock)
                        }
                    }
                    Screen(screen, usb, revision, driveErrors, ::send)
                }
            }
        }
    }

    override fun onDestroy() {
        unregisterReceiver(usbEvents)
        // Une écriture en cours continue dans le service ; l'écran suivant relira l'état auprès de Rust.
        if (Bridge.listener === answered) Bridge.listener = null
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
private fun Boxed(modifier: Modifier = Modifier, selected: Boolean = false, content: @Composable () -> Unit) {
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

@Composable
private fun Screen(
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
