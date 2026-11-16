package app.bonkos.bonk

import java.net.ServerSocket
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertEquals

class NetworkTest {
    @Test
    fun seulsLAppEtSaVersionSontAnnonces() {
        ServerSocket(0).use { server ->
            var agent: String? = null
            val reader =
                thread {
                    server.accept().use { client ->
                        val headers = client.getInputStream().bufferedReader()
                        agent =
                            generateSequence { headers.readLine() }
                                .takeWhile { it.isNotEmpty() }
                                .firstOrNull { it.startsWith("User-Agent:", ignoreCase = true) }
                                ?.substringAfter(':')
                                ?.trim()
                        client.getOutputStream().write("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".toByteArray())
                    }
                }
            open("http://127.0.0.1:${server.localPort}/catalog.json").apply {
                assertEquals(204, responseCode)
                disconnect()
            }
            reader.join()
            assertEquals("bonk/${BuildConfig.VERSION_NAME}", agent)
        }
    }
}
