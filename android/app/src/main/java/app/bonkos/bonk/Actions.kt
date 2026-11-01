package app.bonkos.bonk

import android.content.Context
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.util.Log
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import org.json.JSONArray
import org.json.JSONObject

private const val CATALOG = "https://catalog.bonk-os.app/v1/catalog.json"
private const val BUFFER = 1 shl 20
private const val RETRY_MS = 10_000L
private const val TAG = "Bonk"

private fun fetchOrEmpty(address: String): ByteArray =
    runCatching { URL(address).readBytes() }.getOrDefault(ByteArray(0))

/** Ce que Rust demande et que seul Android sait faire : voir l'USB, le réseau, les fichiers. */
class Actions(
    private val context: Context,
    private val send: (JSONObject) -> Unit,
    private val driveError: (String, String?) -> Unit,
) {
    private val usb = context.getSystemService(UsbManager::class.java)
    private val sizes = HashMap<String, Long>()
    private val failedAt = HashMap<String, Long>()
    private val enumerating = AtomicBoolean(false)

    @Volatile var cancelled = false

    private fun size(device: UsbDevice): Long? {
        sizes[device.deviceName]?.let { return it }
        // Réessayer sans répit retirerait sans cesse la clé au système.
        val failed = failedAt[device.deviceName]
        if (failed != null && System.currentTimeMillis() - failed < RETRY_MS) return null
        return runCatching {
            val answer = JSONObject(usb.withDrive(device) { fd, raw -> Native.probe(fd, raw) })
            check(!answer.has("error")) { answer.getString("error") }
            answer.getLong("blocks") * answer.getLong("block_size")
        }.onSuccess {
            sizes[device.deviceName] = it
            failedAt.remove(device.deviceName)
            driveError(device.deviceName, null)
        }.onFailure {
            Log.w(TAG, "${device.deviceName}: $it")
            failedAt[device.deviceName] = System.currentTimeMillis()
            driveError(device.deviceName, it.message ?: it.toString())
        }.getOrNull()
    }

    /** Une clé autorisée n'est interrogée qu'une fois : l'interroger la retire un instant au système. */
    fun enumerate() {
        if (!enumerating.compareAndSet(false, true)) return
        thread {
            try {
                val disks = JSONArray()
                val drives = usb.drives()
                drives.filter(usb::hasPermission).forEach { device ->
                    size(device)?.let {
                        disks.put(
                            JSONObject()
                                .put("device", device.deviceName)
                                .put("model", "${device.manufacturerName.orEmpty()} ${device.productName.orEmpty()}".trim())
                                .put("serial", device.serialNumber.orEmpty())
                                .put("size_bytes", it),
                        )
                    }
                }
                val present = drives.map { it.deviceName }.toSet()
                sizes.keys.retainAll(present)
                failedAt.keys.retainAll(present)
                send(JSONObject().put("type", "disks").put("disks", disks))
            } finally {
                enumerating.set(false)
            }
        }
    }

    fun catalog() {
        thread {
            val answer =
                JSONObject(
                    Native.loadCatalog(
                        File(context.filesDir, "catalog").path,
                        fetchOrEmpty(CATALOG),
                        fetchOrEmpty("$CATALOG.sig"),
                    ),
                )
            val message = JSONObject()
            if (answer.has("error")) {
                message.put("type", "catalog_failed").put("message", answer.getString("error"))
            }
            send(message)
        }
    }

    private fun download(address: String): File {
        val target = File(context.cacheDir, "image.download")
        val connection = URL(address).openConnection() as HttpURLConnection
        val total = connection.contentLengthLong
        check(context.cacheDir.usableSpace > total) { "not enough free space on the phone: $total bytes needed" }
        var done = 0L
        var reported = 0L
        val started = System.nanoTime()
        connection.inputStream.use { input ->
            target.outputStream().use { output ->
                val buffer = ByteArray(BUFFER)
                while (true) {
                    check(!cancelled) { "cancelled" }
                    val count = input.read(buffer)
                    if (count < 0) break
                    output.write(buffer, 0, count)
                    done += count
                    if (done - reported > 16 * BUFFER) {
                        reported = done
                        val seconds = (System.nanoTime() - started) / 1e9
                        send(
                            JSONObject()
                                .put("type", "download")
                                .put("done", done)
                                .put("total", total)
                                .put("rate", done / seconds),
                        )
                    }
                }
            }
        }
        return target
    }

    fun write(request: JSONObject) {
        cancelled = false
        WriteService.start(context)
        thread {
            var downloaded: File? = null
            try {
                val device =
                    usb.drives().first { it.deviceName == request.getString("device") }
                val image = request.getString("image")
                val hash = request.getString("hash")
                check(!image.startsWith("http") || hash.isNotEmpty()) { "an address needs a SHA-256 hash" }
                val source =
                    if (image.startsWith("http")) {
                        download(image).also { downloaded = it }.path
                    } else {
                        image
                    }
                usb.withDrive(device) { fd, raw ->
                    Native.write(
                        fd,
                        raw,
                        request.getString("disk_id"),
                        source,
                        hash,
                        request.getBoolean("raw"),
                        request.getBoolean("mbr"),
                    )
                }
                send(JSONObject())
            } catch (error: Exception) {
                send(JSONObject().put("type", "failed").put("message", error.message ?: error.toString()))
            } finally {
                downloaded?.delete()
                WriteService.stop(context)
            }
        }
    }
}
