package app.bonkos.bonk

/** La bibliothèque Rust : le même modèle d'écran et le même moteur d'écriture que sur ordinateur. */
object Native {
    init {
        System.loadLibrary("bonk_android")
    }

    /** Un message en JSON ; rend l'état à dessiner et les effets à accomplir. */
    external fun dispatch(message: String): String

    external fun setLanguage(french: Boolean)

    /** Vérifie la signature ; sans octets, reprend la dernière copie vérifiée. */
    external fun loadCatalog(directory: String, catalog: ByteArray, signature: ByteArray): String

    /** Écrit et vérifie. Bloque : hors du fil de l'écran. La progression arrive par [dispatch]. */
    external fun write(
        fd: Int,
        descriptors: ByteArray,
        diskId: String,
        source: String,
        sha256: String,
        raw: Boolean,
        mbr: Boolean,
    )

    external fun probe(fd: Int, descriptors: ByteArray): String

    external fun readHash(fd: Int, descriptors: ByteArray, offset: Long, length: Int): String

    external fun writeBench(fd: Int, descriptors: ByteArray): String
}
