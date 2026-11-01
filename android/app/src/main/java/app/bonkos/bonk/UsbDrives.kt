package app.bonkos.bonk

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.os.Build

const val ACTION_PERMISSION = "app.bonkos.bonk.USB_PERMISSION"

private const val SUBCLASS_SCSI = 6
private const val PROTOCOL_BULK_ONLY = 0x50

private fun UsbDevice.storageInterface(): UsbInterface? =
    (0 until interfaceCount).map(::getInterface).firstOrNull {
        it.interfaceClass == UsbConstants.USB_CLASS_MASS_STORAGE &&
            it.interfaceSubclass == SUBCLASS_SCSI &&
            it.interfaceProtocol == PROTOCOL_BULK_ONLY
    }

/** Le stockage interne du téléphone n'est pas un périphérique USB : il ne peut pas apparaître ici. */
fun UsbManager.drives(): List<UsbDevice> = deviceList.values.filter { it.storageInterface() != null }

fun UsbManager.askPermission(context: Context, device: UsbDevice) {
    // Le système y ajoute sa réponse : l'intention doit rester modifiable, et explicite pour y avoir droit.
    val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
    val intent = Intent(ACTION_PERMISSION).setPackage(context.packageName)
    requestPermission(device, PendingIntent.getBroadcast(context, 0, intent, flags))
}

/** Réclame la clé, en détachant le pilote du système, le temps de `work`. */
fun <T> UsbManager.withDrive(device: UsbDevice, work: (fd: Int, descriptors: ByteArray) -> T): T {
    val connection = openDevice(device) ?: error("openDevice refused")
    try {
        val storage = device.storageInterface() ?: error("no storage interface")
        check(connection.claimInterface(storage, true)) { "claimInterface refused" }
        try {
            return work(connection.fileDescriptor, connection.rawDescriptors)
        } finally {
            connection.releaseInterface(storage)
        }
    } finally {
        connection.close()
    }
}
