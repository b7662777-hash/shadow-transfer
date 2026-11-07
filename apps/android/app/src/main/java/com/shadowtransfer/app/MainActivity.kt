package com.shadowtransfer.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.shadowtransfer.app.ui.ShadowScreen
import com.shadowtransfer.app.ui.ShadowTheme

class MainActivity : ComponentActivity() {
    private val repository: ShadowRepository
        get() = (application as ShadowApplication).repository

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestRuntimePermissions()
        setContent {
            ShadowTheme {
                ShadowScreen(repository)
            }
        }
    }

    override fun onStart() {
        super.onStart()
        repository.onForeground()
    }

    override fun onStop() {
        repository.onBackground()
        super.onStop()
    }

    private fun requestRuntimePermissions() {
        val missing = mutableListOf<String>()
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            missing.add(Manifest.permission.POST_NOTIFICATIONS)
        }
        if (Build.VERSION.SDK_INT >= 37) {
            val localNetwork = "android.permission.ACCESS_LOCAL_NETWORK"
            if (ContextCompat.checkSelfPermission(this, localNetwork) !=
                PackageManager.PERMISSION_GRANTED
            ) {
                missing.add(localNetwork)
            }
        }
        if (missing.isNotEmpty()) {
            ActivityCompat.requestPermissions(this, missing.toTypedArray(), REQUEST_PERMISSIONS)
        }
    }

    companion object {
        private const val REQUEST_PERMISSIONS = 42
    }
}
