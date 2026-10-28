package dev.lancem.peegel.ui

import android.text.format.DateUtils
import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.lancem.peegel.sync.ClipFiles
import dev.lancem.peegel.sync.ClipItem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import dev.lancem.peegel.sync.PairInfo
import dev.lancem.peegel.sync.Status

data class SetupState(
    val notificationsGranted: Boolean = true,
    val backgroundAllowed: Boolean = true,
    val canAddTile: Boolean = false,
)

class ScreenActions(
    val scan: () -> Unit,
    val pairWithLink: (String) -> Boolean,
    val confirmPair: (PairInfo) -> Unit,
    val cancelPair: () -> Unit,
    val sendClipboard: () -> Unit,
    val recopy: (ClipItem) -> Unit,
    val setPaused: (Boolean) -> Unit,
    val unpair: () -> Unit,
    val allowNotifications: () -> Unit,
    val allowBackground: () -> Unit,
    val addTile: () -> Unit,
)

private val Online = Color(0xFF22C55E)
private val Waiting = Color(0xFFF59E0B)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PeegelScreen(
    status: Status,
    paired: Boolean,
    history: List<ClipItem>,
    paused: Boolean,
    setup: SetupState,
    pendingPair: PairInfo?,
    actions: ScreenActions,
) {
    var menuOpen by remember { mutableStateOf(false) }
    var confirmUnpair by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Peegel", fontWeight = FontWeight.SemiBold) },
                actions = {
                    if (paired) {
                        Box {
                            IconButton(onClick = { menuOpen = true }) {
                                Icon(Icons.Default.MoreVert, contentDescription = "More")
                            }
                            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                                DropdownMenuItem(
                                    text = { Text("Pair with another computer") },
                                    onClick = { menuOpen = false; actions.scan() },
                                )
                                DropdownMenuItem(
                                    text = { Text("Unpair") },
                                    onClick = { menuOpen = false; confirmUnpair = true },
                                )
                            }
                        }
                    }
                },
            )
        },
    ) { padding ->
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = 16.dp,
                end = 16.dp,
                top = padding.calculateTopPadding() + 4.dp,
                bottom = padding.calculateBottomPadding() + 24.dp,
            ),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item { StatusCard(status, paused) }

            if (status is Status.NotPaired || status is Status.Error) {
                item { PairCard(actions) }
            }

            if (status is Status.Connected || status is Status.Connecting) {
                item {
                    Button(
                        onClick = actions.sendClipboard,
                        modifier = Modifier.fillMaxWidth().height(56.dp),
                        enabled = status is Status.Connected,
                        shape = RoundedCornerShape(16.dp),
                    ) {
                        Icon(Icons.AutoMirrored.Filled.Send, contentDescription = null)
                        Spacer(Modifier.width(10.dp))
                        Text("Send clipboard to computer")
                    }
                }
                item {
                    SettingRow(
                        title = "Receive from computer",
                        subtitle = if (paused) "Paused" else "Copies on your computer land here automatically",
                        checked = !paused,
                        onChange = { actions.setPaused(!it) },
                    )
                }
            }

            if (status !is Status.NotPaired) {
                val tips = buildList {
                    if (!setup.notificationsGranted) add(Triple("Show the status notification", "It has a one‑tap Send clipboard button.", actions.allowNotifications))
                    if (!setup.backgroundAllowed) add(Triple("Keep running in the background", "Stops Samsung battery saving from disconnecting Peegel.", actions.allowBackground))
                    if (setup.canAddTile) add(Triple("Add the Quick Settings tile", "Swipe down and tap to send your clipboard.", actions.addTile))
                }
                items(tips) { (title, text, action) -> TipCard(title, text, action) }
            }

            if (history.isNotEmpty()) {
                item {
                    Text(
                        "Recent",
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 8.dp, start = 4.dp),
                    )
                }
                items(history, key = { it.id }) { item -> ClipRow(item) { actions.recopy(item) } }
            }
        }
    }

    if (pendingPair != null) {
        AlertDialog(
            onDismissRequest = actions.cancelPair,
            title = { Text("Pair with ${pendingPair.serverName}?") },
            text = { Text("Clipboard contents will be shared between this phone and ${pendingPair.serverName}.") },
            confirmButton = { TextButton(onClick = { actions.confirmPair(pendingPair) }) { Text("Pair") } },
            dismissButton = { TextButton(onClick = actions.cancelPair) { Text("Cancel") } },
        )
    }

    if (confirmUnpair) {
        AlertDialog(
            onDismissRequest = { confirmUnpair = false },
            title = { Text("Unpair?") },
            text = { Text("This phone will stop syncing. You can pair again by scanning a new code.") },
            confirmButton = { TextButton(onClick = { confirmUnpair = false; actions.unpair() }) { Text("Unpair") } },
            dismissButton = { TextButton(onClick = { confirmUnpair = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun StatusCard(status: Status, paused: Boolean) {
    val (title, subtitle, color) = when (status) {
        is Status.Connected -> Triple("Connected to ${status.name}", if (paused) "Receiving is paused" else "Clipboard is in sync", Online)
        is Status.Connecting -> Triple("Looking for ${status.name}…", "Make sure both devices are on the same Wi‑Fi", Waiting)
        is Status.Pairing -> Triple("Pairing with ${status.name}…", "This takes a second", Waiting)
        is Status.Error -> Triple("Something went wrong", status.message, MaterialTheme.colorScheme.error)
        Status.NotPaired -> Triple("Not paired yet", "Pair with your computer to start syncing", MaterialTheme.colorScheme.outline)
    }
    val dot by animateColorAsState(color, label = "status")
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(20.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHigh),
    ) {
        Row(Modifier.padding(20.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(12.dp).clip(CircleShape).background(dot))
            Spacer(Modifier.width(14.dp))
            Column {
                Text(title, style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(2.dp))
                Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun PairCard(actions: ScreenActions) {
    var showLinkField by remember { mutableStateOf(false) }
    var link by remember { mutableStateOf("") }
    Card(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(20.dp)) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Pair with your computer", style = MaterialTheme.typography.titleMedium)
            Text(
                "Open Peegel on your computer, go to Devices and scan the QR code shown there.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Button(onClick = actions.scan, modifier = Modifier.fillMaxWidth()) { Text("Scan QR code") }
            if (showLinkField) {
                OutlinedTextField(
                    value = link,
                    onValueChange = { link = it },
                    label = { Text("peegel://pair?…") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                FilledTonalButton(
                    onClick = { if (actions.pairWithLink(link)) showLinkField = false },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Pair") }
            } else {
                TextButton(onClick = { showLinkField = true }) { Text("Paste a pairing link instead") }
            }
        }
    }
}

@Composable
private fun SettingRow(title: String, subtitle: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .clickable { onChange(!checked) }
            .padding(horizontal = 4.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(checked = checked, onCheckedChange = onChange)
    }
}

@Composable
private fun TipCard(title: String, text: String, action: () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
    ) {
        Row(Modifier.padding(start = 16.dp, top = 12.dp, bottom = 12.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium)
                Text(text, style = MaterialTheme.typography.bodySmall)
            }
            TextButton(onClick = action) { Text("Allow") }
        }
    }
}

@Composable
private fun ClipRow(item: ClipItem, onClick: () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
        shape = RoundedCornerShape(14.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
    ) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier
                    .size(36.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Text(if (item.fromPhone) "📱" else "💻")
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                if (item.imagePath != null) {
                    val thumb by produceState<ImageBitmap?>(null, item.imagePath) {
                        value = withContext(Dispatchers.IO) { ClipFiles.thumbnail(item.imagePath)?.asImageBitmap() }
                    }
                    thumb?.let {
                        Image(
                            bitmap = it,
                            contentDescription = "Image",
                            contentScale = ContentScale.Fit,
                            modifier = Modifier
                                .heightIn(max = 140.dp)
                                .clip(RoundedCornerShape(8.dp)),
                        )
                    } ?: Text("Image", style = MaterialTheme.typography.bodyMedium)
                } else {
                    Text(item.text, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                }
                Text(
                    (if (item.fromPhone) "Sent from this phone" else "From computer") + " · " +
                        DateUtils.getRelativeTimeSpanString(item.ts, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
