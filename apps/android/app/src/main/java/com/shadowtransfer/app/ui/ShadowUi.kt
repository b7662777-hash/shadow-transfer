package com.shadowtransfer.app.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.collectAsState
import com.shadowtransfer.app.ApprovalUi
import com.shadowtransfer.app.ShadowRepository
import com.shadowtransfer.app.TransferUi
import com.shadowtransfer.app.UiState
import kotlinx.coroutines.delay
import java.io.File

val Background = Color(0xFF070B14)
val Surface = Color(0xFF0B1220)
val Elevated = Color(0xFF101A2C)
val Outline = Color(0xFF1E2C44)
val TextPrimary = Color(0xFFE8EEF9)
val Muted = Color(0xFF8FA0B8)
val Accent = Color(0xFF3B82F6)
val AccentDark = Color(0xFF1E3A8A)
val Cyan = Color(0xFF22D3EE)
val Success = Color(0xFF34D399)
val Danger = Color(0xFFF87171)
val Warning = Color(0xFFFBBF24)

private val Scheme = darkColorScheme(
    primary = Accent,
    onPrimary = Color.White,
    secondary = Cyan,
    background = Background,
    onBackground = TextPrimary,
    surface = Surface,
    onSurface = TextPrimary,
    surfaceVariant = Elevated,
    onSurfaceVariant = Muted,
    outline = Outline,
    error = Danger,
)

@Composable
fun ShadowTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = Scheme,
        typography = MaterialTheme.typography.copy(
            headlineMedium = TextStyle(
                fontWeight = FontWeight.SemiBold,
                fontSize = 26.sp,
                letterSpacing = 1.5.sp,
            ),
            titleMedium = TextStyle(fontWeight = FontWeight.SemiBold, fontSize = 16.sp),
            bodySmall = TextStyle(fontSize = 12.sp, color = Muted),
        ),
        content = content,
    )
}

private enum class AppTab(val label: String) {
    Home("Home"),
    Activity("Activity"),
    Settings("Settings"),
}

@Composable
fun ShadowScreen(repository: ShadowRepository) {
    val state by repository.state.collectAsState()
    val snackbar = remember { SnackbarHostState() }

    LaunchedEffect(state.messageId) {
        val message = state.message ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        repository.clearMessage()
    }

    val filePicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments(),
    ) { uris -> repository.stageFiles(uris) }
    val folderPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocumentTree(),
    ) { uri -> uri?.let(repository::stageFolder) }
    val destinationPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocumentTree(),
    ) { uri -> uri?.let(repository::setDestination) }

    var tab by remember { mutableStateOf(AppTab.Home) }
    var host by rememberSaveable { mutableStateOf("") }
    var port by rememberSaveable { mutableStateOf(ShadowRepository.DEFAULT_PORT.toString()) }
    var code by rememberSaveable { mutableStateOf("") }

    Scaffold(
        containerColor = Background,
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            NavigationBar(containerColor = Surface, tonalElevation = 0.dp) {
                AppTab.entries.forEach { item ->
                    NavigationBarItem(
                        selected = tab == item,
                        onClick = { tab = item },
                        icon = {
                            Icon(
                                when (item) {
                                    AppTab.Home -> Icons.Outlined.Home
                                    AppTab.Activity -> Icons.Outlined.History
                                    AppTab.Settings -> Icons.Outlined.Settings
                                },
                                contentDescription = item.label,
                            )
                        },
                        label = { Text(item.label) },
                        colors = NavigationBarItemDefaults.colors(
                            selectedIconColor = Accent,
                            selectedTextColor = Accent,
                            indicatorColor = AccentDark,
                            unselectedIconColor = Muted,
                            unselectedTextColor = Muted,
                        ),
                    )
                }
            }
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            Header(state, onSettings = { tab = AppTab.Settings })
            BoxWithConstraints(Modifier.weight(1f)) {
                val wide = maxWidth > 900.dp
                when (tab) {
                    AppTab.Home -> HomeContent(
                        state = state,
                        repository = repository,
                        wide = wide,
                        host = host,
                        port = port,
                        code = code,
                        onHost = { host = it },
                        onPort = { port = it },
                        onCode = { code = it },
                        onPickFiles = { filePicker.launch(arrayOf("*/*")) },
                        onPickFolder = { folderPicker.launch(null) },
                        onChooseDestination = { destinationPicker.launch(null) },
                        onOpenActivity = { tab = AppTab.Activity },
                    )
                    AppTab.Activity -> ActivityContent(state, repository, wide)
                    AppTab.Settings -> SettingsContent(
                        state = state,
                        repository = repository,
                        onChooseDestination = { destinationPicker.launch(null) },
                    )
                }
            }
        }
    }

    state.approvals.firstOrNull()?.let { approval ->
        ApprovalDialog(approval, repository)
    }
}

@Composable
private fun HomeContent(
    state: UiState,
    repository: ShadowRepository,
    wide: Boolean,
    host: String,
    port: String,
    code: String,
    onHost: (String) -> Unit,
    onPort: (String) -> Unit,
    onCode: (String) -> Unit,
    onPickFiles: () -> Unit,
    onPickFolder: () -> Unit,
    onChooseDestination: () -> Unit,
    onOpenActivity: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        ConnectionOverview(
            state = state,
            onSend = onPickFiles,
            onReceive = { repository.setReceiveMode(!state.receiveOn) },
            onScan = repository::scanDevices,
        )
        if (wide) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(Modifier.weight(1f)) {
                    ReceiveCard(state, repository, onChooseFolder = onChooseDestination)
                }
                Column(Modifier.weight(1f)) {
                    SendCard(
                        state = state,
                        repository = repository,
                        host = host,
                        port = port,
                        code = code,
                        onHost = onHost,
                        onPort = onPort,
                        onCode = onCode,
                        onPickFiles = onPickFiles,
                        onPickFolder = onPickFolder,
                    )
                }
            }
        } else {
            ReceiveCard(state, repository, onChooseFolder = onChooseDestination)
            SendCard(
                state = state,
                repository = repository,
                host = host,
                port = port,
                code = code,
                onHost = onHost,
                onPort = onPort,
                onCode = onCode,
                onPickFiles = onPickFiles,
                onPickFolder = onPickFolder,
            )
        }
        TransfersCard(state, repository)
        TextButton(onClick = onOpenActivity) { Text("Open activity") }
    }
}

@Composable
private fun ActivityContent(
    state: UiState,
    repository: ShadowRepository,
    wide: Boolean,
) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (wide) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(Modifier.weight(1f)) { TransfersCard(state, repository) }
                Column(Modifier.weight(1f)) { HistoryCard(state) }
            }
        } else {
            TransfersCard(state, repository)
            HistoryCard(state)
        }
    }
}

@Composable
private fun ConnectionOverview(
    state: UiState,
    onSend: () -> Unit,
    onReceive: () -> Unit,
    onScan: () -> Unit,
) {
    SectionCard(title = "CONNECTION") {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    state.deviceName,
                    style = MaterialTheme.typography.titleMedium,
                    color = TextPrimary,
                )
                Text(
                    if (state.receiveOn) "Receiving on port ${state.listeningPort ?: ShadowRepository.DEFAULT_PORT}"
                    else "Ready to send or receive",
                    style = MaterialTheme.typography.bodySmall,
                    color = if (state.receiveOn) Success else Muted,
                )
                Text(
                    "${state.devices.size} nearby · ${state.trustedDevices.size} trusted",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Text(
                if (state.receiveOn) "ONLINE" else "IDLE",
                style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.SemiBold),
                color = if (state.receiveOn) Success else Muted,
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = onSend,
                modifier = Modifier
                    .weight(1f)
                    .height(50.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Accent),
            ) {
                Text("Send files", fontWeight = FontWeight.SemiBold)
            }
            OutlinedButton(
                onClick = onReceive,
                modifier = Modifier
                    .weight(1f)
                    .height(50.dp),
                border = BorderStroke(1.dp, if (state.receiveOn) Success else Outline),
            ) {
                Text(
                    if (state.receiveOn) "Stop receiving" else "Receive",
                    color = if (state.receiveOn) Success else TextPrimary,
                )
            }
        }
        TextButton(onClick = onScan, enabled = !state.scanning) {
            Text(if (state.scanning) "Scanning…" else "Scan for devices")
        }
    }
}

@Composable
private fun Header(state: UiState, onSettings: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                "SHADOW TRANSFER",
                style = MaterialTheme.typography.headlineMedium.copy(letterSpacing = 1.5.sp),
                color = TextPrimary,
            )
            Text(
                "${state.deviceName} · ${shortFingerprint(state.fingerprint)}",
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            )
        }
        IconButton(onClick = onSettings) {
            Icon(Icons.Outlined.Settings, contentDescription = "Settings", tint = Muted)
        }
    }
}

@Composable
private fun Header(onSettings: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text("SHADOW TRANSFER", style = MaterialTheme.typography.headlineMedium, color = TextPrimary)
            Text("Transfer Without Limits.", style = MaterialTheme.typography.bodySmall)
        }
        IconButton(onClick = onSettings) {
            Icon(Icons.Outlined.Settings, contentDescription = "Settings", tint = Muted)
        }
    }
}

@Composable
private fun SectionCard(
    title: String,
    trailing: @Composable RowScope.() -> Unit = {},
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(18.dp),
        colors = CardDefaults.cardColors(containerColor = Surface),
        border = BorderStroke(1.dp, Outline),
    ) {
        Column(
            Modifier.padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    title,
                    style = MaterialTheme.typography.labelSmall.copy(
                        fontWeight = FontWeight.SemiBold,
                        letterSpacing = 1.2.sp,
                    ),
                    color = Muted,
                    modifier = Modifier.weight(1f),
                )
                trailing()
            }
            content()
        }
    }
}

@Composable
private fun ReceiveCard(
    state: UiState,
    repository: ShadowRepository,
    onChooseFolder: () -> Unit,
) {
    SectionCard(
        title = "RECEIVE",
        trailing = {
            Switch(
                checked = state.receiveOn || state.starting,
                enabled = !state.starting,
                onCheckedChange = repository::setReceiveMode,
                colors = SwitchDefaults.colors(
                    checkedThumbColor = Color.White,
                    checkedTrackColor = AccentDark,
                    uncheckedTrackColor = Elevated,
                    uncheckedBorderColor = Outline,
                ),
            )
        },
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                when {
                    state.starting -> "Starting receiver…"
                    state.receiveOn -> "Listening on port ${state.listeningPort ?: ShadowRepository.DEFAULT_PORT}"
                    else -> "Off"
                },
                color = when {
                    state.starting -> Warning
                    state.receiveOn -> Success
                    else -> Muted
                },
                style = MaterialTheme.typography.bodySmall,
            )
        }
        state.receiverError?.let { error ->
            Text(error, color = Danger, style = MaterialTheme.typography.bodySmall)
        }
        Text("Device name", style = MaterialTheme.typography.labelSmall, color = Muted)
        Text(state.deviceName, style = MaterialTheme.typography.titleMedium)
        if (state.receiveOn && state.pairingCode != null) {
            PairingCodeBlock(state.pairingCode, state.pairingExpiresAt, repository::regenerateCode)
        }
        HorizontalDivider(color = Outline)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Save received files to", style = MaterialTheme.typography.labelSmall, color = Muted)
                Text(state.destinationLabel, style = MaterialTheme.typography.bodySmall)
            }
            TextButton(onClick = onChooseFolder) { Text("Choose folder") }
        }
        Text(
            "Received files are staged in app storage first, verified with BLAKE3, then copied to the folder you choose.",
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@Composable
private fun PairingCodeBlock(code: String, expiresAt: Long, onNewCode: () -> Unit) {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(expiresAt) {
        while (true) {
            now = System.currentTimeMillis()
            delay(1000)
        }
    }
    val remaining = ((expiresAt - now) / 1000L).coerceAtLeast(0)
    Card(
        colors = CardDefaults.cardColors(containerColor = Elevated),
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("PAIRING CODE", style = MaterialTheme.typography.labelSmall, color = Muted)
            Text(
                code,
                color = if (remaining > 0) Accent else Muted,
                style = TextStyle(
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 34.sp,
                    letterSpacing = 5.sp,
                ),
            )
            Text(
                if (remaining > 0) {
                    "Expires in ${remaining}s · enter it on the sending device"
                } else {
                    "Code expired · create a new one"
                },
                style = MaterialTheme.typography.bodySmall,
            )
            TextButton(onClick = onNewCode) { Text("New code") }
        }
    }
}

@Composable
private fun SendCard(
    state: UiState,
    repository: ShadowRepository,
    host: String,
    port: String,
    code: String,
    onHost: (String) -> Unit,
    onPort: (String) -> Unit,
    onCode: (String) -> Unit,
    onPickFiles: () -> Unit,
    onPickFolder: () -> Unit,
) {
    SectionCard(title = "SEND") {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = onPickFiles) { Text("Choose files") }
            OutlinedButton(onClick = onPickFolder) { Text("Choose folder") }
        }
        state.stagingLabel?.let { label ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(label, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
                TextButton(onClick = repository::clearStaged) { Text("Clear") }
            }
        }
        HorizontalDivider(color = Outline)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                "Nearby devices",
                style = MaterialTheme.typography.labelSmall,
                color = Muted,
                modifier = Modifier.weight(1f),
            )
            TextButton(
                onClick = repository::scanDevices,
                enabled = !state.scanning,
            ) {
                Text(if (state.scanning) "Scanning…" else "Scan")
            }
        }
        state.devices.forEach { device ->
            val deviceHost = device.addr.substringBeforeLast(':')
            val devicePort = device.addr.substringAfterLast(':', "47831")
            OutlinedButton(
                onClick = {
                    onHost(deviceHost)
                    onPort(devicePort)
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(Modifier.fillMaxWidth()) {
                    Text(device.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(device.addr, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        OutlinedTextField(
            value = host,
            onValueChange = onHost,
            label = { Text("Receiver IP address") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
            colors = fieldColors(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = port,
                onValueChange = onPort,
                label = { Text("Port") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                modifier = Modifier.width(110.dp),
                colors = fieldColors(),
            )
            OutlinedTextField(
                value = code,
                onValueChange = { onCode(it.uppercase().take(9)) },
                label = { Text("Pairing code") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.Characters,
                    keyboardType = KeyboardType.Ascii,
                ),
                modifier = Modifier.weight(1f),
                colors = fieldColors(),
            )
        }
        Button(
            onClick = { repository.sendStaged(host, port, code) },
            enabled = state.stagingLabel != null && host.isNotBlank() && !state.scanning,
            modifier = Modifier
                .fillMaxWidth()
                .height(48.dp),
            colors = ButtonDefaults.buttonColors(containerColor = Accent),
        ) {
            Text("Send", fontWeight = FontWeight.SemiBold)
        }
        Text(
            "The first connection needs the 8-character code shown on the receiving device.",
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@Composable
private fun TransfersCard(state: UiState, repository: ShadowRepository) {
    SectionCard(title = "TRANSFERS") {
        if (state.transfers.isEmpty()) {
            Text("No transfers yet.", style = MaterialTheme.typography.bodySmall)
        }
        state.transfers.take(12).forEach { transfer ->
            TransferRow(transfer) { repository.cancelTransfer(transfer.id) }
        }
    }
}

@Composable
private fun TransferRow(transfer: TransferUi, onCancel: () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                if (transfer.direction == "send") "SEND" else "RECEIVE",
                style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.Bold),
                color = if (transfer.direction == "send") Accent else Success,
            )
            Text(
                "  ${transfer.peer}",
                style = MaterialTheme.typography.bodySmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Text(phaseLabel(transfer.phase), style = MaterialTheme.typography.bodySmall, color = phaseColor(transfer.phase))
        }
        Text(
            if (transfer.fileName.isBlank()) "${transfer.fileCount} file(s)" else transfer.fileName,
            style = MaterialTheme.typography.bodyMedium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        LinearProgressIndicator(
            progress = transfer.percent,
            modifier = Modifier
                .fillMaxWidth()
                .height(6.dp),
            color = Accent,
            trackColor = Outline,
        )
        Row {
            Text(
                "${ShadowRepository.formatBytes(transfer.totalDone)} / ${ShadowRepository.formatBytes(transfer.totalBytes)}",
                style = MaterialTheme.typography.bodySmall,
            )
            if (transfer.active) {
                Text(
                    "   ${ShadowRepository.formatSpeed(transfer.speedBps)} · ETA ${ShadowRepository.formatEta(transfer.etaSecs)}",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
        transfer.detail?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = phaseColor(transfer.phase))
        }
        if (transfer.active) {
            TextButton(onClick = onCancel) { Text("Cancel") }
        }
        HorizontalDivider(color = Outline)
    }
}

@Composable
private fun HistoryCard(state: UiState) {
    SectionCard(title = "RECENT") {
        if (state.recent.isEmpty()) {
            Text("Completed transfers will appear here.", style = MaterialTheme.typography.bodySmall)
        }
        state.recent.take(12).forEach { entry ->
            Column(Modifier.padding(vertical = 3.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        entry.direction,
                        style = MaterialTheme.typography.labelSmall,
                        color = if (entry.direction == "Sent") Accent else Success,
                    )
                    Text(
                        "  ${entry.peer}",
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Text(
                        entry.status,
                        style = MaterialTheme.typography.bodySmall,
                        color = when (entry.status) {
                            "Verified" -> Success
                            "Failed" -> Danger
                            "Cancelled" -> Warning
                            else -> Muted
                        },
                    )
                }
                Text(
                    "${relativeTime(entry.at)} · ${entry.files} file(s) · ${ShadowRepository.formatBytes(entry.bytes)}",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}

@Composable
private fun ApprovalDialog(approval: ApprovalUi, repository: ShadowRepository) {
    AlertDialog(
        onDismissRequest = {},
        containerColor = Surface,
        title = {
            Text(
                if (approval.kind == "pairing") "Pair this device?" else "Incoming transfer",
                color = TextPrimary,
            )
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                if (approval.kind == "pairing") {
                    Text("${approval.device} wants to pair with this tablet.", color = TextPrimary)
                    Text(
                        approval.fingerprint,
                        style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                    )
                    Text(
                        "Only approve if you entered this tablet's code on that device and you recognise it.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                } else {
                    Text(
                        "${approval.device} wants to send ${approval.fileCount} file(s), " +
                            ShadowRepository.formatBytes(approval.totalBytes) + ".",
                        color = TextPrimary,
                    )
                    approval.preview.take(6).forEach { name ->
                        Text("• $name", style = MaterialTheme.typography.bodySmall)
                    }
                    if (approval.fileCount > approval.preview.size) {
                        Text(
                            "… and ${approval.fileCount - approval.preview.size} more",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { repository.respondApproval(approval.id, true) }) {
                Text("Approve", color = Accent)
            }
        },
        dismissButton = {
            TextButton(onClick = { repository.respondApproval(approval.id, false) }) {
                Text("Decline", color = Muted)
            }
        },
    )
}

@Composable
private fun SettingsContent(
    state: UiState,
    repository: ShadowRepository,
    onChooseDestination: () -> Unit,
) {
    var name by remember(state.deviceName) { mutableStateOf(state.deviceName) }
    LaunchedEffect(Unit) { repository.refreshTrustedDevices() }

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        SectionCard(title = "DEVICE") {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it.take(64) },
                label = { Text("Device name") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
                colors = fieldColors(),
            )
            Button(
                onClick = { repository.setDeviceName(name) },
                colors = ButtonDefaults.buttonColors(containerColor = Accent),
            ) {
                Text("Save device name")
            }
            Text(
                "Changing the name takes effect after restarting receive mode.",
                style = MaterialTheme.typography.bodySmall,
            )
        }

        SectionCard(title = "STORAGE") {
            Text(state.destinationLabel, color = TextPrimary)
            OutlinedButton(onClick = onChooseDestination) { Text("Choose receive folder") }
            Text(
                "Received files are verified in app staging before they are copied into this folder.",
                style = MaterialTheme.typography.bodySmall,
            )
        }

        SectionCard(title = "IDENTITY") {
            Text(
                state.fingerprint,
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
            )
            Text(
                "The identity key stays in this app's private storage and never leaves the device.",
                style = MaterialTheme.typography.bodySmall,
            )
        }

        SectionCard(title = "TRUSTED DEVICES") {
            if (state.trustedDevices.isEmpty()) {
                Text("No paired devices yet.", style = MaterialTheme.typography.bodySmall)
            }
            state.trustedDevices.forEach { device ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(device.name, color = TextPrimary)
                        Text(
                            shortFingerprint(device.fingerprint),
                            style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                            color = Muted,
                        )
                    }
                    TextButton(onClick = { repository.revokeTrusted(device.fingerprint) }) {
                        Text("Remove", color = Danger)
                    }
                }
            }
            Text(
                "Removing a device requires it to pair again before its next transfer.",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun fieldColors() = OutlinedTextFieldDefaults.colors(
    focusedBorderColor = Accent,
    unfocusedBorderColor = Outline,
    focusedTextColor = TextPrimary,
    unfocusedTextColor = TextPrimary,
    focusedLabelColor = Accent,
    unfocusedLabelColor = Muted,
    cursorColor = Accent,
)

private fun phaseLabel(phase: String): String = when (phase) {
    "connecting" -> "Connecting"
    "pairing" -> "Pairing"
    "transferring" -> "Transferring"
    "verifying" -> "Verifying"
    "completed" -> "Verified"
    "failed" -> "Failed"
    "cancelled" -> "Cancelled"
    else -> phase
}

private fun phaseColor(phase: String): Color = when (phase) {
    "completed" -> Success
    "failed" -> Danger
    "cancelled" -> Warning
    "verifying" -> Warning
    "transferring" -> Accent
    else -> Muted
}

private fun shortFingerprint(value: String): String =
    if (value.length <= 20) value else "${value.take(12)}…${value.takeLast(4)}"

private fun relativeTime(at: Long): String {
    val seconds = ((System.currentTimeMillis() - at) / 1000L).coerceAtLeast(0)
    return when {
        seconds < 60 -> "${seconds}s ago"
        seconds < 3600 -> "${seconds / 60}m ago"
        seconds < 86400 -> "${seconds / 3600}h ago"
        else -> "${seconds / 86400}d ago"
    }
}
