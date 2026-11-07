package com.shadowtransfer.app

data class TransferUi(
    val id: String,
    val direction: String,
    val peer: String,
    val phase: String,
    val fileName: String = "",
    val fileIndex: Int = 0,
    val fileCount: Int = 0,
    val fileBytes: Long = 0,
    val fileSize: Long = 0,
    val totalDone: Long = 0,
    val totalBytes: Long = 0,
    val speedBps: Double = 0.0,
    val avgBps: Double = 0.0,
    val etaSecs: Long? = null,
    val detail: String? = null,
    val updatedAt: Long = System.currentTimeMillis(),
) {
    val active: Boolean
        get() = phase in setOf("connecting", "pairing", "transferring", "verifying")

    val percent: Float
        get() = if (totalBytes <= 0L) 0f
        else (totalDone.toDouble() / totalBytes.toDouble()).coerceIn(0.0, 1.0).toFloat()
}

data class ApprovalUi(
    val id: String,
    val kind: String,
    val device: String,
    val fingerprint: String = "",
    val transferId: String? = null,
    val fileCount: Int = 0,
    val totalBytes: Long = 0,
    val preview: List<String> = emptyList(),
)

data class DeviceUi(
    val name: String,
    val addr: String,
    val fingerprint: String,
)

data class HistoryUi(
    val at: Long,
    val direction: String,
    val peer: String,
    val files: Int,
    val bytes: Long,
    val status: String,
)

data class UiState(
    val deviceName: String,
    val fingerprint: String,
    val receiveOn: Boolean = false,
    val starting: Boolean = false,
    val listeningPort: Int? = null,
    val pairingCode: String? = null,
    val pairingExpiresAt: Long = 0L,
    val transfers: List<TransferUi> = emptyList(),
    val approvals: List<ApprovalUi> = emptyList(),
    val devices: List<DeviceUi> = emptyList(),
    val scanning: Boolean = false,
    val recent: List<HistoryUi> = emptyList(),
    val destinationLabel: String = "Choose receive folder",
    val stagingLabel: String? = null,
    val receiverError: String? = null,
    val message: String? = null,
    val messageIsError: Boolean = false,
    val messageId: Long = 0L,
)
