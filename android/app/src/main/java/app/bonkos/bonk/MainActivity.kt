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
import android.util.Log
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Surface
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
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

    /** Android 13 et plus masquent la notification de l'écriture sans cet accord ; l'écriture n'en dépend pas. */
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

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
        val state = answer.state
        runOnUiThread {
            // Redessiner ou toucher à la fenêtre sans changement fait tressauter l'écran.
            if (state != screen) {
                screen = state
                keepScreenOn(state.running)
                // Le résultat reste lisible après coup, même une fois le panneau refermé.
                (state.panel as? FinishedPanel)?.let { Log.i(TAG, it.message) }
            }
            answer.effects.forEach(::accomplish)
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
            is Write -> {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                    checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
                ) {
                    askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                }
                actions.write(effect)
            }
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
        // L'horloge et la détection ne tournent que l'écran visible : en arrière-plan, elles réveilleraient Rust
        // et l'USB pour rien. L'écriture, elle, vit dans le service.
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
