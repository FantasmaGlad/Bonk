package app.bonkos.bonk

import android.content.Context
import android.util.Log
import java.io.File
import java.net.Proxy
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

private const val TAG = "Bonk"
private const val ICON_WORKERS = 8
private const val ICONS_TIMEOUT_S = 120L

/**
 * The catalog keeps its last verified copy when the network fails: nothing here may prevent it. With a `proxy`
 * that does not answer the result is empty, never a direct connection.
 */
private fun fetchOrEmpty(address: String, proxy: String?): ByteArray =
    runCatching {
        open(address, proxy?.let(::socks) ?: Proxy.NO_PROXY).inputStream.use { it.readBytes() }
    }.getOrDefault(ByteArray(0))

internal class CatalogSync(private val context: Context, private val send: (ToRust) -> Unit) {
    fun catalog(proxy: String?) {
        thread {
            val directory = File(context.filesDir, "catalog").path
            val answer =
                runCatching {
                    Protocol.catalogAddresses(Native.catalogAddresses()).let { at ->
                        Protocol.outcome(
                            Native.loadCatalog(directory, fetchOrEmpty(at.catalog, proxy), fetchOrEmpty(at.signature, proxy)),
                        )
                    }
                }.getOrElse { Outcome(error = it.message ?: it.toString()) }
            send(answer.error?.let(::CatalogFailed) ?: Refresh)
            if (answer.error == null) icons(directory, proxy)
        }
    }

    /** After the list: the cards show at once, their logos as they arrive, [ICON_WORKERS] at a time. A logo that fails stays absent. */
    private fun icons(directory: String, proxy: String?) {
        val wanted = runCatching { Protocol.icons(Native.missingIcons(directory)) }.getOrDefault(emptyList())
        if (wanted.isEmpty()) return
        val pool = Executors.newFixedThreadPool(ICON_WORKERS)
        val stored = AtomicInteger()
        wanted.forEach { icon ->
            pool.execute {
                val bytes = fetchOrEmpty(icon.url, proxy)
                if (bytes.isNotEmpty()) {
                    Protocol.outcome(Native.storeIcon(directory, icon.sha256, bytes)).error?.let { Log.w(TAG, it) }
                }
                if (stored.incrementAndGet() % ICON_WORKERS == 0) send(Refresh)
            }
        }
        pool.shutdown()
        pool.awaitTermination(ICONS_TIMEOUT_S, TimeUnit.SECONDS)
        send(Refresh)
    }
}
