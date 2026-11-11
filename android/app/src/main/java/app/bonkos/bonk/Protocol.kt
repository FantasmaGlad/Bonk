package app.bonkos.bonk

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonClassDiscriminator


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

@Serializable @SerialName("language")
data object SwitchLanguage : ToRust

@Serializable @SerialName("menu")
data object ToggleMenu : ToRust

/** Image menu filter; without a value the filter is removed. */
@Serializable @SerialName("category")
data class CategoryFilter(val value: String? = null) : ToRust

@Serializable @SerialName("system")
data class SystemFilter(val value: String? = null) : ToRust

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

@Serializable @SerialName("failed")
data class Failed(val message: String, val code: String? = null) : ToRust

/** Failure born in the app, tied to a Bonk code: the screen shows its translated title. */
class BonkFailure(val code: String, message: String) : Exception(message)

@Serializable @SerialName("catalog_failed")
data class CatalogFailed(val message: String) : ToRust

/** Nothing to report: only reread the state, after a write for instance. Rust ignores this type. */
/** A `bonk://install/<image>` link the app was opened with; Rust checks it and reads nothing else from it. */
@Serializable @SerialName("open")
data class Open(val link: String) : ToRust

@Serializable @SerialName("refresh")
data object Refresh : ToRust

/** A vault panel button: Rust names its action. */
@Serializable @SerialName("vault")
data class VaultButton(val action: String) : ToRust

@Serializable @SerialName("vault_picked")
data class VaultPicked(val value: String) : ToRust

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

@Serializable
sealed interface Effect

@Serializable @SerialName("enumerate")
data object Enumerate : Effect

@Serializable @SerialName("catalog")
data object LoadCatalog : Effect

@Serializable @SerialName("pick")
data object PickImage : Effect

@Serializable @SerialName("save_language")
data class SaveLanguage(val value: String) : Effect

/** The confirmed draft: `device` is the chosen disk's device, absent if it left meanwhile. */
@Serializable @SerialName("write")
data class Write(
    @SerialName("disk_id") val diskId: String,
    val device: String?,
    val image: String,
    val hash: String,
    val raw: Boolean,
    val mbr: Boolean,
) : Effect

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
) : Effect

@Serializable
data class Answer(val state: ScreenState = ScreenState(), val effects: List<Effect> = emptyList())

/** Everything the screen shows; texts already come in the chosen language. */
@Serializable
data class ScreenState(
    val intro: String = "",
    val language: String = "",
    /** Language chosen in Bonk (`fr`, `en`): the app reads its own texts in it. */
    val lang: String = "en",
    val status: String = "",
    /** What became of the image a link asked for. */
    val notice: String? = null,
    @SerialName("show_all") val showAll: Boolean = false,
    @SerialName("show_all_label") val showAllLabel: String = "",
    val empty: String = "",
    val disks: List<DiskView> = emptyList(),
    val panel: Panel? = null,
    /** The vault panel of the chosen disk; it takes the place of the write panel. */
    val vault: VaultView? = null,
    val running: Boolean = false,
)

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
data class DiskView(
    val id: String,
    val device: String,
    val model: String,
    val facts: String,
    val badge: String,
    val state: String,
    val selected: Boolean,
)

/** The write panel, named by `kind`. Absent when no disk is chosen. */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("kind")
sealed interface Panel

@Serializable @SerialName("draft")
data class DraftPanel(
    val title: String,
    /** Absent if the chosen disk just left the list. */
    val summary: String? = null,
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
    @SerialName("all_systems") val allSystems: String = "",
    val category: String? = null,
    val system: String? = null,
    val categories: List<String> = emptyList(),
    val systems: List<String> = emptyList(),
    val rows: List<MenuRow> = emptyList(),
)

/** A menu image; without `index` it is shown but cannot be chosen. */
@Serializable
data class MenuRow(val title: String, val note: String? = null, val index: Int? = null)

@Serializable
data class Outcome(val error: String? = null)

@Serializable
data class Probe(
    @SerialName("size_bytes") val sizeBytes: Long = 0,
    val partitions: List<PartitionInfo> = emptyList(),
    val error: String? = null,
)

object Protocol {
    private val toRust = Json { classDiscriminator = "type" }

    /** A field added on the Rust side must not break an older app. */
    private val fromRust =
        Json {
            classDiscriminator = "do"
            ignoreUnknownKeys = true
        }

    fun encode(message: ToRust): String = toRust.encodeToString(ToRust.serializer(), message)

    fun answer(text: String): Answer = fromRust.decodeFromString(Answer.serializer(), text)

    fun probe(text: String): Probe = fromRust.decodeFromString(Probe.serializer(), text)

    fun outcome(text: String): Outcome = fromRust.decodeFromString(Outcome.serializer(), text)

    /** A write or a vault action, handed to the service. */
    fun encodeTask(task: Effect): String = fromRust.encodeToString(Effect.serializer(), task)

    fun decodeTask(text: String): Effect = fromRust.decodeFromString(Effect.serializer(), text)
}
