package dev.omalauncher

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.DropBoxManager
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/** One crash seen in the system's crash log, kept until the user taps its notification. */
class Crash(
    val process: String,
    val pid: String,
    val kind: String,
    val excerpt: String,
    val at: Long = System.currentTimeMillis(),
)

/**
 * Watches Android's crash records (the DropBox) for apps that crash, and announces each with a notification whose
 * tap hands the crash to the default coding agent, like Omarchy's crash watcher does with systemd-coredump.
 * Reading other apps' crashes needs READ_LOGS, which only adb can grant:
 * `adb shell pm grant <package> android.permission.READ_LOGS`. Without it the log shows nothing and this stays idle.
 */
class CrashWatch(
    private val context: Context,
    private val scope: CoroutineScope,
) {
    /** Crashes by the id a notification carries: a tap can only name one the launcher itself saw. */
    private val seen = LinkedHashMap<Int, Crash>()
    private var nextId = 1
    private val lastAnnounced = HashMap<String, Long>()

    fun crash(id: Int): Crash? = synchronized(seen) { seen[id] }

    fun start() {
        // The system's own record of crashes (its DropBox), announced to holders of READ_LOGS: no logcat, so no
        // "allow access to all device logs?" prompt. Registered at runtime, as the broadcast is only for live receivers.
        val receiver =
            object : BroadcastReceiver() {
                override fun onReceive(
                    c: Context,
                    intent: Intent,
                ) {
                    val tag = intent.getStringExtra(DropBoxManager.EXTRA_TAG) ?: return
                    if (tag !in TAGS) return
                    val time = intent.getLongExtra(DropBoxManager.EXTRA_TIME, 0L)
                    scope.launch(Dispatchers.IO) {
                        runCatching {
                            val box = context.getSystemService(DropBoxManager::class.java)
                            val entry = box.getNextEntry(tag, time - 1)
                            if (entry == null) {
                                Log.w(TAG, "crash entry $tag not readable: needs READ_LOGS and the GET_USAGE_STATS app op")
                                return@launch
                            }
                            val text = entry.use { it.getText(MAX_CHARS) } ?: return@launch
                            announce(listOf(text))
                        }.onFailure { Log.w(TAG, "crash entry unreadable", it) }
                    }
                }
            }
        runCatching {
            context.registerReceiver(receiver, IntentFilter(DropBoxManager.ACTION_DROPBOX_ENTRY_ADDED), Context.RECEIVER_NOT_EXPORTED)
        }.onFailure { Log.w(TAG, "crash watch not registered", it) }
    }

    private fun announce(group: List<String>) {
        val text = group.joinToString("\n")
        val java = JAVA_PROCESS.find(text)
        val native = NATIVE_PROCESS.find(text)
        val (name, pid, kind) =
            when {
                java != null -> Triple(java.groupValues[1], java.groupValues[2], "Java exception")
                native != null -> Triple(native.groupValues[2], native.groupValues[1], SIGNAL.find(text)?.groupValues?.get(1) ?: "native crash")
                else -> return
            }
        if (name == context.packageName) return
        val now = System.currentTimeMillis()
        // A crash loop dumps repeatedly: once per program a minute.
        val last = lastAnnounced[name]
        if (last != null && now - last < DEDUPE_MS) return
        lastAnnounced[name] = now
        val crash = Crash(name, pid, kind, text.take(MAX_CHARS))
        val id =
            synchronized(seen) {
                val id = nextId++
                seen[id] = crash
                while (seen.size > KEEP) seen.remove(seen.keys.first())
                id
            }
        notify(id, crash)
    }

    private fun notify(
        id: Int,
        crash: Crash,
    ) {
        val nm = context.getSystemService(NotificationManager::class.java)
        if (!nm.areNotificationsEnabled()) return
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Crashes", NotificationManager.IMPORTANCE_HIGH))
        val open =
            PendingIntent.getActivity(
                context,
                id,
                Intent(context, MainActivity::class.java)
                    .setAction("$ACTION_DIAGNOSE.$id")
                    .putExtra(EXTRA_CRASH, id)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        val n =
            Notification
                .Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle("Crashed: ${crash.process}")
                .setContentText("Tap to diagnose with your agent")
                .setContentIntent(open)
                .setAutoCancel(true)
                .build()
        nm.notify(CRASH_NOTIFICATION_BASE + id, n)
    }

    companion object {
        const val EXTRA_CRASH = "crash_id"
        private const val ACTION_DIAGNOSE = "dev.omalauncher.DIAGNOSE_CRASH"
        private const val CHANNEL = "crashes"
        private const val CRASH_NOTIFICATION_BASE = 5000
        private const val TAG = "CrashWatch"
        private const val DEDUPE_MS = 60_000L
        private const val MAX_CHARS = 4_000
        private const val KEEP = 20

        private val TAGS = setOf("data_app_crash", "system_app_crash", "data_app_native_crash", "system_app_native_crash", "SYSTEM_TOMBSTONE")

        // A DropBox crash entry: "Process: com.app\nPID: 1234"
        private val JAVA_PROCESS = Regex("""Process: ([\w.:]+)\s+PID: (\d+)""")

        // "DEBUG : pid: 1234, tid: 1234, name: x  >>> com.app <<<"
        private val NATIVE_PROCESS = Regex("""pid: (\d+), tid: \d+, name: \S+\s+>>> ([\w./:-]+) <<<""")
        private val SIGNAL = Regex("""signal \d+ \((\w+)""")

        /** What the agent is asked, shaped like Omarchy's `omarchy-agent-crash` prompt. */
        fun prompt(crash: Crash): String =
            """
            An app crashed on this Android phone and I want to know why.

            What the system's crash log recorded:
              process:  ${crash.process}
              PID:      ${crash.pid}
              kind:     ${crash.kind}

            The crash record (this Linux can't see Android's logs, so this is all there is):

            ${crash.excerpt}

            Work from this evidence. Say what most likely happened and what to try, and when a crash looks like a bug
            worth reporting to the app's developer. Do not change anything on the phone.
            """.trimIndent()
    }
}
