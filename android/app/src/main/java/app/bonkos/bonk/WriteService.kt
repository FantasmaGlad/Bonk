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

/** Safety net if the service never released the lock: no write lasts six hours. */
private const val WAKE_LOCK_MS = 6 * 60 * 60 * 1000L

/** Without a service Android may freeze the process as soon as the app is left, mid-write. */
class WriteService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private val writing = AtomicBoolean(false)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        val title = inLanguage(Bridge.language).getString(R.string.writing_notification)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, title, NotificationManager.IMPORTANCE_LOW))
        // Tapping the notification reopens the screen, with the progress and the result.
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
        // Foreground first: Android refuses to stop a service that is not there yet.
        val request = intent?.getStringExtra(REQUEST)?.let(Protocol::decodeTask)
        if (request == null || !writing.compareAndSet(false, true)) {
            if (!writing.get()) stopSelf()
            return START_NOT_STICKY
        }
        thread {
            val actions = Actions(this, Bridge::send) { _, _ -> }
            try {
                when (request) {
                    is Write -> actions.runWrite(request)
                    is VaultRun -> actions.runVault(request)
                    else -> {}
                }
            } finally {
                when (request) {
                    is Write -> request.device
                    is VaultRun -> request.device
                    else -> null
                }?.let(Bridge.written::add)
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
        /** A write or a vault action that writes to the key. */
        fun start(context: Context, request: Effect) {
            val intent = Intent(context, WriteService::class.java).putExtra(REQUEST, Protocol.encodeTask(request))
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
