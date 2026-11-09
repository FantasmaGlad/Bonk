package app.bonkos.bonk

import android.content.Context
import android.content.res.Configuration
import java.util.Locale

/** Android resources follow the language chosen in Bonk, not the phone's. */
fun Context.inLanguage(tag: String): Context =
    createConfigurationContext(Configuration(resources.configuration).apply { setLocale(Locale.forLanguageTag(tag)) })
