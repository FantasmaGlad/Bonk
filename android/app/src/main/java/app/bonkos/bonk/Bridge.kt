package app.bonkos.bonk

import android.os.ParcelFileDescriptor
import android.util.Log
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/** Process-wide: a write outlives the screen, and its messages never target a destroyed screen. */
object Bridge {
    /** One thread for Rust and JSON, off the UI thread: a click never waits for the previous tick. */
    private val worker = Executors.newSingleThreadExecutor()

    @Volatile var listener: ((String) -> Unit)? = null

    @Volatile var language = "en"

    /** Read by the download, which Rust does not see; Rust stops the write on its side. */
    @Volatile var cancelled = false

    /** Keys touched by a write: their cached probe describes a table that no longer exists. */
    val written: MutableSet<String> = ConcurrentHashMap.newKeySet()

    private var picked: ParcelFileDescriptor? = null

    private var pickedName: String? = null

    fun send(message: ToRust) {
        if (message == Cancel) cancelled = true
        val text = Protocol.encode(message)
        worker.execute {
            // A Rust panic comes back as an exception: caught, only this message is lost.
            runCatching { Native.dispatch(text) }
                .onSuccess { answer -> listener?.invoke(answer) }
                .onFailure { Log.e("Bonk", "native dispatch failed", it) }
        }
    }

    /** Open until the next choice, even if the screen that chose it is destroyed. */
    @Synchronized
    fun pick(file: ParcelFileDescriptor?, name: String? = null): String? {
        picked?.close()
        picked = file
        pickedName = name
        return file?.let { "/proc/self/fd/${it.fd}" }
    }

    /** The descriptor path has no name: the vault names the ISO as the system showed it. */
    @Synchronized
    fun nameOf(path: String): String = pickedName.takeIf { picked?.let { "/proc/self/fd/${it.fd}" } == path }.orEmpty()
}
