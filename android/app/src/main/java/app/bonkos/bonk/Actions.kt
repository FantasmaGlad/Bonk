package app.bonkos.bonk

import android.content.Context
import android.hardware.usb.UsbManager
import kotlin.concurrent.thread

class Actions(
    private val context: Context,
    send: (ToRust) -> Unit,
    driveError: (String, String?) -> Unit,
) {
    private val drives = DriveProbe(context.getSystemService(UsbManager::class.java), send, driveError)
    private val catalogs = CatalogSync(context, send)
    private val operations = Operations(context, send)

    /** An allowed key is probed once: probing takes it from the system for a moment. */
    fun enumerate() = drives.enumerate()

    fun catalog(proxy: String?) = catalogs.catalog(proxy)

    /** The write belongs to the service: closing the screen does not stop it. */
    fun write(request: Write) {
        Bridge.cancelled = false
        WriteService.start(context, request)
    }

    fun vault(request: VaultRun) {
        Bridge.cancelled = false
        if (request.action == "list") {
            thread { operations.runVault(request) }
        } else {
            WriteService.start(context, request)
        }
    }
}
