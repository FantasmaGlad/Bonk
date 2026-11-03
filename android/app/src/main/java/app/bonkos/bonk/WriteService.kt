package app.bonkos.bonk

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
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

private const val CHANNEL = "write"
private const val NOTIFICATION = 1
private const val REQUEST = "request"

/** Filet si le service ne relâchait jamais le verrou : aucune écriture ne dure six heures. */
private const val WAKE_LOCK_MS = 6 * 60 * 60 * 1000L

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
        val title = inLanguage(Bridge.language).getString(R.string.writing_notification)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, title, NotificationManager.IMPORTANCE_LOW))
        // Toucher la notification rouvre l'écran, où se lisent la progression et le résultat.
        val open =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val notification =
            Notification.Builder(this, CHANNEL)
                .setContentTitle(title)
                .setSmallIcon(android.R.drawable.stat_sys_upload)
                .setContentIntent(open)
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
                    .apply { acquire(WAKE_LOCK_MS) }
        }
        // Premier plan d'abord : un échec immédiat de l'écriture arrête alors un service déjà au premier plan,
        // ce qu'Android accepte, et jamais l'inverse.
        val request = intent?.getStringExtra(REQUEST)?.let(Protocol::decodeWrite)
        if (request == null || !writing.compareAndSet(false, true)) {
            if (!writing.get()) stopSelf()
            return START_NOT_STICKY
        }
        thread {
            try {
                Actions(this, Bridge::send) { _, _ -> }.runWrite(request)
            } finally {
                request.device?.let(Bridge.written::add)
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
        fun start(context: Context, request: Write) {
            val intent = Intent(context, WriteService::class.java).putExtra(REQUEST, Protocol.encodeWrite(request))
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
