package app.bonkos.bonk

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class BrowseView(
    val title: String,
    val back: String,
    /** While the catalog is read, or why it could not be. */
    val waiting: String? = null,
    val grid: BrowseGrid? = null,
    val system: SystemView? = null,
)

@Serializable
data class BrowseGrid(
    val search: String,
    @SerialName("search_hint") val searchHint: String,
    val filters: List<FilterView>,
    val cards: List<CardView>,
    val none: String,
)

/** `options` and `chosen` are labels; `kind` names the filter for Rust. */
@Serializable
data class FilterView(val kind: String, val all: String, val options: List<String>, val chosen: String? = null)

/** `icon`: a logo checked by Rust against its signed hash, in the app's files. */
@Serializable
data class CardView(val name: String, val count: String, val icon: String? = null)

@Serializable
data class SystemView(
    val name: String,
    val icon: String? = null,
    val back: String,
    val write: String,
    val variants: List<VariantView>,
)

/** `id` only for an image Bonk can write; otherwise `note` says why not. */
@Serializable
data class VariantView(val title: String, val facts: String, val id: String? = null, val note: String? = null)

/** A logo to download; Rust keeps it only if its bytes match `sha256`. */
@Serializable
data class IconDownload(val url: String, val sha256: String)
