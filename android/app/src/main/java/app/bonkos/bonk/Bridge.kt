package app.bonkos.bonk

import android.os.ParcelFileDescriptor
import android.util.Log
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/**
 * Le lien avec Rust, à l'échelle du processus et non de l'écran : une écriture continue quand on ferme
 * l'écran, et ses messages ne doivent jamais viser un écran détruit.
 */
object Bridge {
    /** Un seul fil pour Rust et le JSON, hors du fil de l'écran : un clic n'attend jamais le battement précédent. */
    private val worker = Executors.newSingleThreadExecutor()

    /** L'écran qui affiche les réponses ; aucun pendant qu'il est fermé, et Rust garde l'état. */
    @Volatile var listener: ((String) -> Unit)? = null

    /** Langue choisie dans Bonk, telle que Rust la rend ; le service y écrit sa notification. */
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
            // Une panique de Rust revient en exception : non rattrapée, elle tuerait l'application. Rust reprend
            // son état au message suivant ; celui-ci est seulement perdu, et journalisé.
            runCatching { Native.dispatch(text) }
                .onSuccess { answer -> listener?.invoke(answer) }
                .onFailure { Log.e("Bonk", "native dispatch failed", it) }
        }
    }

    /**
     * L'image choisie se lit par le chemin de son descripteur : il reste ouvert jusqu'au choix suivant, même
     * si l'écran qui l'a choisie a été détruit entre-temps.
     */
    @Synchronized
    fun pick(file: ParcelFileDescriptor?): String? {
        picked?.close()
        picked = file
        return file?.let { "/proc/self/fd/${it.fd}" }
    }
}
