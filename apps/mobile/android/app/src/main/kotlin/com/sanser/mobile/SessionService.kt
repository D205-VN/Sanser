package com.sanser.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Keeps an open remote session (sound, voice, chat) running while Sanser is
 * in the background, with the ongoing notification Android requires. It owns
 * no media itself: the activity's native client keeps doing the work.
 */
class SessionService : Service() {
    companion object {
        private const val ONGOING = 7302
        const val SESSION_CHANNEL = "sanser_session"
        const val MESSAGE_CHANNEL = "sanser_messages"
        private const val EXTRA_MICROPHONE = "microphone"
        fun channels(context: Context) {
            val manager = context.getSystemService(NotificationManager::class.java) ?: return
            manager.createNotificationChannel(NotificationChannel(SESSION_CHANNEL, "Phiên đang mở", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Hiện khi phiên điều khiển hoặc xem màn hình vẫn mở lúc Sanser ở nền."
                setShowBadge(false)
            })
            manager.createNotificationChannel(NotificationChannel(MESSAGE_CHANNEL, "Tin nhắn và kết nối", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "Tin nhắn chat và báo mất kết nối khi Sanser ở nền."
            })
        }
        /** Must be called while the activity is visible (Android 12+ rule). */
        fun start(context: Context, microphone: Boolean) {
            val intent = Intent(context, SessionService::class.java).putExtra(EXTRA_MICROPHONE, microphone)
            try { context.startForegroundService(intent) } catch (_: Exception) { /* Background start refused: session still runs while visible. */ }
        }
        fun stop(context: Context) { context.stopService(Intent(context, SessionService::class.java)) }
        fun open(context: Context): PendingIntent = PendingIntent.getActivity(
            context, 0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        channels(this)
        val microphone = intent?.getBooleanExtra(EXTRA_MICROPHONE, false) == true &&
            checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED
        val notification = Notification.Builder(this, SESSION_CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("Sanser đang kết nối")
            .setContentText(if (microphone) "Phiên vẫn mở · micro đang bật" else "Phiên vẫn mở · chạm để quay lại")
            .setOngoing(true)
            .setContentIntent(open(this))
            .build()
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                if (microphone && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                startForeground(ONGOING, notification, types)
            } else startForeground(ONGOING, notification)
        } catch (_: Exception) { stopSelf() }
        return START_NOT_STICKY
    }

    // Swiping Sanser away from recents ends the session with it.
    override fun onTaskRemoved(rootIntent: Intent?) { stopSelf(); super.onTaskRemoved(rootIntent) }
}
