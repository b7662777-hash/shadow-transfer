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

val Background = Color(0xFF07080A)
val Surface = Color(0xFF12141A)
val Elevated = Color(0xFF191C23)
val Outline = Color(0xFF2A2E37)
val TextPrimary = Color(0xFFE7E9EF)
val Muted = Color(0xFF8E94A0)
val Accent = Color(0xFF8F7EFF)
val AccentDark = Color(0xFF3E337A)
val Success = Color(0xFF4ADE80)
val Danger = Color(0xFFF87171)
val Warning = Color(0xFFFACC15)

private val Scheme = darkColorScheme(
    primary = Accent,
    onPrimary = Color.White,
    secondary = Success,
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

    var showSettings by remember { mutableStateOf(false) }
    var host by rememberSaveable { mutableStateOf("") }
    var port by rememberSaveable { mutableStateOf(ShadowRepository.DEFAULT_PORT.toString()) }
    var code by rememberSaveable { mutableStateOf("") }

    Scaffold(
        containerColor = Background,
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        BoxWithConstraints(
            Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            val wide = maxWidth > 900.dp
            if (wide) {
                Row(
                    Modifier
                        .fillMaxSize()
                        .padding(16.dp),
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Column(
                        Modifier
                            .weight(1f)
                            .verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Header(onSettings = { showSettings = true })
                        ReceiveCard(state, repository, onChooseFolder = { destinationPicker.launch(null) })
                        SendCard(
                            state = state,
                            repository = repository,
                            host = host,
                            port = port,
                            code = code,
                            onHost = { host = it },
                            onPort = { port = it },
                            onCode = { code = it },
                            onPickFiles = { filePicker.launch(arrayOf("*/*")) },
                            onPickFolder = { folderPicker.launch(null) },
                        )
                    }
                    Column(
                        Modifier
                            .weight(1f)
                            .verticalScroll(rememberScrollState()),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        HeaderMeta(state)
                        TransfersCard(state, repository)
                        HistoryCard(state)
                    }
                }
            } else {
                Column(
                    Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Header(onSettings = { showSettings = true })
                    HeaderMeta(state)
                    ReceiveCard(state, repository, onChooseFolder = { destinationPicker.launch(null) })
                    SendCard(
                        state = state,
                        repository = repository,
                        host = host,
                        port = port,
                        code = code,
                        onHost = { host = it },
                        onPort = { port = it },
                        onCode = { code = it },
                        onPickFiles = { filePicker.launch(arrayOf("*/*")) },
                        onPickFolder = { folderPicker.launch(null) },
                    )
                    TransfersCard(state, repository)
                    HistoryCard(state)
                }
            }
        }
    }

    state.approvals.firstOrNull()?.let { approval ->
        ApprovalDialog(approval, repository)
    }
    if (showSettings) {
        SettingsDialog(state, repository) { showSettings = false }
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
private fun HeaderMeta(state: UiState) {
    Text(
        if (state.fingerprint.isBlank()) "Identity unavailable" else
            "Device fingerprint ${shortFingerprint(state.fingerprint)}",
        style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
    )
}

@Composable
private fun SectionCard(
    title: String,
    trailing: @Composable RowScope.() -> Unit = {},
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        colors = CardDefaults.cardColors(containerColor = Surface),
        border = BorderStroke(1.dp, Outline),
    ) {
        Column(
            Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
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
            colors = ButtonDefaults.buttonColors(containerColor = AccentDark),
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
private fun SettingsDialog(state: UiState, repository: ShadowRepository, onClose: () -> Unit) {
    var name by remember { mutableStateOf(state.deviceName) }
    AlertDialog(
        onDismissRequest = onClose,
        containerColor = Surface,
        title = { Text("Settings", color = TextPrimary) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it.take(64) },
                    label = { Text("Device name") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                    colors = fieldColors(),
                )
                Text("IDENTITY FINGERPRINT", style = MaterialTheme.typography.labelSmall, color = Muted)
                Text(
                    state.fingerprint,
                    style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                )
                Text(
                    "The identity key is stored in this app's private storage and never leaves the device.",
                    style = MaterialTheme.typography.bodySmall,
                )
                Text(
                    "Receive folder: ${state.destinationLabel}",
                    style = MaterialTheme.typography.bodySmall,
                )
                Text(
                    "Changing the device name takes effect after restarting receive mode.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = {
                repository.setDeviceName(name)
                onClose()
            }) { Text("Save", color = Accent) }
        },
        dismissButton = {
            TextButton(onClick = onClose) { Text("Close", color = Muted) }
        },
    )
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
