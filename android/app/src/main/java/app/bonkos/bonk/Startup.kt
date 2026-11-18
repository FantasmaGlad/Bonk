package app.bonkos.bonk

import android.content.Context
import android.content.SharedPreferences
import android.content.res.Configuration
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import java.util.Locale

/** The native library starts with the settings saved on the phone. */
internal fun startNative(preferences: SharedPreferences, theme: String) {
    Native.start(
        preferences.getString("lang", "auto") ?: "auto",
        theme,
        Locale.getDefault().language == "fr",
        preferences.getString("view.show_all", "false") == "true",
        preferences.getString("network.offline", "false") == "true",
        preferences.getString("network.incognito", "false") == "true",
        preferences.getString("notifications", "true") == "true",
    )
}

/** Read once per start: the screen relaunches to change it. */
internal fun Context.isDark(theme: String): Boolean =
    when (theme) {
        "dark" -> true
        "light" -> false
        else -> resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
    }

/** System icons follow the page, not the phone theme. */
internal fun ComponentActivity.showBars(palette: Palette) {
    val bars =
        if (palette.dark) {
            SystemBarStyle.dark(palette.paper.toInt())
        } else {
            SystemBarStyle.light(palette.paper.toInt(), palette.ink.toInt())
        }
    enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
}
