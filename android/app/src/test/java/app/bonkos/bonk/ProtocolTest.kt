package app.bonkos.bonk

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject

/** Les échantillons viennent du code Rust (`screen.rs`) : ces tests prouvent que les deux côtés parlent pareil. */
class ProtocolTest {
    private val samples: JsonObject =
        Json.parseToJsonElement(
            checkNotNull(javaClass.classLoader?.getResource("protocole.json")) { "protocole.json absent" }.readText(),
        ).jsonObject

    @Test
    fun chaqueEffetDeRustSeLitEnType() {
        val effects = samples.getValue("effects")
        val answer = Protocol.answer(JsonObject(mapOf("effects" to effects)).toString())
        assertEquals(
            listOf(
                Enumerate,
                LoadCatalog,
                PickImage,
                SaveLanguage("fr"),
                Write("usb-S1-001-002", "/dev/bus/usb/001/002", "/x.iso", "", raw = false, mbr = false),
            ),
            answer.effects,
        )
    }

    @Test
    fun chaqueMessageSEcritCommeRustLAttend() {
        val sent =
            listOf(
                Disks(
                    listOf(
                        Drive(
                            "/dev/bus/usb/001/002",
                            "Generic UDisk",
                            "S1",
                            32_000_000_000,
                            listOf(PartitionInfo("ESP", 1_048_576), PartitionInfo(null, 2_097_152)),
                        ),
                    ),
                ),
                Tick,
                Clock,
                Select("usb-S1-001-002"),
                SwitchLanguage,
                ToggleMenu,
                CategoryFilter("Servers"),
                CategoryFilter(),
                SystemFilter("debian"),
                Choose(0),
                ToggleAdvanced,
                ImageInput("/x.iso"),
                HashInput("abc"),
                Raw(true),
                Mbr(false),
                Browse,
                Picked("/proc/self/fd/42"),
                Start,
                Cancel,
                Dismiss,
                Download(1024, 4096, 512.0),
                Failed("the drive is no longer connected"),
                CatalogFailed("404"),
            )
        val expected = samples.getValue("messages").jsonArray
        assertEquals(expected.size, sent.size, "un message de Rust n'a pas de type Kotlin, ou l'inverse")
        sent.zip(expected).forEach { (message, sample) ->
            assertEquals(sample, Json.parseToJsonElement(Protocol.encode(message)), "$message")
        }
    }

    @Test
    fun uneDemandeDEcriturePasseAuServiceSansPerte() {
        val write = Write("usb-S1-001-002", "/dev/bus/usb/001/002", "https://x/y.iso", "ab", raw = true, mbr = false)
        assertEquals(write, Protocol.decodeWrite(Protocol.encodeWrite(write)))
    }

    @Test
    fun uneReponseDeSondeEnErreurSeLit() {
        assertEquals("claimInterface refused", Protocol.probe("""{"error":"claimInterface refused"}""").error)
        assertEquals(emptyList(), Protocol.probe("{}").partitions)
    }
}
