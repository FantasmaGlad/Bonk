package app.bonkos.bonk

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import kotlin.concurrent.thread

private const val TAG = "Bonk"
private const val MEBIBYTE = 1_048_576

class MainActivity : ComponentActivity() {
    private var revision by mutableIntStateOf(0)

    private val usbEvents =
        object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                revision++
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Fond clair : les icônes du système doivent rester sombres, quel que soit le thème du téléphone.
        val bars = SystemBarStyle.light(Tokens.PAPER.toInt(), Tokens.INK.toInt())
        enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
        val filter =
            IntentFilter().apply {
                addAction(ACTION_PERMISSION)
                addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
                addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
            }
        ContextCompat.registerReceiver(this, usbEvents, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        val usb = getSystemService(UsbManager::class.java)
        setContent {
            BonkTheme {
                Surface(Modifier.fillMaxSize(), color = Color(Tokens.PAPER)) {
                    DriveScreen(usb, revision)
                }
            }
        }
    }

    override fun onDestroy() {
        unregisterReceiver(usbEvents)
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
            ),
        content = content,
    )
}

@Composable
private fun DriveScreen(usb: UsbManager, revision: Int) {
    Column(
        Modifier.safeDrawingPadding()
            .padding(Tokens.SPACE_PAGE.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_LARGE.dp),
    ) {
        Text("BONK", fontSize = Tokens.TEXT_TITLE.sp)
        val drives = remember(revision) { usb.drives() }
        if (drives.isEmpty()) {
            Text(stringResource(R.string.no_drive), fontSize = Tokens.TEXT_BODY.sp)
        }
        drives.forEach { DriveCard(usb, it) }
    }
}

@Composable
private fun DriveCard(usb: UsbManager, device: UsbDevice) {
    val context = androidx.compose.ui.platform.LocalContext.current
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

    OutlinedCard(
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        border = BorderStroke(1.dp, Color(Tokens.INK)),
        colors = CardDefaults.outlinedCardColors(containerColor = Color(Tokens.PAPER)),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            Modifier.padding(Tokens.SPACE_MEDIUM.dp),
            verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
        ) {
            Text(
                "${device.manufacturerName.orEmpty()} ${device.productName.orEmpty()}",
                fontSize = Tokens.TEXT_BODY.sp,
            )
            if (!usb.hasPermission(device)) {
                BonkButton(stringResource(R.string.allow)) { usb.askPermission(context, device) }
                return@Column
            }
            Text(device.serialNumber.orEmpty(), fontSize = Tokens.TEXT_SMALL.sp, color = Color(Tokens.SECONDARY))
            BonkButton(stringResource(R.string.read_test)) {
                run { fd, descriptors ->
                    Native.probe(fd, descriptors) + "\n" +
                        Native.readHash(fd, descriptors, 0, MEBIBYTE)
                }
            }
            if (BuildConfig.BENCH_SERIAL.isNotEmpty() && device.serialNumber == BuildConfig.BENCH_SERIAL) {
                Text(stringResource(R.string.write_warning), fontSize = Tokens.TEXT_SMALL.sp)
                BonkButton(stringResource(R.string.write_test)) {
                    run { fd, descriptors -> Native.writeBench(fd, descriptors) }
                }
            }
            if (report.isNotEmpty()) {
                Text(report, fontSize = Tokens.TEXT_SMALL.sp)
            }
        }
    }
}

@Composable
private fun BonkButton(label: String, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        shape = RoundedCornerShape(Tokens.RADIUS.dp),
        colors = ButtonDefaults.buttonColors(containerColor = Color(Tokens.INK), contentColor = Color(Tokens.PAPER)),
    ) {
        Text(label, fontSize = Tokens.TEXT_BODY.sp)
    }
}
