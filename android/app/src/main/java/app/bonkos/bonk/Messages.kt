package app.bonkos.bonk

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
sealed interface ToRust

@Serializable @SerialName("disks")
data class Disks(val disks: List<Drive>) : ToRust

@Serializable @SerialName("tick")
data object Tick : ToRust

@Serializable @SerialName("clock")
data object Clock : ToRust

@Serializable @SerialName("select")
data class Select(val id: String) : ToRust

@Serializable @SerialName("details")
data object ToggleDetails : ToRust

/** Also show the disks the safety filter excludes, with the reason. */
@Serializable @SerialName("show_all")
data class ShowAll(val on: Boolean) : ToRust

/** A settings page action: `open`, `close`, `language` or `theme` with its stored value. */
@Serializable @SerialName("settings")
data class SettingsAction(val action: String, val value: String? = null) : ToRust

@Serializable @SerialName("menu")
data object ToggleMenu : ToRust

/** Image menu filter; without a value the filter is removed. */
@Serializable @SerialName("category")
data class CategoryFilter(val value: String? = null) : ToRust

@Serializable @SerialName("base")
data class BaseFilter(val value: String? = null) : ToRust

@Serializable @SerialName("system")
data class SystemFilter(val value: String? = null) : ToRust

/** Part of a system or image name; empty shows them all. */
@Serializable @SerialName("menu_search")
data class MenuSearch(val value: String) : ToRust

@Serializable @SerialName("arch")
data class ArchFilter(val value: String? = null) : ToRust

@Serializable @SerialName("choose")
data class Choose(val index: Int) : ToRust

@Serializable @SerialName("advanced")
data object ToggleAdvanced : ToRust

@Serializable @SerialName("image")
data class ImageInput(val value: String) : ToRust

@Serializable @SerialName("hash")
data class HashInput(val value: String) : ToRust

@Serializable @SerialName("raw")
data class Raw(val on: Boolean) : ToRust

@Serializable @SerialName("mbr")
data class Mbr(val on: Boolean) : ToRust

@Serializable @SerialName("browse")
data object Browse : ToRust

@Serializable @SerialName("picked")
data class Picked(val value: String) : ToRust

@Serializable @SerialName("start")
data object Start : ToRust

@Serializable @SerialName("cancel")
data object Cancel : ToRust

@Serializable @SerialName("dismiss")
data object Dismiss : ToRust

@Serializable @SerialName("download")
data class Download(val done: Long, val total: Long?, val rate: Double) : ToRust

/** A catalog image saved: where the user finds it, and its size. */
@Serializable @SerialName("downloaded")
data class Downloaded(val path: String, val bytes: Long) : ToRust

@Serializable @SerialName("failed")
data class Failed(val message: String, val code: String? = null) : ToRust

@Serializable @SerialName("catalog_failed")
data class CatalogFailed(val message: String) : ToRust

/** A `bonk://install/<image>` link the app was opened with; Rust checks it and reads nothing else from it. */
@Serializable @SerialName("open")
data class Open(val link: String) : ToRust

/** Rereads the state, after a write for instance; Rust ignores the type. */
@Serializable @SerialName("refresh")
data object Refresh : ToRust

/** A vault panel button: Rust names its action. */
@Serializable @SerialName("vault")
data class VaultButton(val action: String) : ToRust

@Serializable @SerialName("vault_picked")
data class VaultPicked(val value: String) : ToRust

/** The round incognito button: turns downloads through Tor on or off. */
@Serializable @SerialName("incognito")
data object ToggleIncognito : ToRust

@Serializable @SerialName("tor_project")
data object OpenTorProject : ToRust

/** Catalog tab: `open`, `close`, `close_system`. */
@Serializable @SerialName("catalog")
data class CatalogAction(val action: String) : ToRust

@Serializable @SerialName("catalog_search")
data class CatalogSearch(val value: String) : ToRust

/** Catalog download: `start` with the variant's `id`, `cancel`, `dismiss`. */
@Serializable @SerialName("catalog_download")
data class CatalogDownload(val action: String, val id: String? = null) : ToRust

/** A filter by the label shown; without a value the filter is removed. */
@Serializable @SerialName("catalog_filter")
data class CatalogFilter(val kind: String, val value: String? = null) : ToRust

@Serializable @SerialName("catalog_system")
data class CatalogSystem(val name: String) : ToRust

@Serializable @SerialName("catalog_write")
data class CatalogWrite(val id: String) : ToRust

/** A key as the app sees it: USB device, serial and what the probe read. */
@Serializable
data class Drive(
    val device: String,
    val model: String,
    val serial: String,
    @SerialName("size_bytes") val sizeBytes: Long,
    val partitions: List<PartitionInfo>,
)

@Serializable
data class PartitionInfo(val label: String?, @SerialName("size_bytes") val sizeBytes: Long)
