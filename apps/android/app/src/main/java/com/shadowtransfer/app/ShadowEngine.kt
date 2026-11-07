package com.shadowtransfer.app

object ShadowEngine {
    init {
        System.loadLibrary("shadow_android")
    }

    external fun nativeInit(filesDir: String, deviceName: String): String
    external fun nativeFingerprint(): String?
    external fun nativeSetDeviceName(name: String): Boolean
    external fun nativeStartReceiver(destination: String, deviceName: String): Boolean
    external fun nativeStopReceiver(): Boolean
    external fun nativePairingCode(): String?
    external fun nativeNewPairingCode(): String?
    external fun nativeRespond(approvalId: String, accept: Boolean): Boolean
    external fun nativeCancelTransfer(transferId: String): Boolean
    external fun nativeSendFiles(
        host: String,
        port: Int,
        pathsJson: String,
        pairingCode: String,
    ): String?
    external fun nativeDiscover(): Boolean
    external fun nativePollEvent(): String?
}
