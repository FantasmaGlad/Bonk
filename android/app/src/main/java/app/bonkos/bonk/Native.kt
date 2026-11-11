package app.bonkos.bonk

object Native {
    init {
        System.loadLibrary("bonk_android")
    }

    external fun dispatch(message: String): String

    external fun setLanguage(french: Boolean)

    /** Checks the signature; without bytes, falls back to the last verified copy. */
    external fun loadCatalog(directory: String, catalog: ByteArray, signature: ByteArray): String

    /** Blocks. Rust refuses if `device` and `serial` are not the chosen disk's. */
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

    /** Blocks. `iso` and `name` for `add`, `grub` (the folder of the downloaded images) for `init` and `update`. */
    external fun vault(
        fd: Int,
        descriptors: ByteArray,
        action: String,
        diskId: String,
        device: String,
        serial: String,
        iso: String,
        name: String,
        grub: String,
    )

    external fun probe(fd: Int, descriptors: ByteArray): String

    external fun readHash(fd: Int, descriptors: ByteArray, offset: Long, length: Int): String

    external fun writeBench(fd: Int, descriptors: ByteArray): String
}
