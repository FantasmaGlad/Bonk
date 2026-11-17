package app.bonkos.bonk

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonClassDiscriminator

@Serializable
data class Answer(val state: ScreenState = ScreenState(), val effects: List<Effect> = emptyList())

/** Everything the screen shows; texts already come in the chosen language. */
@Serializable
data class ScreenState(
    @SerialName("settings_button") val settingsButton: String = "",
    /** The settings page; absent on the main page. */
    val settings: SettingsView? = null,
    /** Absent offline. */
    val incognito: IncognitoView? = null,
    /** Absent offline. */
    @SerialName("catalog_button") val catalogButton: String? = null,
    /** The catalog tab; absent on the other pages. */
    val browse: BrowseView? = null,
    /** Language chosen in Bonk (`fr`, `en`): the app reads its own texts in it. */
    val lang: String = "en",
    val status: String = "",
    /** What became of the image a link asked for. */
    val notice: String? = null,
    val empty: String = "",
    val disks: List<DiskView> = emptyList(),
    val panel: Panel? = null,
    /** The vault panel of the chosen disk; it takes the place of the write panel. */
    val vault: VaultView? = null,
    val running: Boolean = false,
)

@Serializable
data class IncognitoView(val available: Boolean, val on: Boolean, val label: String, val link: String)

@Serializable
data class VaultView(
    val title: String,
    val busy: Boolean,
    val lines: List<String>,
    val fraction: Float? = null,
    val images: List<VaultImage>,
    val buttons: List<VaultAction>,
)

@Serializable
data class VaultImage(val name: String, val size: String)

@Serializable
data class VaultAction(val label: String, val action: String, val primary: Boolean)

@Serializable
data class SettingsView(
    val title: String,
    val back: String,
    /** False while a drive is being written: a relaunch would end it. */
    val enabled: Boolean,
    val sections: List<SettingsSection>,
    @SerialName("disks_title") val disksTitle: String,
    @SerialName("show_all") val showAll: Boolean,
    @SerialName("show_all_label") val showAllLabel: String,
    @SerialName("network_title") val networkTitle: String = "",
    val offline: Boolean = false,
    @SerialName("offline_label") val offlineLabel: String = "",
    @SerialName("offline_note") val offlineNote: String = "",
    @SerialName("incognito_at_start") val incognitoAtStart: Boolean = false,
    @SerialName("incognito_available") val incognitoAvailable: Boolean = false,
    @SerialName("incognito_label") val incognitoLabel: String = "",
    @SerialName("incognito_note") val incognitoNote: String = "",
)

@Serializable
data class SettingsSection(val title: String, val note: String, val options: List<SettingsOption>)

@Serializable
data class SettingsOption(val label: String, val action: String, val value: String, val chosen: Boolean)

@Serializable
data class DiskView(
    val id: String,
    val device: String,
    val model: String,
    val facts: String,
    val badge: String,
    val state: String,
    val selected: Boolean,
    /** Device node, serial and partitions, when opened. */
    val details: List<String> = emptyList(),
    /** Under the selected card only. */
    @SerialName("details_button") val detailsButton: String? = null,
)

/** The write panel, named by `kind`. Absent when no disk is chosen. */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("kind")
sealed interface Panel

@Serializable @SerialName("draft")
data class DraftPanel(
    val title: String,
    /** The erased disk, in the recap; absent if it just left the list. */
    val summary: String? = null,
    @SerialName("image_line") val imageLine: String = "",
    val warning: String,
    val menu: MenuView,
    val advanced: Boolean,
    @SerialName("advanced_button") val advancedButton: String,
    val image: String,
    @SerialName("image_hint") val imageHint: String,
    val pick: String,
    val hash: String,
    @SerialName("hash_hint") val hashHint: String,
    val raw: Boolean,
    @SerialName("raw_label") val rawLabel: String,
    val mbr: Boolean,
    @SerialName("mbr_label") val mbrLabel: String,
    val button: String,
    val enabled: Boolean,
    @SerialName("vault_button") val vaultButton: String,
) : Panel

@Serializable @SerialName("running")
data class RunningPanel(val line: String, val fraction: Float? = null, val cancel: String) : Panel

@Serializable @SerialName("finished")
data class FinishedPanel(val message: String, val close: String) : Panel

/** The image menu: a waiting line until the catalog is read. */
@Serializable
data class MenuView(
    val ready: Boolean,
    val line: String = "",
    val open: Boolean = false,
    val button: String = "",
    @SerialName("all_categories") val allCategories: String = "",
    @SerialName("all_bases") val allBases: String = "",
    @SerialName("all_systems") val allSystems: String = "",
    val category: String? = null,
    val base: String? = null,
    val system: String? = null,
    val categories: List<String> = emptyList(),
    val bases: List<String> = emptyList(),
    val systems: List<String> = emptyList(),
    val search: String = "",
    @SerialName("search_hint") val searchHint: String = "",
    @SerialName("all_arches") val allArches: String = "",
    val arch: String? = null,
    val arches: List<String> = emptyList(),
    val rows: List<MenuRow> = emptyList(),
)

/** A menu image; without `index` it is shown but cannot be chosen. */
@Serializable
data class MenuRow(val title: String, val note: String? = null, val index: Int? = null)
