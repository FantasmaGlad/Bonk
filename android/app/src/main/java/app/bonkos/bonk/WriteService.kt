package app.bonkos.bonk

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.content.ContextCompat
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import org.json.JSONObject

private const val CHANNEL = "write"
private const val NOTIFICATION = 1
private const val REQUEST = "request"

/**
 * Porte l'écriture : sans service, Android peut figer le processus dès qu'on quitte l'application, et
 * l'écriture s'arrête à mi-chemin. Elle vit ici, et non dans l'écran, pour que fermer l'écran ne la tue pas.
 */
class WriteService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private val writing = AtomicBoolean(false)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        val title = getString(R.string.writing_notification)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, title, NotificationManager.IMPORTANCE_LOW))
        val notification =
            Notification.Builder(this, CHANNEL)
                .setContentTitle(title)
                .setSmallIcon(android.R.drawable.stat_sys_upload)
                .setOngoing(true)
                .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } else {
            startForeground(NOTIFICATION, notification)
        }
        if (wakeLock == null) {
            wakeLock =
                getSystemService(PowerManager::class.java)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "bonk:write")
                    .apply { acquire() }
        }
        // Premier plan d'abord : un échec immédiat de l'écriture arrête alors un service déjà au premier plan,
        // ce qu'Android accepte, et jamais l'inverse.
        val request = intent?.getStringExtra(REQUEST)?.let(::JSONObject)
        if (request == null || !writing.compareAndSet(false, true)) {
            if (!writing.get()) stopSelf()
            return START_NOT_STICKY
        }
        thread {
            try {
                Actions(this, Bridge::send) { _, _ -> }.runWrite(request)
            } finally {
                writing.set(false)
                stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        wakeLock?.release()
        super.onDestroy()
    }

    companion object {
        fun start(context: Context, request: JSONObject) {
            val intent = Intent(context, WriteService::class.java).putExtra(REQUEST, request.toString())
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
