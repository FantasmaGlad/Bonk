package app.bonkos.bonk

import java.net.HttpURLConnection
import java.net.ServerSocket
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class NetworkTest {
    /** The headers the server received for one request opened by `connect`. */
    private fun headersOf(connect: (String) -> HttpURLConnection): Map<String, String> {
        ServerSocket(0).use { server ->
            var headers = emptyMap<String, String>()
            val reader =
                thread {
                    server.accept().use { client ->
                        val lines = client.getInputStream().bufferedReader()
                        headers =
                            generateSequence { lines.readLine() }
                                .takeWhile { it.isNotEmpty() }
                                .drop(1)
                                .associate { it.substringBefore(':').lowercase() to it.substringAfter(':').trim() }
                        client.getOutputStream().write("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".toByteArray())
                    }
                }
            connect("http://127.0.0.1:${server.localPort}/catalog.json").apply {
                assertEquals(204, responseCode)
                disconnect()
            }
            reader.join()
            return headers
        }
    }

    @Test
    fun seulsLAppEtSaVersionSontAnnonces() {
        assertEquals("bonk/${BuildConfig.VERSION_NAME}", headersOf { open(it) }["user-agent"])
    }

    @Test
    fun uneImageSeDemandeSansCompression() {
        assertEquals("identity", headersOf { open(it, compressed = false) }["accept-encoding"])
        assertNull(headersOf { open(it) }["accept-encoding"]?.takeIf { it == "identity" })
    }
}
