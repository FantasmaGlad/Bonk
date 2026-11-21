package app.bonkos.bonk

import android.content.Context
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.annotation.RequiresApi

class DownloadService : OperationService() {
    override val channel = "download"
    override val notificationId = 2
    override val genericTitle = R.string.downloading_notification
    override val icon = android.R.drawable.stat_sys_download
    @RequiresApi(Build.VERSION_CODES.Q)
    override fun serviceType() = ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
    override val cancel: ToRust = CatalogDownload("cancel")

    override fun perform(request: Effect) {
        if (request is DownloadImage) ImageDownload(this, Bridge::send).run(request)
    }

    companion object {
        fun start(
            context: Context,
            request: DownloadImage,
        ) = OperationService.start(context, DownloadService::class.java, request)
    }
}
