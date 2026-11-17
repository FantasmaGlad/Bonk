package app.bonkos.bonk

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.StatFs
import android.provider.MediaStore
import androidx.annotation.RequiresApi
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.net.Proxy
import java.security.MessageDigest

/** Copies until the end or a cancel, and reports progress about every 16 MiB. */
internal fun copyReporting(
    input: InputStream,
    output: OutputStream,
    total: Long,
    digest: MessageDigest?,
    send: (ToRust) -> Unit,
): Long {
    val buffer = ByteArray(BUFFER)
    var done = 0L
    var reported = 0L
    val started = System.nanoTime()
    while (true) {
        if (Bridge.cancelled) throw BonkFailure("E4003", "cancelled during the download")
        val count = input.read(buffer)
        if (count < 0) break
        output.write(buffer, 0, count)
        digest?.update(buffer, 0, count)
        done += count
        if (done - reported > 16 * BUFFER) {
            reported = done
            val seconds = (System.nanoTime() - started) / 1e9
            // The system gives -1 for an unknown size: Rust then expects no total.
            send(Download(done, total.takeIf { it >= 0 }, done / seconds))
        }
    }
    return done
}

/**
 * A catalog image saved for the user, never written: in Downloads from Android 10, before that in the app's
 * own download folder, which needs no storage permission. The hash is checked while the bytes arrive.
 */
class ImageDownload(private val context: Context, private val send: (ToRust) -> Unit) {
    /** Blocks; the download service runs it. */
    fun run(request: DownloadImage) {
        Bridge.cancelled = false
        try {
            val connection = open(request.url, request.proxy?.let(::socks) ?: Proxy.NO_PROXY, compressed = false)
            val total = connection.contentLengthLong
            val target = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) Shared(request.name) else Private(request.name)
            if (total > 0 && target.free() <= total) {
                throw BonkFailure("E3008", "not enough free space on the phone: $total bytes needed")
            }
            val digest = MessageDigest.getInstance("SHA-256")
            val bytes =
                try {
                    target.output().use { output -> connection.inputStream.use { copyReporting(it, output, total, digest, send) } }
                } catch (failure: Exception) {
                    target.discard()
                    throw failure
                }
            val actual = digest.digest().joinToString("") { "%02x".format(it) }
            if (!actual.equals(request.sha256, ignoreCase = true)) {
                target.discard()
                throw BonkFailure("E3005", "SHA-256 $actual, the catalog gives ${request.sha256}; the file was removed")
            }
            send(Downloaded(target.publish(), bytes))
        } catch (error: BonkFailure) {
            send(Failed(error.message.orEmpty(), error.code))
        } catch (error: IOException) {
            send(Failed(error.message ?: error.toString(), "E3009"))
        } catch (error: Exception) {
            send(Failed(error.message ?: error.toString()))
        }
    }

    private interface Target {
        fun free(): Long

        fun output(): OutputStream

        fun discard()

        /** Where the user finds it, as the phone names it. */
        fun publish(): String
    }

    /** Hidden from other apps until complete, so a half image is never offered. */
    @RequiresApi(Build.VERSION_CODES.Q)
    private inner class Shared(private val name: String) : Target {
        private val resolver = context.contentResolver
        private var uri: Uri? = null

        override fun free(): Long = StatFs((context.getExternalFilesDir(null) ?: context.filesDir).path).availableBytes

        override fun output(): OutputStream {
            val values =
                ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, name)
                    put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
                    put(MediaStore.Downloads.IS_PENDING, 1)
                }
            val created =
                resolver.insert(MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY), values)
                    ?: throw IOException("Downloads refused the file")
            uri = created
            return resolver.openOutputStream(created) ?: throw IOException("Downloads gave no stream")
        }

        override fun discard() {
            uri?.let { resolver.delete(it, null, null) }
        }

        override fun publish(): String {
            val created = uri ?: return name
            resolver.update(created, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            // The system renames a second copy: the name it kept is the one to show.
            val shown =
                resolver.query(created, arrayOf(MediaStore.Downloads.DISPLAY_NAME), null, null, null)?.use { row ->
                    if (row.moveToFirst()) row.getString(0) else null
                }
            return "${Environment.DIRECTORY_DOWNLOADS}/${shown ?: name}"
        }
    }

    private inner class Private(private val name: String) : Target {
        private val file = File(context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: context.filesDir, name)

        override fun free(): Long = StatFs(file.parentFile?.path ?: context.filesDir.path).availableBytes

        override fun output(): OutputStream = file.outputStream()

        override fun discard() {
            file.delete()
        }

        override fun publish(): String = file.path
    }
}
