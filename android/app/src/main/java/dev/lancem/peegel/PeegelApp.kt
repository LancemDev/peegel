package dev.lancem.peegel

import android.app.Application
import android.content.Context
import dev.lancem.peegel.sync.Store
import dev.lancem.peegel.sync.Sync

class PeegelApp : Application() {
    lateinit var sync: Sync
        private set

    override fun onCreate() {
        super.onCreate()
        sync = Sync(this, Store(this))
    }
}

val Context.app: PeegelApp get() = applicationContext as PeegelApp
