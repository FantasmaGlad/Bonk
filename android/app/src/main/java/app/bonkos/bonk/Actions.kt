package app.bonkos.bonk

import android.content.Context
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.storage.StorageManager
import android.util.Log
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.InetSocketAddress
import java.net.Proxy
import java.net.URI
import java.net.URL
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

private const val CATALOG = "https://catalog.bonk-os.app/v2/catalog.json"
internal const val BUFFER = 1 shl 20
private const val RETRY_MS = 10_000L
private const val TAG = "Bonk"
private const val ICON_WORKERS = 8
private const val ICONS_TIMEOUT_S = 120L

// Without a timeout a silent connection would block the download and its cancel.
private const val CONNECT_TIMEOUT_MS = 30_000
private const val READ_TIMEOUT_MS = 60_000

internal fun open(address: String, proxy: Proxy = Proxy.NO_PROXY): HttpURLConnection =
    (URL(address).openConnection(proxy) as HttpURLConnection).apply {
        connectTimeout = CONNECT_TIMEOUT_MS
        readTimeout = READ_TIMEOUT_MS
    }

/** Rust only hands over `socks5h://` addresses: anything else is refused, never a direct connection. */
internal fun socks(address: String): Proxy {
    val uri = URI(address)
    if (uri.scheme != "socks5h" || uri.host == null || uri.port < 0) {
        throw BonkFailure("E3008", "$address: only socks5h:// is accepted")
    }
    return Proxy(Proxy.Type.SOCKS, InetSocketAddress(uri.host, uri.port))
}

/** The catalog keeps its last verified copy when the network fails: nothing here may prevent it. */
private fun fetchOrEmpty(address: String): ByteArray =
    runCatching { open(address).inputStream.use { it.readBytes() } }.getOrDefault(ByteArray(0))

class Actions(
    private val context: Context,
    private val send: (ToRust) -> Unit,
    private val driveError: (String, String?) -> Unit,
) {
    private val usb = context.getSystemService(UsbManager::class.java)
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

    /** An allowed key is probed once: probing takes it from the system for a moment. */
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
            val directory = File(context.filesDir, "catalog").path
            val answer =
                runCatching {
                    Protocol.outcome(Native.loadCatalog(directory, fetchOrEmpty(CATALOG), fetchOrEmpty("$CATALOG.sig")))
                }.getOrElse { Outcome(error = it.message ?: it.toString()) }
            send(answer.error?.let(::CatalogFailed) ?: Refresh)
            if (answer.error == null) icons(directory)
        }
    }

    /**
     * After the list: the cards show at once, their logos as they arrive, [ICON_WORKERS] at a time. One by one,
     * a cold cache took about 20 s. A logo that fails stays absent.
     */
    private fun icons(directory: String) {
        val wanted = runCatching { Protocol.icons(Native.missingIcons(directory)) }.getOrDefault(emptyList())
        if (wanted.isEmpty()) return
        val pool = Executors.newFixedThreadPool(ICON_WORKERS)
        val stored = AtomicInteger()
        wanted.forEach { icon ->
            pool.execute {
                val bytes = fetchOrEmpty(icon.url)
                if (bytes.isNotEmpty()) {
                    Protocol.outcome(Native.storeIcon(directory, icon.sha256, bytes)).error?.let { Log.w(TAG, it) }
                }
                if (stored.incrementAndGet() % ICON_WORKERS == 0) send(Refresh)
            }
        }
        pool.shutdown()
        pool.awaitTermination(ICONS_TIMEOUT_S, TimeUnit.SECONDS)
        send(Refresh)
    }

    private fun download(address: String, proxy: String?): File {
        val target = File(context.cacheDir, "image.download")
        val connection = open(address, proxy?.let(::socks) ?: Proxy.NO_PROXY)
        val total = connection.contentLengthLong
        // allocateBytes clears system caches rather than refuse an image that would fit.
        val storage = context.getSystemService(StorageManager::class.java)
        val volume = storage.getUuidForPath(context.cacheDir)
        if (storage.getAllocatableBytes(volume) <= total) {
            throw BonkFailure("E3008", "not enough free space on the phone: $total bytes needed")
        }
        if (total > 0) storage.allocateBytes(volume, total)
        connection.inputStream.use { input ->
            target.outputStream().use { output -> copyReporting(input, output, total, null, send) }
        }
        return target
    }

    /** The write belongs to the service: closing the screen does not stop it. */
    fun write(request: Write) {
        Bridge.cancelled = false
        WriteService.start(context, request)
    }

    /** Reading a vault is brief; what writes to the key belongs to the service, as a write does. */
    fun vault(request: VaultRun) {
        Bridge.cancelled = false
        if (request.action == "list") {
            thread { runVault(request) }
        } else {
            WriteService.start(context, request)
        }
    }

    /** Rust checks the bytes against the signed catalog; the bound only stops a runaway download. */
    private fun downloadGrub(grub: GrubDownload, folder: File) {
        val target = File(folder, grub.arch)
        open(grub.url).inputStream.use { input ->
            target.outputStream().use { output ->
                val buffer = ByteArray(BUFFER)
                var done = 0L
                while (true) {
                    if (Bridge.cancelled) throw BonkFailure("E4003", "cancelled during the download")
                    val count = input.read(buffer)
                    if (count < 0) break
                    done += count
                    if (done > grub.sizeBytes) throw BonkFailure("E3009", "GRUB is larger than the catalog says")
                    output.write(buffer, 0, count)
                }
            }
        }
    }

    fun runVault(request: VaultRun) {
        var grub: File? = null
        try {
            val device =
                usb.drives().firstOrNull { it.deviceName == request.device }
                    ?: throw BonkFailure("E1003", "the drive is no longer connected")
            grub =
                request.grub.takeIf { it.isNotEmpty() }?.let { images ->
                    // One file per processor, named by it: Rust reads the folder.
                    File(context.cacheDir, "grub").apply {
                        deleteRecursively()
                        mkdirs()
                        images.forEach { downloadGrub(it, this) }
                    }
                }
            val iso = request.iso.orEmpty()
            usb.withDrive(device) { fd, raw ->
                Native.vault(
                    fd,
                    raw,
                    request.action,
                    request.diskId,
                    device.deviceName,
                    device.serialNumber.orEmpty(),
                    iso,
                    Bridge.nameOf(iso),
                    grub?.path.orEmpty(),
                )
            }
            send(Refresh)
        } catch (error: BonkFailure) {
            send(Failed(error.message.orEmpty(), error.code))
        } catch (error: IOException) {
            // Network cut, timeout: only the GRUB download does I/O here.
            send(Failed(error.message ?: error.toString(), "E3009"))
        } catch (error: Exception) {
            send(Failed(error.message ?: error.toString()))
        } finally {
            grub?.deleteRecursively()
        }
    }

    fun runWrite(request: Write) {
        var downloaded: File? = null
        try {
            val device =
                usb.drives().firstOrNull { it.deviceName == request.device }
                    ?: throw BonkFailure("E1003", "the drive is no longer connected")
            val image = request.image
            val hash = request.hash
            if (image.startsWith("http") && hash.isEmpty()) throw BonkFailure("E0001", "an address needs a SHA-256 hash")
            val source =
                if (image.startsWith("http")) {
                    download(image, request.proxy).also { downloaded = it }.path
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
        } catch (error: BonkFailure) {
            send(Failed(error.message.orEmpty(), error.code))
        } catch (error: IOException) {
            // Network cut, timeout: only the download does I/O here.
            send(Failed(error.message ?: error.toString(), "E3008"))
        } catch (error: Exception) {
            send(Failed(error.message ?: error.toString()))
        } finally {
            downloaded?.delete()
        }
    }
}
