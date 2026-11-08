package app.bonkos.bonk

import android.os.ParcelFileDescriptor
import android.util.Log
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/** À l'échelle du processus : une écriture survit à l'écran, et ses messages ne visent jamais un écran détruit. */
object Bridge {
    /** Un seul fil pour Rust et le JSON, hors du fil de l'écran : un clic n'attend jamais le battement précédent. */
    private val worker = Executors.newSingleThreadExecutor()

    @Volatile var listener: ((String) -> Unit)? = null

    @Volatile var language = "en"

    /** Lu par le téléchargement, que Rust ne voit pas ; Rust arrête l'écriture de son côté. */
    @Volatile var cancelled = false

    /** Clés touchées par une écriture : leur sonde en cache décrit une table qui n'existe plus. */
    val written: MutableSet<String> = ConcurrentHashMap.newKeySet()

    private var picked: ParcelFileDescriptor? = null

    fun send(message: ToRust) {
        if (message == Cancel) cancelled = true
        val text = Protocol.encode(message)
        worker.execute {
            // Une panique de Rust revient en exception : rattrapée, seul ce message est perdu.
            runCatching { Native.dispatch(text) }
                .onSuccess { answer -> listener?.invoke(answer) }
                .onFailure { Log.e("Bonk", "native dispatch failed", it) }
        }
    }

    /** Ouvert jusqu'au choix suivant, même si l'écran qui l'a choisi est détruit. */
    @Synchronized
    fun pick(file: ParcelFileDescriptor?): String? {
        picked?.close()
        picked = file
        return file?.let { "/proc/self/fd/${it.fd}" }
    }
}
