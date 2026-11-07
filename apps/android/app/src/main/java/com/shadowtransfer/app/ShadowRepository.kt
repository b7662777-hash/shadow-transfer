package com.shadowtransfer.app

import android.content.Context
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import kotlin.math.roundToLong

class ShadowRepository(private val context: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val prefs = context.getSharedPreferences("shadow_transfer", Context.MODE_PRIVATE)
    private val incomingRoot =
        File(context.getExternalFilesDir(null) ?: context.filesDir, "incoming").apply { mkdirs() }
    private val outgoingRoot = File(context.cacheDir, "outgoing").apply { mkdirs() }
    private val historyFile = File(context.filesDir, "history.json")

    private val transfers = linkedMapOf<String, TransferUi>()
    private val history = mutableListOf<HistoryUi>()
    private val transferPeers = mutableMapOf<String, String>()
    private var stagedPaths: List<String> = emptyList()
    private var outboundBatch: File? = null
    private var destinationUri: Uri? = null
    private var multicastLock: WifiManager.MulticastLock? = null
    private var transferWifiLock: WifiManager.WifiLock? = null
    private var autoDiscoveryJob: Job? = null
    @Volatile
    private var foreground = false
    private var messageCounter = 0L

    private val defaultName = (Build.MODEL ?: "Android Tablet").trim().take(64)
        .ifEmpty { "Android Tablet" }
    val fingerprint: String
    private val _state: MutableStateFlow<UiState>
    val state: StateFlow<UiState> get() = _state.asStateFlow()

    init {
        val storedName = prefs.getString("device_name", defaultName) ?: defaultName
        val initResult = runCatching {
            JSONObject(ShadowEngine.nativeInit(context.filesDir.absolutePath, storedName))
        }
        fingerprint = initResult.getOrNull()?.optString("fingerprint").orEmpty()
        destinationUri = prefs.getString("dest_uri", null)?.let {
            runCatching { Uri.parse(it) }.getOrNull()
        }
        loadHistory()
        _state = MutableStateFlow(
            UiState(
                deviceName = storedName,
                fingerprint = fingerprint,
                destinationLabel = destinationUri?.let { SafSaver.label(context, it) }
                    ?: "Choose receive folder",
                recent = history.toList(),
                receiverError = if (initResult.isFailure) {
                    "Rust engine failed to initialize: ${initResult.exceptionOrNull()?.message}"
                } else {
                    null
                },
            ),
        )
        refreshTrustedDevices()
        pollLoop()
    }

    private fun pollLoop() {
        scope.launch {
            while (isActive) {
                val raw = runCatching { ShadowEngine.nativePollEvent() }.getOrNull()
                if (raw == null) {
                    delay(120)
                    continue
                }
                runCatching { handleEvent(JSONObject(raw)) }
                    .onFailure { Log.w(TAG, "event handling failed", it) }
            }
        }
    }

    private fun handleEvent(event: JSONObject) {
        when (event.optString("type")) {
            "receiver_started" -> {
                _state.update {
                    it.copy(
                        receiveOn = true,
                        starting = false,
                        receiverError = null,
                        listeningPort = event.optInt("port", DEFAULT_PORT),
                    )
                }
                TransferService.start(context)
            }
            "receiver_failed" -> {
                _state.update {
                    it.copy(
                        receiveOn = false,
                        starting = false,
                        receiverError = event.optString("error"),
                    )
                }
                TransferService.stop(context)
                toast("Cannot start receiving: ${event.optString("error")}", true)
            }
            "listening" -> _state.update {
                it.copy(listeningPort = event.optInt("port", DEFAULT_PORT))
            }
            "pairing_code" -> {
                val expires = event.optInt("expires", 299)
                _state.update {
                    it.copy(
                        pairingCode = event.optString("code"),
                        pairingExpiresAt = System.currentTimeMillis() + expires * 1000L,
                    )
                }
            }
            "approval" -> handleApproval(event)
            "progress" -> handleProgress(event)
            "file_received" -> importReceived(event)
            "finished" -> handleFinished(event)
            "failed" -> handleFailed(event)
            "stopped" -> {
                _state.update {
                    it.copy(
                        receiveOn = false,
                        starting = false,
                        pairingCode = null,
                        listeningPort = null,
                    )
                }
                TransferService.stop(context)
            }
            "send_started" -> {
                val id = event.optString("transfer_id")
                val peer = event.optString("peer")
                transfers[id] = TransferUi(
                    id = id,
                    direction = "send",
                    peer = peer,
                    phase = "connecting",
                    detail = null,
                )
                refreshTransfers()
            }
            "send_finished" -> handleSendFinished(event)
            "send_failed" -> handleSendFailed(event)
            "scanning" -> _state.update { it.copy(scanning = true) }
            "devices" -> {
                val devices = event.optJSONArray("devices")?.let { array ->
                    (0 until array.length()).map { index ->
                        val item = array.optJSONObject(index) ?: JSONObject()
                        DeviceUi(
                            name = item.optString("name"),
                            addr = item.optString("addr"),
                            fingerprint = item.optString("fingerprint"),
                        )
                    }
                } ?: emptyList()
                _state.update { it.copy(devices = devices, scanning = false) }
                releaseMulticast()
                if (devices.isEmpty()) {
                    toast("No devices found. Check receive mode on the other device.", false)
                }
            }
            "scan_failed" -> {
                _state.update { it.copy(scanning = false) }
                releaseMulticast()
                toast("Discovery failed: ${event.optString("error")}", true)
            }
        }
        updateTransferWifiLock()
    }

    private fun handleApproval(event: JSONObject) {
        val transferId = event.optString("transfer_id").takeIf { it.isNotBlank() }
        val approval = ApprovalUi(
            id = event.optString("id"),
            kind = event.optString("kind"),
            device = event.optString("device"),
            fingerprint = event.optString("fingerprint"),
            transferId = transferId,
            fileCount = event.optInt("file_count"),
            totalBytes = event.optLong("total_bytes"),
            preview = event.optJSONArray("preview")?.let { array ->
                (0 until array.length()).map { array.optString(it) }
            } ?: emptyList(),
        )
        transferId?.let { transferPeers[it] = approval.device }
        _state.update { it.copy(approvals = it.approvals + approval) }
    }

    private fun handleProgress(event: JSONObject) {
        val id = event.optString("transfer_id")
        if (id.isBlank()) return
        val old = transfers[id]
        val direction = event.optString("direction", old?.direction ?: "receive")
        val peer = old?.peer ?: transferPeers[id] ?: if (direction == "receive") {
            "Incoming device"
        } else {
            "Remote device"
        }
        transfers[id] = TransferUi(
            id = id,
            direction = direction,
            peer = peer,
            phase = event.optString("phase", "transferring"),
            fileName = event.optString("file_name"),
            fileIndex = event.optInt("file_index"),
            fileCount = event.optInt("file_count"),
            fileBytes = event.optLong("file_bytes"),
            fileSize = event.optLong("file_size"),
            totalDone = event.optLong("total_done"),
            totalBytes = event.optLong("total_bytes"),
            speedBps = event.optDouble("speed_bps", 0.0),
            avgBps = event.optDouble("avg_bps", 0.0),
            etaSecs = if (event.isNull("eta_secs")) null else event.optLong("eta_secs"),
            detail = old?.detail,
            updatedAt = System.currentTimeMillis(),
        )
        refreshTransfers()
    }

    private fun handleFinished(event: JSONObject) {
        val id = event.optString("transfer_id")
        val files = event.optInt("files")
        val bytes = event.optLong("bytes")
        val duration = event.optDouble("duration", 0.0)
        val peer = transfers[id]?.peer ?: "Android device"
        val old = transfers[id] ?: TransferUi(id, "receive", peer, "completed")
        transfers[id] = old.copy(
            phase = "completed",
            totalDone = bytes,
            totalBytes = bytes,
            fileCount = files,
            detail = "BLAKE3 verified · %.1fs".format(duration),
            updatedAt = System.currentTimeMillis(),
        )
        refreshTransfers()
        addHistory(HistoryUi(System.currentTimeMillis(), "Received", peer, files, bytes, "Verified"))
        toast("Received $files file(s), ${formatBytes(bytes)}", false)
    }

    private fun importReceived(event: JSONObject) {
        val file = File(event.optString("path"))
        if (!file.exists()) return
        val tree = destinationUri
        if (tree == null) {
            toast(
                "Received ${file.name}. No destination folder selected; file is in ${file.parentFile?.absolutePath}.",
                false,
            )
            return
        }
        val relative = runCatching { file.relativeTo(incomingRoot).path }.getOrElse { file.name }
        val result = SafSaver.importFile(context, tree, file, relative)
        if (result.isSuccess) {
            file.delete()
        } else {
            toast(
                "Received, but saving to the selected folder failed: ${result.exceptionOrNull()?.message}. File kept at ${file.absolutePath}.",
                true,
            )
        }
    }

    private fun handleFailed(event: JSONObject) {
        val id = event.optString("transfer_id")
        val error = event.optString("error")
        val cancelled = error.contains("cancelled", ignoreCase = true)
        if (id.isNotBlank()) {
            transfers[id]?.let { card ->
                transfers[id] = card.copy(
                    phase = if (cancelled) "cancelled" else "failed",
                    detail = error,
                    updatedAt = System.currentTimeMillis(),
                )
                refreshTransfers()
                addHistory(
                    HistoryUi(
                        System.currentTimeMillis(),
                        if (card.direction == "send") "Sent" else "Received",
                        card.peer,
                        card.fileCount,
                        card.totalDone,
                        if (cancelled) "Cancelled" else "Failed",
                    ),
                )
            }
        }
        toast(if (cancelled) "Transfer cancelled" else "Transfer failed: $error", !cancelled)
    }

    private fun handleSendFinished(event: JSONObject) {
        val id = event.optString("transfer_id")
        val files = event.optInt("files")
        val bytes = event.optLong("bytes")
        val peer = event.optString("peer", transfers[id]?.peer ?: "Remote device")
        val old = transfers[id]
        transfers[id] = (old ?: TransferUi(id, "send", peer, "completed")).copy(
            phase = "completed",
            totalDone = bytes,
            totalBytes = bytes,
            fileCount = files,
            detail = "BLAKE3 verified on receiver",
            updatedAt = System.currentTimeMillis(),
        )
        refreshTransfers()
        addHistory(HistoryUi(System.currentTimeMillis(), "Sent", peer, files, bytes, "Verified"))
        cleanupOutbound()
        toast("Sent $files file(s), ${formatBytes(bytes)}", false)
    }

    private fun handleSendFailed(event: JSONObject) {
        val id = event.optString("transfer_id")
        val error = event.optString("error")
        val cancelled = error.contains("cancelled", ignoreCase = true)
        transfers[id]?.let { card ->
            transfers[id] = card.copy(
                phase = if (cancelled) "cancelled" else "failed",
                detail = error,
                updatedAt = System.currentTimeMillis(),
            )
            refreshTransfers()
            addHistory(
                HistoryUi(
                    System.currentTimeMillis(),
                    "Sent",
                    card.peer,
                    card.fileCount,
                    card.totalDone,
                    if (cancelled) "Cancelled" else "Failed",
                ),
            )
        }
        toast(if (cancelled) "Send cancelled" else "Send failed: $error", !cancelled)
    }

    private fun refreshTransfers() {
        val list = transfers.values.sortedWith(
            compareByDescending<TransferUi> { it.active }.thenByDescending { it.updatedAt },
        )
        _state.update { it.copy(transfers = list) }
    }

    fun setReceiveMode(enabled: Boolean) {
        if (enabled) {
            if (_state.value.receiveOn || _state.value.starting) return
            _state.update { it.copy(starting = true, receiverError = null) }
            TransferService.start(context)
            val started = runCatching {
                ShadowEngine.nativeStartReceiver(incomingRoot.absolutePath, _state.value.deviceName)
            }.getOrDefault(false)
            if (!started) {
                TransferService.stop(context)
                _state.update { it.copy(starting = false, receiveOn = false) }
                toast("Cannot start receive mode", true)
            }
        } else {
            runCatching { ShadowEngine.nativeStopReceiver() }
            TransferService.stop(context)
            releaseMulticast()
            _state.update {
                it.copy(
                    receiveOn = false,
                    starting = false,
                    pairingCode = null,
                    listeningPort = null,
                )
            }
        }
    }

    fun regenerateCode() {
        runCatching { ShadowEngine.nativeNewPairingCode() }
            .onFailure { toast("Cannot create a pairing code: ${it.message}", true) }
    }

    fun respondApproval(id: String, accept: Boolean) {
        val pairing = _state.value.approvals.firstOrNull { it.id == id }?.kind == "pairing"
        runCatching { ShadowEngine.nativeRespond(id, accept) }
            .onFailure { toast("Cannot answer request: ${it.message}", true) }
        _state.update { it.copy(approvals = it.approvals.filterNot { approval -> approval.id == id }) }
        if (pairing && accept) {
            scope.launch {
                delay(800)
                refreshTrustedDevices()
            }
        }
    }

    fun cancelTransfer(id: String) {
        runCatching { ShadowEngine.nativeCancelTransfer(id) }
            .onFailure { toast("Cannot cancel: ${it.message}", true) }
    }

    fun scanDevices() {
        if (_state.value.scanning) return
        scope.launch { runDiscoveryPass() }
    }

    fun onForeground() {
        foreground = true
        if (autoDiscoveryJob?.isActive == true) return
        autoDiscoveryJob = scope.launch {
            while (isActive && foreground) {
                val transferActive = _state.value.transfers.any { it.active }
                if (!transferActive && !_state.value.scanning) {
                    runDiscoveryPass()
                }
                delay(10_000)
            }
        }
    }

    fun onBackground() {
        foreground = false
        autoDiscoveryJob?.cancel()
        autoDiscoveryJob = null
        releaseMulticast()
        _state.update { it.copy(scanning = false) }
    }

    private suspend fun runDiscoveryPass() {
        if (_state.value.scanning) return
        acquireMulticast()
        _state.update { it.copy(scanning = true) }
        val started = runCatching { ShadowEngine.nativeDiscover() }.getOrDefault(false)
        if (!started) {
            releaseMulticast()
            _state.update { it.copy(scanning = false) }
            toast("Cannot start discovery", true)
            return
        }
        delay(4_500)
        releaseMulticast()
        _state.update { it.copy(scanning = false) }
    }

    fun refreshTrustedDevices() {
        val devices = runCatching {
            val raw = ShadowEngine.nativeListTrusted() ?: "[]"
            val array = JSONArray(raw)
            (0 until array.length()).mapNotNull { index ->
                val item = array.optJSONObject(index) ?: return@mapNotNull null
                TrustedDeviceUi(
                    name = item.optString("name"),
                    fingerprint = item.optString("fingerprint"),
                    lastSeen = item.optLong("lastSeen"),
                )
            }
        }.getOrElse {
            Log.w(TAG, "cannot read trusted devices", it)
            emptyList()
        }
        _state.update { it.copy(trustedDevices = devices) }
    }

    fun revokeTrusted(fingerprint: String) {
        val revoked = runCatching { ShadowEngine.nativeRevokeTrusted(fingerprint) }
            .getOrDefault(false)
        if (revoked) {
            toast("Device trust revoked. It must pair again.", false)
            refreshTrustedDevices()
        } else {
            toast("Cannot revoke this device", true)
        }
    }

    fun setDestination(uri: Uri) {
        try {
            context.contentResolver.takePersistableUriPermission(
                uri,
                android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    android.content.Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
            )
        } catch (error: SecurityException) {
            toast("The folder did not grant persistent access: ${error.message}", true)
            return
        }
        destinationUri = uri
        prefs.edit().putString("dest_uri", uri.toString()).apply()
        _state.update { it.copy(destinationLabel = SafSaver.label(context, uri)) }
    }

    fun stageFiles(uris: List<Uri>) {
        if (uris.isEmpty()) return
        scope.launch {
            runCatching {
                val batch = newOutboundBatch()
                Staging.copyFiles(context, uris, batch)
            }.onSuccess { paths ->
                stagedPaths = paths
                _state.update { it.copy(stagingLabel = "${paths.size} file(s) ready to send") }
            }.onFailure {
                toast("Cannot prepare files: ${it.message}", true)
            }
        }
    }

    fun stageFolder(uri: Uri) {
        scope.launch {
            runCatching {
                val batch = newOutboundBatch()
                Staging.copyFolder(context, uri, batch)
                    ?: error("Folder is not accessible")
            }.onSuccess { path ->
                stagedPaths = listOf(path)
                val name = File(path).name
                _state.update { it.copy(stagingLabel = "Folder ready: $name") }
            }.onFailure {
                toast("Cannot prepare folder: ${it.message}", true)
            }
        }
    }

    fun clearStaged() {
        stagedPaths = emptyList()
        cleanupOutbound()
        _state.update { it.copy(stagingLabel = null) }
    }

    fun sendStaged(host: String, portText: String, code: String): Boolean {
        if (stagedPaths.isEmpty()) {
            toast("Choose files or a folder first", true)
            return false
        }
        val address = host.trim()
            .removePrefix("http://")
            .removePrefix("https://")
            .substringBefore('/')
        if (address.isEmpty()) {
            toast("Enter the receiver IP address", true)
            return false
        }
        if (address.contains(':')) {
            toast("Enter only the IP address in the host field", true)
            return false
        }
        val port = portText.trim().toIntOrNull() ?: DEFAULT_PORT
        if (port !in 1..65535) {
            toast("Invalid port", true)
            return false
        }
        val id = runCatching {
            ShadowEngine.nativeSendFiles(
                address,
                port,
                JSONArray(stagedPaths).toString(),
                code.trim(),
            )
        }.getOrNull()
        if (id.isNullOrBlank()) {
            toast("Cannot start the send operation", true)
            return false
        }
        toast("Connecting to $address:$port…", false)
        return true
    }

    fun setDeviceName(name: String) {
        val clean = name.trim().take(64)
        if (clean.isEmpty()) {
            toast("Device name cannot be empty", true)
            return
        }
        prefs.edit().putString("device_name", clean).apply()
        runCatching { ShadowEngine.nativeSetDeviceName(clean) }
            .onFailure {
                toast("Cannot update name: ${it.message}", true)
                return
            }
        _state.update {
            it.copy(
                deviceName = clean,
                message = "Device name saved. Restart receive mode to advertise it.",
                messageIsError = false,
                messageId = ++messageCounter,
            )
        }
    }

    fun clearMessage() {
        _state.update { it.copy(message = null) }
    }

    private fun newOutboundBatch(): File {
        cleanupOutbound()
        val dir = File(outgoingRoot, "batch-${System.currentTimeMillis()}").apply { mkdirs() }
        outboundBatch = dir
        return dir
    }

    private fun cleanupOutbound() {
        outboundBatch?.deleteRecursively()
        outboundBatch = null
        stagedPaths = emptyList()
    }

    private fun acquireMulticast() {
        if (multicastLock?.isHeld == true) return
        val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
        multicastLock = wifi?.createMulticastLock("shadow-transfer-discovery")?.apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    private fun releaseMulticast() {
        runCatching { multicastLock?.takeIf { it.isHeld }?.release() }
        multicastLock = null
    }

    @Suppress("DEPRECATION")
    private fun updateTransferWifiLock() {
        val active = _state.value.transfers.any { it.active }
        val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
        if (active && wifi != null) {
            if (transferWifiLock?.isHeld != true) {
                transferWifiLock = wifi
                    .createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "shadow-transfer:transfer")
                    .apply {
                        setReferenceCounted(false)
                        acquire()
                    }
            }
        } else if (!active) {
            runCatching { transferWifiLock?.takeIf { it.isHeld }?.release() }
            transferWifiLock = null
        }
    }

    private fun toast(text: String, error: Boolean) {
        _state.update {
            it.copy(
                message = text,
                messageIsError = error,
                messageId = ++messageCounter,
            )
        }
    }

    private fun addHistory(entry: HistoryUi) {
        history.add(0, entry)
        while (history.size > 50) history.removeAt(history.lastIndex)
        persistHistory()
        _state.update { it.copy(recent = history.toList()) }
    }

    private fun persistHistory() {
        runCatching {
            val array = JSONArray()
            history.forEach { entry ->
                array.put(
                    JSONObject()
                        .put("at", entry.at)
                        .put("direction", entry.direction)
                        .put("peer", entry.peer)
                        .put("files", entry.files)
                        .put("bytes", entry.bytes)
                        .put("status", entry.status),
                )
            }
            historyFile.writeText(array.toString())
        }
    }

    private fun loadHistory() {
        runCatching {
            if (!historyFile.exists()) return
            val array = JSONArray(historyFile.readText())
            for (index in 0 until array.length()) {
                val entry = array.optJSONObject(index) ?: continue
                history.add(
                    HistoryUi(
                        at = entry.optLong("at"),
                        direction = entry.optString("direction"),
                        peer = entry.optString("peer"),
                        files = entry.optInt("files"),
                        bytes = entry.optLong("bytes"),
                        status = entry.optString("status"),
                    ),
                )
            }
        }
    }

    companion object {
        const val DEFAULT_PORT = 47831
        private const val TAG = "ShadowRepository"

        fun formatBytes(bytes: Long): String {
            if (bytes < 1024) return "$bytes B"
            val units = arrayOf("KiB", "MiB", "GiB", "TiB")
            var value = bytes.toDouble()
            var unit = -1
            while (value >= 1024.0 && unit < units.lastIndex) {
                value /= 1024.0
                unit += 1
            }
            return String.format("%.1f %s", value, units[unit])
        }

        fun formatSpeed(bytesPerSecond: Double): String {
            if (!bytesPerSecond.isFinite() || bytesPerSecond <= 0.0) return "0 B/s"
            return formatBytes(bytesPerSecond.roundToLong().coerceAtLeast(0)) + "/s"
        }

        fun formatEta(seconds: Long?): String = when {
            seconds == null -> "—"
            seconds < 60 -> "${seconds}s"
            seconds < 3600 -> "${seconds / 60}m ${seconds % 60}s"
            else -> "${seconds / 3600}h ${(seconds % 3600) / 60}m"
        }
    }
}
