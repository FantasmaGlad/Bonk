package app.bonkos.bonk

object Native {
    init {
        System.loadLibrary("bonk_android")
    }

    external fun dispatch(message: String): String

    external fun setLanguage(french: Boolean)

    /** Vérifie la signature ; sans octets, reprend la dernière copie vérifiée. */
    external fun loadCatalog(directory: String, catalog: ByteArray, signature: ByteArray): String

    /** Bloque. Rust refuse si `device` et `serial` ne sont pas ceux du disque choisi. */
    external fun write(
        fd: Int,
        descriptors: ByteArray,
        diskId: String,
        device: String,
        serial: String,
        source: String,
        sha256: String,
        raw: Boolean,
        mbr: Boolean,
    )

    external fun probe(fd: Int, descriptors: ByteArray): String

    external fun readHash(fd: Int, descriptors: ByteArray, offset: Long, length: Int): String

    external fun writeBench(fd: Int, descriptors: ByteArray): String
}
