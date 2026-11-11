package app.bonkos.bonk

import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.res.stringResource
import kotlin.concurrent.thread

private const val TAG = "Bonk"
private const val MEBIBYTE = 1_048_576

/** Development-only tests: they tell whether writing works on this device. */
@Composable
internal fun Tests(usb: UsbManager, revision: Int) {
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
        if (report.isNotEmpty()) Small(report, LocalPalette.current.ink)
    }
}
