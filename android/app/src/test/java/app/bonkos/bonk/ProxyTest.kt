package app.bonkos.bonk

import java.net.InetSocketAddress
import java.net.Proxy
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class ProxyTest {
    @Test
    fun orbotSeLitEnProxySocks() {
        val proxy = socks("socks5h://127.0.0.1:9050")
        assertEquals(Proxy.Type.SOCKS, proxy.type())
        assertEquals(9050, (proxy.address() as InetSocketAddress).port)
    }

    @Test
    fun toutAutreSchemaEstRefuse() {
        assertFailsWith<BonkFailure> { socks("socks5://127.0.0.1:9050") }
        assertFailsWith<BonkFailure> { socks("http://127.0.0.1:8118") }
        assertFailsWith<BonkFailure> { socks("socks5h://127.0.0.1") }
    }
}
