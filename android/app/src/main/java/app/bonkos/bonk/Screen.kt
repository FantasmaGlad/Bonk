package app.bonkos.bonk

import android.hardware.usb.UsbManager
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
internal fun Screen(
    state: ScreenState,
    usb: UsbManager,
    revision: Int,
    errors: Map<String, String>,
    send: (ToRust) -> Unit,
) {
    val settings = state.settings
    if (settings != null) {
        SettingsPage(settings, send)
        return
    }
    val browse = state.browse
    if (browse != null) {
        CatalogPage(browse, send)
        return
    }
    Column(
        Modifier.safeDrawingPadding()
            .padding(Tokens.SPACE_PAGE.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_LARGE.dp),
    ) {
        Header(state, send)
        Small(state.status)
        state.notice?.let { Body(it) }
        WaitingDrives(state, usb, revision, errors)
        Disks(state, send)
        when (val panel = state.panel) {
            is RunningPanel -> Running(panel, send)
            is FinishedPanel -> Boxed {
                Body(panel.message)
                Primary(panel.close) { send(Dismiss) }
            }
            is DraftPanel, null -> {}
        }
        // Absent from the published build: only useful with the test key named at build time.
        if (BuildConfig.BENCH_SERIAL.isNotEmpty()) {
            HorizontalDivider(color = Color(LocalPalette.current.ink))
            Tests(usb, revision)
        }
    }
}
