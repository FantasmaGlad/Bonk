package app.bonkos.bonk

import android.content.Context
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.annotation.RequiresApi

/** A write or a vault action that writes to the key. */
class WriteService : OperationService() {
    override val channel = "write"
    override val notificationId = 1
    override val genericTitle = R.string.writing_notification
    override val icon = android.R.drawable.stat_sys_upload
    @RequiresApi(Build.VERSION_CODES.Q)
    override fun serviceType() = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
    override val cancel: ToRust = Cancel

    override fun perform(request: Effect) {
        val operations = Operations(this, Bridge::send)
        try {
            when (request) {
                is Write -> operations.runWrite(request)
                is VaultRun -> operations.runVault(request)
                else -> {}
            }
        } finally {
            when (request) {
                is Write -> request.device
                is VaultRun -> request.device
                else -> null
            }?.let(Bridge.written::add)
        }
    }

    companion object {
        fun start(
            context: Context,
            request: Effect,
        ) = OperationService.start(context, WriteService::class.java, request)
    }
}
