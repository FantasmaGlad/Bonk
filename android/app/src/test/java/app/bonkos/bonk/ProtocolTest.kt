package app.bonkos.bonk

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject

/** Samples come from the Rust code (`screen.rs`): these tests prove both sides speak the same. */
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
                SaveSetting("view.show_all", "true"),
                Relaunch("theme", "dark"),
                Write("usb-S1-001-002", "/dev/bus/usb/001/002", "/x.iso", "", raw = false, mbr = true),
                PickVaultIso,
                VaultRun("add", "usb-S1-001-002", "/dev/bus/usb/001/002", "/proc/self/fd/43"),
                VaultRun("update", "usb-S1-001-002", "/dev/bus/usb/001/002"),
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
                ToggleDetails,
                ShowAll(true),
                SettingsAction("open"),
                SettingsAction("close"),
                SettingsAction("language", "fr"),
                SettingsAction("theme", "dark"),
                SettingsAction("offline", "true"),
                ToggleMenu,
                CategoryFilter("Servers"),
                CategoryFilter(),
                BaseFilter("Debian"),
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
                Failed("the drive is no longer connected", "E1003"),
                Failed("unexpected"),
                CatalogFailed("404"),
                Open("bonk://install/debian-13-netinst-x86_64"),
                VaultButton("open"),
                VaultButton("close"),
                VaultButton("prepare"),
                VaultButton("confirm"),
                VaultButton("browse"),
                VaultButton("update"),
                VaultButton("cancel"),
                VaultPicked("/proc/self/fd/43"),
            )
        val expected = samples.getValue("messages").jsonArray
        assertEquals(expected.size, sent.size, "un message de Rust n'a pas de type Kotlin, ou l'inverse")
        sent.zip(expected).forEach { (message, sample) ->
            assertEquals(sample, Json.parseToJsonElement(Protocol.encode(message)), "$message")
        }
    }

    @Test
    fun chaqueFormeDeLEtatDEcranSeLitEnType() {
        val states =
            samples.getValue("states").jsonArray.map {
                Protocol.answer(JsonObject(mapOf("state" to it)).toString()).state
            }
        assertEquals(6, states.size)
        val (idle, draft, running, finished) = states
        val (vaultReady, vaultBusy) = states.drop(4)
        assertEquals(null, idle.panel)
        assertTrue("debian-13-netinst-x86_64" in checkNotNull(idle.notice))
        assertEquals(null, draft.notice)
        assertTrue(states.all { it.lang in setOf("fr", "en") })
        assertEquals(emptyList(), idle.disks)

        val panel = assertIs<DraftPanel>(draft.panel)
        assertTrue(draft.disks.single().selected)
        assertTrue(panel.enabled)
        assertTrue(panel.menu.ready && panel.menu.open)
        assertEquals(listOf(0, null), panel.menu.rows.map { it.index })
        assertEquals(null, panel.menu.category)

        assertTrue(running.running)
        assertEquals(0.25f, assertIs<RunningPanel>(running.panel).fraction)

        assertTrue("E2002" in assertIs<FinishedPanel>(finished.panel).message)
        assertTrue(panel.vaultButton.isNotEmpty())

        val ready = checkNotNull(vaultReady.vault)
        assertEquals(listOf("debian-live.iso"), ready.images.map { it.name })
        assertEquals(listOf("browse", "update", "close"), ready.buttons.map { it.action })
        assertTrue(!ready.busy && !vaultReady.running)
        val busy = checkNotNull(vaultBusy.vault)
        assertTrue(busy.busy && vaultBusy.running)
        assertEquals(0.25f, busy.fraction)
        assertEquals(listOf("cancel"), busy.buttons.map { it.action })
    }

    @Test
    fun uneTachePasseAuServiceSansPerte() {
        val write = Write("usb-S1-001-002", "/dev/bus/usb/001/002", "https://x/y.iso", "ab", raw = true, mbr = false)
        assertEquals(write, Protocol.decodeTask(Protocol.encodeTask(write)))
        val vault = VaultRun("init", "usb-S1-001-002", "/dev/bus/usb/001/002", grub = listOf(GrubDownload("x86_64", "https://x/BOOTX64.EFI", 5), GrubDownload("aarch64", "https://x/BOOTAA64.EFI", 6)))
        assertEquals(vault, Protocol.decodeTask(Protocol.encodeTask(vault)))
    }

    @Test
    fun uneReponseDeSondeEnErreurSeLit() {
        assertEquals("claimInterface refused", Protocol.probe("""{"error":"claimInterface refused"}""").error)
        assertEquals(emptyList(), Protocol.probe("{}").partitions)
    }
}
