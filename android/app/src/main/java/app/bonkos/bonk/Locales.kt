package app.bonkos.bonk

import android.content.Context
import android.content.res.Configuration
import java.util.Locale

/**
 * Le même contexte, dans la langue choisie dans Bonk : les textes propres à l'application (ressources
 * Android, notification) suivent ce choix plutôt que la langue du téléphone.
 */
fun Context.inLanguage(tag: String): Context =
    createConfigurationContext(Configuration(resources.configuration).apply { setLocale(Locale.forLanguageTag(tag)) })
