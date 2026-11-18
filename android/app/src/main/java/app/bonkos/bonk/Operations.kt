package app.bonkos.bonk

import android.content.Context
import android.hardware.usb.UsbManager
import android.os.storage.StorageManager
import java.io.File
import java.io.IOException
import java.net.Proxy

internal const val BUFFER = 1 shl 20

/** What writes to a key, run by the service so that closing the screen does not stop it. */
internal class Operations(private val context: Context, private val send: (ToRust) -> Unit) {
    private val usb = context.getSystemService(UsbManager::class.java)

    private fun download(address: String, proxy: String?): File {
        val target = File(context.cacheDir, "image.download")
        val connection = open(address, proxy?.let(::socks) ?: Proxy.NO_PROXY, compressed = false)
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

    /** Rust checks the bytes against the signed catalog; the bound only stops a runaway download. */
    private fun downloadGrub(grub: GrubDownload, folder: File, proxy: String?) {
        val target = File(folder, grub.arch)
        open(grub.url, proxy?.let(::socks) ?: Proxy.NO_PROXY).inputStream.use { input ->
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
                        images.forEach { downloadGrub(it, this, request.proxy) }
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
