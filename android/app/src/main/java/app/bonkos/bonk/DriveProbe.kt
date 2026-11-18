package app.bonkos.bonk

import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

private const val RETRY_MS = 10_000L
private const val TAG = "Bonk"

internal class DriveProbe(
    private val usb: UsbManager,
    private val send: (ToRust) -> Unit,
    private val driveError: (String, String?) -> Unit,
) {
    private val probes = HashMap<String, Probe>()
    private val failedAt = HashMap<String, Long>()
    private val enumerating = AtomicBoolean(false)

    private fun probe(device: UsbDevice): Probe? {
        // A key written since the last probe has a new table: probe it again.
        if (Bridge.written.remove(device.deviceName)) probes.remove(device.deviceName)
        probes[device.deviceName]?.let { return it }
        // Retrying without pause would keep taking the key from the system.
        val failed = failedAt[device.deviceName]
        if (failed != null && System.currentTimeMillis() - failed < RETRY_MS) return null
        return runCatching {
            val answer = Protocol.probe(usb.withDrive(device) { fd, raw -> Native.probe(fd, raw) })
            check(answer.error == null) { answer.error.orEmpty() }
            answer
        }.onSuccess {
            probes[device.deviceName] = it
            failedAt.remove(device.deviceName)
            driveError(device.deviceName, null)
        }.onFailure {
            Log.w(TAG, "${device.deviceName}: $it")
            failedAt[device.deviceName] = System.currentTimeMillis()
            driveError(device.deviceName, it.message ?: it.toString())
        }.getOrNull()
    }

    fun enumerate() {
        if (!enumerating.compareAndSet(false, true)) return
        thread {
            try {
                val drives = usb.drives()
                val disks =
                    drives.filter(usb::hasPermission).mapNotNull { device ->
                        probe(device)?.let {
                            Drive(
                                device = device.deviceName,
                                model = "${device.manufacturerName.orEmpty()} ${device.productName.orEmpty()}".trim(),
                                serial = device.serialNumber.orEmpty(),
                                sizeBytes = it.sizeBytes,
                                partitions = it.partitions,
                            )
                        }
                    }
                val present = drives.map { it.deviceName }.toSet()
                probes.keys.retainAll(present)
                failedAt.keys.retainAll(present)
                send(Disks(disks))
            } finally {
                enumerating.set(false)
            }
        }
    }
}
