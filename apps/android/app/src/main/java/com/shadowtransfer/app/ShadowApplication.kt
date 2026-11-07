package com.shadowtransfer.app

import android.app.Application

class ShadowApplication : Application() {
    lateinit var repository: ShadowRepository
        private set

    override fun onCreate() {
        super.onCreate()
        repository = ShadowRepository(this)
    }
}
