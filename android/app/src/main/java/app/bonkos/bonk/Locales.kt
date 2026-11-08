package app.bonkos.bonk

import android.content.Context
import android.content.res.Configuration
import java.util.Locale

/** Les ressources Android suivent la langue choisie dans Bonk, pas celle du téléphone. */
fun Context.inLanguage(tag: String): Context =
    createConfigurationContext(Configuration(resources.configuration).apply { setLocale(Locale.forLanguageTag(tag)) })
