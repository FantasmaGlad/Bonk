package app.bonkos.bonk

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

class BonkFailure(val code: String, message: String) : Exception(message)

@Serializable
data class Outcome(val error: String? = null)

@Serializable
data class Notice(val title: String, val body: String, val percent: Int? = null, val last: Boolean = false)

@Serializable
data class CatalogAddresses(val catalog: String, val signature: String)

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

    fun catalogAddresses(json: String): CatalogAddresses = fromRust.decodeFromString(CatalogAddresses.serializer(), json)

    fun answer(text: String): Answer = fromRust.decodeFromString(Answer.serializer(), text)

    fun probe(text: String): Probe = fromRust.decodeFromString(Probe.serializer(), text)

    fun outcome(text: String): Outcome = fromRust.decodeFromString(Outcome.serializer(), text)

    fun icons(text: String): List<IconDownload> = fromRust.decodeFromString(text)

    /** `null` when nothing runs or the setting is off. */
    fun notice(text: String): Notice? = fromRust.decodeFromString(text)

    fun encodeTask(task: Effect): String = fromRust.encodeToString(Effect.serializer(), task)

    fun decodeTask(text: String): Effect = fromRust.decodeFromString(Effect.serializer(), text)
}
