package app.bonkos.bonk

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonClassDiscriminator

/*
 * Ce qui passe entre l'application et Rust (`crates/bonk-android/src/screen.rs`). Les échantillons de
 * `src/test/resources/protocole.json` viennent du code Rust ; les tests de ce côté les relisent avec ces types.
 */

/** Un message vers Rust ; `type` le désigne. */
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

/** Filtre du menu des images ; sans valeur, le filtre est retiré. */
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

/** Échec né dans l'application, rattaché à un code de Bonk : l'écran en montre le titre traduit. */
class BonkFailure(val code: String, message: String) : Exception(message)

@Serializable @SerialName("catalog_failed")
data class CatalogFailed(val message: String) : ToRust

/** Rien à signaler : seulement relire l'état, par exemple après une écriture. Rust ignore ce type. */
@Serializable @SerialName("refresh")
data object Refresh : ToRust

/** Une clé telle que l'application la voit : périphérique USB, série et ce que la sonde a lu. */
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

/** Ce que Rust demande à l'application ; `do` le désigne. */
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

/** Le brouillon validé : `device` est le périphérique du disque choisi, absent s'il est parti entre-temps. */
@Serializable @SerialName("write")
data class Write(
    @SerialName("disk_id") val diskId: String,
    val device: String?,
    val image: String,
    val hash: String,
    val raw: Boolean,
    val mbr: Boolean,
) : Effect

/** La réponse de Rust à chaque message : l'état à dessiner et les effets à accomplir. */
@Serializable
data class Answer(val state: ScreenState = ScreenState(), val effects: List<Effect> = emptyList())

/** Tout ce que l'écran affiche ; les textes arrivent déjà dans la langue choisie. */
@Serializable
data class ScreenState(
    val intro: String = "",
    val language: String = "",
    val status: String = "",
    @SerialName("show_all") val showAll: Boolean = false,
    @SerialName("show_all_label") val showAllLabel: String = "",
    val empty: String = "",
    val disks: List<DiskView> = emptyList(),
    val panel: Panel? = null,
    val running: Boolean = false,
)

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

/** Le panneau d'écriture ; `kind` le désigne. Absent quand aucun disque n'est choisi. */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("kind")
sealed interface Panel

@Serializable @SerialName("draft")
data class DraftPanel(
    val title: String,
    /** Absent si le disque choisi vient de disparaître de la liste. */
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
) : Panel

@Serializable @SerialName("running")
data class RunningPanel(val line: String, val fraction: Float? = null, val cancel: String) : Panel

@Serializable @SerialName("finished")
data class FinishedPanel(val message: String, val close: String) : Panel

/** Le menu des images : une ligne d'attente tant que le catalogue n'est pas lu. */
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

/** Une image du menu ; sans `index`, elle se montre sans pouvoir être choisie. */
@Serializable
data class MenuRow(val title: String, val note: String? = null, val index: Int? = null)

/** Réponse d'un appel natif qui ne rend rien d'autre qu'un éventuel échec. */
@Serializable
data class Outcome(val error: String? = null)

/** Réponse de la sonde d'une clé. */
@Serializable
data class Probe(
    @SerialName("size_bytes") val sizeBytes: Long = 0,
    val partitions: List<PartitionInfo> = emptyList(),
    val error: String? = null,
)

object Protocol {
    private val toRust = Json { classDiscriminator = "type" }

    /** Un champ ajouté côté Rust ne doit pas casser une application plus ancienne. */
    private val fromRust =
        Json {
            classDiscriminator = "do"
            ignoreUnknownKeys = true
        }

    fun encode(message: ToRust): String = toRust.encodeToString(ToRust.serializer(), message)

    fun answer(text: String): Answer = fromRust.decodeFromString(Answer.serializer(), text)

    fun probe(text: String): Probe = fromRust.decodeFromString(Probe.serializer(), text)

    fun outcome(text: String): Outcome = fromRust.decodeFromString(Outcome.serializer(), text)

    fun encodeWrite(write: Write): String = fromRust.encodeToString(Write.serializer(), write)

    fun decodeWrite(text: String): Write = fromRust.decodeFromString(Write.serializer(), text)
}
