package app.bonkos.bonk

/** La bibliothèque Rust : chaque méthode répond en JSON, une erreur comprise. */
object Native {
    init {
        System.loadLibrary("bonk_android")
    }

    external fun probe(fd: Int, descriptors: ByteArray): String

    external fun readHash(fd: Int, descriptors: ByteArray, offset: Long, length: Int): String

    external fun writeBench(fd: Int, descriptors: ByteArray): String
}
