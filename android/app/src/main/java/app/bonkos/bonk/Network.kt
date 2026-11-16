package app.bonkos.bonk

import java.net.HttpURLConnection
import java.net.InetSocketAddress
import java.net.Proxy
import java.net.URI
import java.net.URL

// Without a timeout a silent connection would block the download and its cancel.
private const val CONNECT_TIMEOUT_MS = 30_000
private const val READ_TIMEOUT_MS = 60_000

/** The system default names the phone model and its build: only the app and its version leave the device. */
internal const val USER_AGENT = "bonk/" + BuildConfig.VERSION_NAME

/**
 * An image is asked as is (`compressed = false`): with gzip the system clears the length the free space check and
 * the progress rely on.
 */
internal fun open(
    address: String,
    proxy: Proxy = Proxy.NO_PROXY,
    compressed: Boolean = true,
): HttpURLConnection =
    (URL(address).openConnection(proxy) as HttpURLConnection).apply {
        connectTimeout = CONNECT_TIMEOUT_MS
        readTimeout = READ_TIMEOUT_MS
        setRequestProperty("User-Agent", USER_AGENT)
        if (!compressed) setRequestProperty("Accept-Encoding", "identity")
    }

/** Rust only hands over `socks5h://` addresses: anything else is refused, never a direct connection. */
internal fun socks(address: String): Proxy {
    val uri = URI(address)
    if (uri.scheme != "socks5h" || uri.host == null || uri.port < 0) {
        throw BonkFailure("E3008", "$address: only socks5h:// is accepted")
    }
    return Proxy(Proxy.Type.SOCKS, InetSocketAddress(uri.host, uri.port))
}
