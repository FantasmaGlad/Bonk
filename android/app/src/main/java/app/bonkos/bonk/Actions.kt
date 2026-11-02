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

private const val CATALOG = "https://catalog.bonk-os.app/v1/catalog.json"
private const val BUFFER = 1 shl 20
private const val RETRY_MS = 10_000L
private const val TAG = "Bonk"

private fun fetchOrEmpty(address: String): ByteArray =
    runCatching { URL(address).readBytes() }.getOrDefault(ByteArray(0))

/** Ce que Rust demande et que seul Android sait faire : voir l'USB, le réseau, les fichiers. */
class Actions(
    private val context: Context,
    private val send: (ToRust) -> Unit,
    private val driveError: (String, String?) -> Unit,
) {
    private val usb = context.getSystemService(UsbManager::class.java)
    /** Réponse de la sonde par clé : taille et partitions. */
    private val probes = HashMap<String, Probe>()
    private val failedAt = HashMap<String, Long>()
    private val enumerating = AtomicBoolean(false)

    private fun probe(device: UsbDevice): Probe? {
        // Une clé écrite depuis la dernière sonde a changé de table : la relire.
        if (Bridge.written.remove(device.deviceName)) probes.remove(device.deviceName)
        probes[device.deviceName]?.let { return it }
        // Réessayer sans répit retirerait sans cesse la clé au système.
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

    /** Une clé autorisée n'est interrogée qu'une fois : l'interroger la retire un instant au système. */
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

    fun catalog() {
        thread {
            val answer =
                Protocol.outcome(
                    Native.loadCatalog(
                        File(context.filesDir, "catalog").path,
                        fetchOrEmpty(CATALOG),
                        fetchOrEmpty("$CATALOG.sig"),
                    ),
                )
            send(answer.error?.let(::CatalogFailed) ?: Refresh)
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
                    check(!Bridge.cancelled) { "cancelled" }
                    val count = input.read(buffer)
                    if (count < 0) break
                    output.write(buffer, 0, count)
                    done += count
                    if (done - reported > 16 * BUFFER) {
                        reported = done
                        val seconds = (System.nanoTime() - started) / 1e9
                        // Une taille inconnue vaut -1 pour le système : Rust attend alors l'absence de total.
                        send(Download(done, total.takeIf { it >= 0 }, done / seconds))
                    }
                }
            }
        }
        return target
    }

    /** L'écriture appartient au service : fermer l'écran ne l'interrompt pas. */
    fun write(request: Write) {
        Bridge.cancelled = false
        WriteService.start(context, request)
    }

    /** Écrit et vérifie, de bout en bout. Bloque : appelé par le service, hors du fil de l'écran. */
    fun runWrite(request: Write) {
        var downloaded: File? = null
        try {
            val device =
                usb.drives().firstOrNull { it.deviceName == request.device }
                    ?: error("the drive is no longer connected")
            val image = request.image
            val hash = request.hash
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
                    request.diskId,
                    device.deviceName,
                    device.serialNumber.orEmpty(),
                    source,
                    hash,
                    request.raw,
                    request.mbr,
                )
            }
            send(Refresh)
        } catch (error: Exception) {
            send(Failed(error.message ?: error.toString()))
        } finally {
            downloaded?.delete()
        }
    }
}
