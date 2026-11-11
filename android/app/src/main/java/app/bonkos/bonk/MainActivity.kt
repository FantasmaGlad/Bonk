package app.bonkos.bonk

import android.Manifest
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.hardware.usb.UsbManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.util.Log
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import java.util.Locale
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val TAG = "Bonk"
private const val CLOCK_MS = 500L
private const val TICKS_PER_REFRESH = 4
private const val NO_EFFECT = "\"effects\":[]"

class MainActivity : ComponentActivity() {
    private var revision by mutableIntStateOf(0)
    private var screen by mutableStateOf(ScreenState())
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

    /** Android 13+ hides the write notification without this permission; the write does not depend on it. */
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    private val pickFile =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            uri?.let {
                // The descriptor path reads like a file: the engine stays the desktop one.
                Bridge.pick(contentResolver.openFileDescriptor(it, "r"))?.let { path ->
                    send(Picked(path))
                }
            }
        }

    private val pickVaultIso =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            uri?.let {
                Bridge.pick(contentResolver.openFileDescriptor(it, "r"), displayName(it))?.let { path ->
                    send(VaultPicked(path))
                }
            }
        }

    /** The name the system shows: the descriptor path the engine reads carries none. */
    private fun displayName(uri: Uri): String? =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0) else null
        }

    private fun send(message: ToRust) = Bridge.send(message)

    private val answered: (String) -> Unit = { raw -> show(raw) }

    private fun show(raw: String) {
        if (raw == lastAnswer && raw.contains(NO_EFFECT)) return
        lastAnswer = raw
        // An unreadable answer is logged; the screen keeps the previous state.
        val answer =
            runCatching { Protocol.answer(raw) }
                .onFailure { Log.e(TAG, "unreadable answer from the native library", it) }
                .getOrElse { return }
        val state = answer.state
        runOnUiThread {
            // Redrawing or touching the window without a change makes the screen flicker.
            if (state != screen) {
                screen = state
                Bridge.language = state.lang
                keepScreenOn(state.running)
                // The result stays readable afterwards, even once the panel is closed.
                (state.panel as? FinishedPanel)?.let { Log.i(TAG, it.message) }
            }
            answer.effects.forEach(::accomplish)
        }
    }

    /** A plugged key reopens this same window (`singleTask`): only the list changes. */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        revision++
        actions.enumerate()
        openLink(intent)
    }

    /** Once per link: a screen rebuilt later must not choose the image again over the user's own choice. */
    private fun openLink(intent: Intent) {
        if (intent.action != Intent.ACTION_VIEW) return
        intent.dataString?.let { send(Open(it)) }
        intent.data = null
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

    private fun askNotificationsIfNeeded() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    private fun accomplish(effect: Effect) {
        when (effect) {
            Enumerate -> actions.enumerate()
            LoadCatalog -> actions.catalog()
            PickImage -> pickFile.launch(arrayOf("*/*"))
            is Write -> {
                askNotificationsIfNeeded()
                actions.write(effect)
            }
            is SaveLanguage -> getPreferences(MODE_PRIVATE).edit { putString("lang", effect.value) }
            PickVaultIso -> pickVaultIso.launch(arrayOf("*/*"))
            is VaultRun -> {
                if (effect.action != "list") askNotificationsIfNeeded()
                actions.vault(effect)
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Light background: system icons must stay dark whatever the phone theme.
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
        openLink(intent)
        // In the background the clock would wake Rust and USB for nothing.
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                var ticks = 0
                while (true) {
                    delay(CLOCK_MS)
                    ticks = (ticks + 1) % TICKS_PER_REFRESH
                    send(if (ticks == 0) Tick else Clock)
                }
            }
        }
        setContent {
            BonkTheme {
                Surface(Modifier.fillMaxSize(), color = Color(Tokens.PAPER)) {
                    val localized = remember(screen.lang) { inLanguage(screen.lang) }
                    CompositionLocalProvider(
                        LocalContext provides localized,
                        LocalConfiguration provides localized.resources.configuration,
                    ) {
                        Screen(screen, usb, revision, driveErrors, ::send)
                    }
                }
            }
        }
    }

    override fun onDestroy() {
        unregisterReceiver(usbEvents)
        // A running write goes on in the service; the next screen rereads the state from Rust.
        if (Bridge.listener === answered) Bridge.listener = null
        super.onDestroy()
    }
}
