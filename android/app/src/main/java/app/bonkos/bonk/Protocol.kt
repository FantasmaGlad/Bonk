package app.bonkos.bonk

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Failure born in the app, tied to a Bonk code: the screen shows its translated title. */
class BonkFailure(val code: String, message: String) : Exception(message)

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

    fun icons(text: String): List<IconDownload> = fromRust.decodeFromString(text)

    /** A write or a vault action, handed to the service. */
    fun encodeTask(task: Effect): String = fromRust.encodeToString(Effect.serializer(), task)

    fun decodeTask(text: String): Effect = fromRust.decodeFromString(Effect.serializer(), text)
}
