package app.bonkos.bonk

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.os.Build
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

const val ACTION_PERMISSION = "app.bonkos.bonk.USB_PERMISSION"

/** One conversation at a time: two interleaved exchanges steal each other's answers. */
private val usbAccess = ReentrantLock()

private const val SUBCLASS_SCSI = 6
private const val PROTOCOL_BULK_ONLY = 0x50

private fun UsbDevice.storageInterface(): UsbInterface? =
    (0 until interfaceCount).map(::getInterface).firstOrNull {
        it.interfaceClass == UsbConstants.USB_CLASS_MASS_STORAGE &&
            it.interfaceSubclass == SUBCLASS_SCSI &&
            it.interfaceProtocol == PROTOCOL_BULK_ONLY
    }

/** The phone's internal storage is not a USB device: it cannot appear here. */
fun UsbManager.drives(): List<UsbDevice> = deviceList.values.filter { it.storageInterface() != null }

fun UsbManager.askPermission(context: Context, device: UsbDevice) {
    // The system adds its answer to it: the intent must stay mutable, and explicit to be allowed.
    val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
    val intent = Intent(ACTION_PERMISSION).setPackage(context.packageName)
    requestPermission(device, PendingIntent.getBroadcast(context, 0, intent, flags))
}

fun <T> UsbManager.withDrive(device: UsbDevice, work: (fd: Int, descriptors: ByteArray) -> T): T =
    usbAccess.withLock {
        val connection = openDevice(device) ?: throw BonkFailure("E4004", "the system refused to open the drive")
        try {
            val storage = device.storageInterface() ?: throw BonkFailure("E4004", "no storage interface on the drive")
            if (!connection.claimInterface(storage, true)) throw BonkFailure("E4004", "the system kept the drive")
            try {
                work(connection.fileDescriptor, connection.rawDescriptors)
            } finally {
                connection.releaseInterface(storage)
            }
        } finally {
            connection.close()
        }
    }
