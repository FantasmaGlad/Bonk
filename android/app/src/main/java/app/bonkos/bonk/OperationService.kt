package app.bonkos.bonk

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.annotation.RequiresApi
import androidx.core.content.ContextCompat
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

private const val REQUEST = "request"
private const val ACTION_CANCEL = "app.bonkos.bonk.CANCEL"
private const val NOTICE_MS = 1000L

/** Safety net if the service never released the lock: nothing Bonk does lasts six hours. */
private const val WAKE_LOCK_MS = 6 * 60 * 60 * 1000L

private const val END_WAIT_MS = 3000L

/**
 * One operation that must go on once the app is left: without a foreground service Android may freeze the
 * process. Its notification shows what Rust says of the operation, with a cancel button, then its end.
 */
abstract class OperationService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private val running = AtomicBoolean(false)

    /** The last start that launched nothing (a cancel, a refused request): stopping must cover it too. */
    private val idleStart = AtomicInteger(0)

    protected abstract val channel: String
    protected abstract val notificationId: Int
    protected abstract val genericTitle: Int
    protected abstract val icon: Int

    /** `ServiceInfo.FOREGROUND_SERVICE_TYPE_*`, as declared in the manifest. */
    @RequiresApi(Build.VERSION_CODES.Q)
    protected abstract fun serviceType(): Int
    protected abstract val cancel: ToRust

    /** Blocks until the operation ends. */
    protected abstract fun perform(request: Effect)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(notificationId, build(null), serviceType())
        } else {
            startForeground(notificationId, build(null))
        }
        // Foreground first: Android refuses to stop a service that is not there yet.
        if (intent?.action == ACTION_CANCEL) {
            Bridge.send(cancel)
            launchedNothing(startId)
            return START_NOT_STICKY
        }
        val request = intent?.getStringExtra(REQUEST)?.let(Protocol::decodeTask)
        if (request == null || !running.compareAndSet(false, true)) {
            launchedNothing(startId)
            return START_NOT_STICKY
        }
        holdWakeLock()
        thread { follow() }
        thread { run(request, startId) }
        return START_NOT_STICKY
    }

    private fun launchedNothing(startId: Int) {
        idleStart.accumulateAndGet(startId, ::maxOf)
        if (!running.get()) stopSelf(startId)
    }

    // stopSelf(id) keeps the service when a later start launched a new operation.
    private fun run(request: Effect, startId: Int) {
        try {
            perform(request)
        } finally {
            running.set(false)
            end()
            stopSelf(maxOf(startId, idleStart.get()))
        }
    }

    /** Rust reads the engine's events when asked: the clock asks, even with the screen gone. */
    private fun follow() {
        while (running.get()) {
            Bridge.send(Clock)
            Thread.sleep(NOTICE_MS)
            if (running.get()) show(currentNotice())
        }
    }

    private fun end() {
        val deadline = System.currentTimeMillis() + END_WAIT_MS
        var notice: Notice? = null
        while (System.currentTimeMillis() < deadline) {
            Bridge.send(Clock)
            Thread.sleep(NOTICE_MS / 4)
            notice = currentNotice()
            if (notice?.last == true) break
        }
        if (notice?.last == true) {
            stopForeground(STOP_FOREGROUND_DETACH)
            show(notice)
        } else {
            stopForeground(STOP_FOREGROUND_REMOVE)
        }
    }

    private fun currentNotice(): Notice? = runCatching { Protocol.notice(Native.notice()) }.getOrNull()

    private fun show(notice: Notice?) {
        getSystemService(NotificationManager::class.java).notify(notificationId, build(notice))
    }

    private fun build(notice: Notice?): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        val localized = inLanguage(Bridge.language)
        val generic = localized.getString(genericTitle)
        manager.createNotificationChannel(NotificationChannel(channel, generic, NotificationManager.IMPORTANCE_LOW))
        val open =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val ongoing = notice?.last != true
        val builder =
            Notification.Builder(this, channel)
                .setContentTitle(notice?.title ?: generic)
                .setSmallIcon(icon)
                .setContentIntent(open)
                .setOngoing(ongoing)
                .setOnlyAlertOnce(true)
        notice?.body?.let(builder::setContentText)
        if (ongoing) {
            notice?.let { builder.setProgress(100, it.percent ?: 0, it.percent == null) }
            val stop =
                PendingIntent.getService(
                    this,
                    0,
                    Intent(this, javaClass).setAction(ACTION_CANCEL),
                    PendingIntent.FLAG_IMMUTABLE,
                )
            builder.addAction(Notification.Action.Builder(null as Icon?, localized.getString(R.string.cancel), stop).build())
        } else {
            builder.setAutoCancel(true)
        }
        return builder.build()
    }

    private fun holdWakeLock() {
        if (wakeLock == null) {
            wakeLock =
                getSystemService(PowerManager::class.java)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "bonk:$channel")
                    .apply { acquire(WAKE_LOCK_MS) }
        }
    }

    /** Android 15 stops a data sync after six hours a day: the operation is cancelled, never left half done. */
    override fun onTimeout(
        startId: Int,
        fgsType: Int,
    ) {
        Bridge.send(cancel)
    }

    override fun onDestroy() {
        wakeLock?.release()
        super.onDestroy()
    }

    companion object {
        fun start(
            context: Context,
            service: Class<out OperationService>,
            request: Effect,
        ) {
            val intent = Intent(context, service).putExtra(REQUEST, Protocol.encodeTask(request))
            ContextCompat.startForegroundService(context, intent)
        }
    }
}
