package app.bonkos.bonk

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
sealed interface Effect

@Serializable @SerialName("enumerate")
data object Enumerate : Effect

/** `proxy`: Tor while incognito; the catalog, its signature and the logos then all go through it. */
@Serializable @SerialName("catalog")
data class LoadCatalog(val proxy: String? = null) : Effect

@Serializable @SerialName("pick")
data object PickImage : Effect

/** Kept for the next start; takes effect now. */
@Serializable @SerialName("save_setting")
data class SaveSetting(val key: String, val value: String) : Effect

/** Kept, then the screen starts again: language and theme never change while it runs. */
@Serializable @SerialName("relaunch")
data class Relaunch(val key: String, val value: String) : Effect

/** The confirmed draft: `device` is the chosen disk's device, absent if it left meanwhile. */
@Serializable @SerialName("write")
data class Write(
    @SerialName("disk_id") val diskId: String,
    val device: String?,
    val image: String,
    val hash: String,
    val raw: Boolean,
    val mbr: Boolean,
    /** `socks5h://` address of Orbot: the image is downloaded through it, names resolved by Tor. */
    val proxy: String? = null,
) : Effect

/** A catalog image to save without writing it; address and hash come from the signed catalog. */
@Serializable @SerialName("download_image")
data class DownloadImage(val url: String, val sha256: String, val name: String, val proxy: String? = null) : Effect

/** A fixed address of Bonk itself, opened in the browser. */
@Serializable @SerialName("open_link")
data class OpenLink(val url: String) : Effect

@Serializable @SerialName("vault_pick")
data object PickVaultIso : Effect

/** One GRUB image, for one processor; Rust checks the bytes against the signed catalog. */
@Serializable
data class GrubDownload(val arch: String, val url: String, @SerialName("size_bytes") val sizeBytes: Long)

/**
 * A vault action on the chosen disk. `iso` for `add`; `grub` for `init` and `update`, empty while the catalog
 * offers none (Rust then refuses with the reason).
 */
@Serializable @SerialName("vault")
data class VaultRun(
    val action: String,
    @SerialName("disk_id") val diskId: String,
    val device: String?,
    val iso: String? = null,
    val grub: List<GrubDownload> = emptyList(),
    /** Tor while incognito, for GRUB. */
    val proxy: String? = null,
) : Effect
