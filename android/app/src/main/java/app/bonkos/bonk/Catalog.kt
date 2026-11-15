package app.bonkos.bonk

import android.graphics.BitmapFactory
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private const val CARD_MIN_DP = 150
private const val CARD_LOGO_DP = 48
private const val PAGE_LOGO_DP = 64

/** The catalog tab: a grid of systems, or one system and its variants. Back closes one level. */
@Composable
internal fun CatalogPage(view: BrowseView, send: (ToRust) -> Unit) {
    val system = view.system
    BackHandler { send(CatalogAction(if (system != null) "close_system" else "close")) }
    Column(
        Modifier.safeDrawingPadding().padding(Tokens.SPACE_PAGE.dp),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_LARGE.dp),
    ) {
Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
        ) {
            Text(view.title, fontSize = Tokens.TEXT_TITLE.sp, modifier = Modifier.weight(1f))
            Secondary(view.back) { send(CatalogAction("close")) }
        }
        val grid = view.grid
        when {
            system != null -> SystemPage(system, send)
            grid != null -> Grid(grid, send)
            else -> Small(view.waiting.orEmpty())
        }
    }
}

@Composable
private fun Grid(grid: BrowseGrid, send: (ToRust) -> Unit) {
    // Only visible cards are drawn: the catalog has about a hundred systems.
    LazyVerticalGrid(
        columns = GridCells.Adaptive(CARD_MIN_DP.dp),
        horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
        verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
    ) {
        item(span = { GridItemSpan(maxLineSpan) }) { Tools(grid, send) }
        if (grid.cards.isEmpty()) item(span = { GridItemSpan(maxLineSpan) }) { Small(grid.none) }
        items(grid.cards, key = { it.name }) { card -> Card(card, send) }
    }
}

@Composable
private fun Tools(grid: BrowseGrid, send: (ToRust) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
        OutlinedTextField(
            value = grid.search,
            onValueChange = { send(CatalogSearch(it)) },
            placeholder = { Small(grid.searchHint) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
            verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp),
        ) {
            grid.filters.forEach { filter ->
                Picker(filter.chosen.orEmpty(), filter.options, filter.all, { CatalogFilter(filter.kind, it) }, send)
            }
        }
    }
}

@Composable
private fun Card(card: CardView, send: (ToRust) -> Unit) {
    Boxed(Modifier.heightIn(min = TOUCH_DP.dp).clickable { send(CatalogSystem(card.name)) }) {
        Logo(card.icon, CARD_LOGO_DP)
        OneLine(card.name)
        Small(card.count)
    }
}

@Composable
private fun SystemPage(system: SystemView, send: (ToRust) -> Unit) {
    LazyColumn(verticalArrangement = Arrangement.spacedBy(Tokens.SPACE_MEDIUM.dp)) {
        item { Secondary(system.back) { send(CatalogAction("close_system")) } }
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Logo(system.icon, PAGE_LOGO_DP)
                Text(system.name, fontSize = Tokens.TEXT_TITLE.sp, modifier = Modifier.padding(start = Tokens.SPACE_LARGE.dp))
            }
        }
        items(system.variants) { variant -> Variant(variant, system.write, send) }
    }
}

@Composable
private fun Variant(variant: VariantView, write: String, send: (ToRust) -> Unit) {
    Boxed {
        Body(variant.title)
        if (variant.facts.isNotEmpty()) Small(variant.facts)
        val id = variant.id
        if (id != null) Primary(write) { send(CatalogWrite(id)) } else Small(variant.note.orEmpty())
    }
}

/** A system without a logo keeps the same room: names stay aligned. */
@Composable
private fun Logo(path: String?, side: Int) {
    val bitmap = remember(path) { path?.let(BitmapFactory::decodeFile)?.asImageBitmap() }
    Box(Modifier.size(side.dp)) {
        if (bitmap != null) Image(bitmap, contentDescription = null, modifier = Modifier.size(side.dp))
    }
}
