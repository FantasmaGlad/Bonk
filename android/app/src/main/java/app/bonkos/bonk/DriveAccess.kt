package app.bonkos.bonk

import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource

@Composable
internal fun WaitingDrives(state: ScreenState, usb: UsbManager, revision: Int, errors: Map<String, String>) {
    val waiting = remember(revision, errors) { usb.drives().filterNot(usb::hasPermission) }
    waiting.forEach { device -> AskAccess(usb, device) }
    errors.forEach { (device, error) ->
        Boxed {
            Body(stringResource(R.string.drive_unreadable, device))
            Small(error, LocalPalette.current.ink)
        }
    }
    if (state.disks.isEmpty() && waiting.isEmpty() && errors.isEmpty()) {
        Body(state.empty)
        if (onChromeOs()) Small(stringResource(R.string.chromeos_share))
    }
}

/** ChromeOS hands a USB drive to its Android system only once the user shares it, and never asks. */
@Composable
private fun onChromeOs(): Boolean {
    val packages = LocalContext.current.packageManager
    return remember { packages.hasSystemFeature(ARC_FEATURE) }
}

private const val ARC_FEATURE = "org.chromium.arc"

@Composable
private fun AskAccess(usb: UsbManager, device: UsbDevice) {
    val context = LocalContext.current
    Boxed {
        Body("${device.manufacturerName.orEmpty()} ${device.productName.orEmpty()}")
        Small(stringResource(R.string.allow_why))
        Primary(stringResource(R.string.allow)) { usb.askPermission(context, device) }
    }
}
