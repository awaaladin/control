package app.rhcs.pulse

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.pm.PackageInfoCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runInterruptible
import java.io.File
import java.io.FileOutputStream

/**
 * Runs the PULSE engine (rhcsd, shipped as librhcsd.so) as a child process, keeps it alive, and
 * feeds it this phone's own cell and location readings. Foreground service, so Android keeps it
 * running with the screen off.
 *
 * The engine ending with exit code 0 (the "Stop" button in PULSE) or the Stop action on the
 * notification means "stopped by the user": it is not restarted, the notification says so and
 * offers Start. Any other end is a crash and restarts with a growing pause.
 */
class EngineService : Service() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var proc: Process? = null
    private var superviseJob: Job? = null
    private var begun = false
    private var reporter: SignalReporter? = null
    @Volatile private var userStop = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                goForeground("Stopped by you", running = false)
                stopEngine()
                return START_STICKY
            }
            ACTION_QUIT -> {
                stopSelf()
                return START_NOT_STICKY
            }
        }
        goForeground(if (Engine.state == "stopped" && Engine.stoppedByUser) "Starting…" else "Reading this phone's signal", running = true)
        if (!Engine.supported()) {
            Engine.state = "error"
            Engine.error = "this phone is 32-bit: the built-in engine is not supported"
            stopSelf()
            return START_NOT_STICKY
        }
        if (!begun) {
            begun = true
            // Location listeners must be registered from a thread with a looper: this is the main thread.
            reporter = SignalReporter(applicationContext).also { it.start(scope) }
            LookupWorker(applicationContext).start(scope)
        }
        if (intent?.action == null && !Engine.enabled(this)) {
            // Android restarted the service after a kill, but the user had switched the engine off.
            Engine.state = "stopped"
            Engine.stoppedByUser = true
            goForeground("Stopped by you", running = false)
            return START_STICKY
        }
        startEngine()
        return START_STICKY
    }

    override fun onDestroy() {
        reporter?.stop()
        userStop = true
        scope.cancel()
        proc?.destroy()
        proc = null
        begun = false
        Engine.state = "stopped"
        super.onDestroy()
    }

    // ------------------------------------------------------------------ start / stop

    private fun startEngine() {
        if (superviseJob?.isActive == true) return
        userStop = false
        Engine.stoppedByUser = false
        setEnabled(true)
        superviseJob = scope.launch { supervise() }
    }

    /** The user's stop: the process is ended and not restarted. */
    private fun stopEngine() {
        userStop = true
        setEnabled(false)
        proc?.destroy()
        if (superviseJob?.isActive != true) markStopped()
    }

    private fun setEnabled(on: Boolean) = Engine.setEnabled(this, on)

    private fun markStopped() {
        Engine.state = "stopped"
        Engine.error = null
        Engine.stoppedByUser = true
        setEnabled(false)
        goForeground("Stopped by you", running = false)
    }

    // ------------------------------------------------------------------ foreground notification

    private fun goForeground(text: String, running: Boolean) {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "PULSE engine", NotificationManager.IMPORTANCE_LOW))
        }
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val action = if (running) {
            PendingIntent.getService(
                this, 1, Intent(this, EngineService::class.java).setAction(ACTION_STOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        } else {
            PendingIntent.getService(
                this, 2, Intent(this, EngineService::class.java).setAction(ACTION_START),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        }
        val n: Notification = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_launcher)
            .setContentTitle(if (running) "PULSE engine is on" else "PULSE engine is off")
            .setContentText(if (running) "$text. Everything stays on this phone." else text)
            .setOngoing(true)
            .setContentIntent(open)
            .addAction(0, if (running) "Stop" else "Start", action)
            .build()
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                var type = ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
                if (checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED ||
                    checkSelfPermission(android.Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED
                ) {
                    type = type or ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION
                }
                startForeground(NOTIF_ID, n, type)
            } else {
                startForeground(NOTIF_ID, n)
            }
        } catch (e: Exception) {
            Engine.state = "error"
            Engine.error = "Android refused to start the background service: ${e.javaClass.simpleName}"
            stopSelf()
        }
    }

    // ------------------------------------------------------------------ engine process

    /** Starts rhcsd; restarts it with a growing pause if it crashes; ends when the user stopped it. */
    private suspend fun supervise() {
        Engine.state = "starting"
        Engine.error = null
        try {
            prepareWeb()
        } catch (e: Exception) {
            Engine.state = "error"
            Engine.error = "could not unpack the dashboard: ${e.message}"
            return
        }
        var pause = 1000L
        while (scope.isActive) {
            val t0 = System.currentTimeMillis()
            var code = -1
            try {
                code = runOnce()
            } catch (e: Exception) {
                Engine.error = "engine start failed: ${e.message}"
            }
            if (!scope.isActive) return
            if (userStop || code == 0) {
                markStopped()
                return
            }
            Engine.state = "error"
            if (Engine.error == null) Engine.error = "the engine stopped"
            if (System.currentTimeMillis() - t0 > 60_000) pause = 1000L
            delay(pause)
            if (userStop) {
                markStopped()
                return
            }
            pause = (pause * 2).coerceAtMost(30_000L)
            Engine.state = "starting"
        }
    }

    /** Runs the engine until it ends; returns its exit code (-1 when it could not start). */
    private suspend fun runOnce(): Int {
        val exe = File(applicationInfo.nativeLibraryDir, "librhcsd.so")
        if (!exe.isFile) {
            Engine.error = "the engine file is missing from this build (librhcsd.so)"
            delay(30_000)
            return -1
        }
        val data = File(filesDir, "data").apply { mkdirs() }
        val pb = ProcessBuilder(exe.absolutePath).directory(data).redirectErrorStream(true)
        pb.environment().apply {
            put("RHCSD_APP", "1")
            put("RHCSD_BIND", "127.0.0.1:${Engine.PORT}")
            put("RHCSD_WEB_DIR", File(filesDir, "web").absolutePath)
            put("RHCSD_DATA_DIR", data.absolutePath)
            put("RHCSD_TOKEN", Engine.token(this@EngineService))
            put("TOWER_LOOKUP", "external")
            put("HOME", filesDir.absolutePath)
        }
        val p = pb.start()
        proc = p
        Engine.error = null
        scope.launch {
            // "running" as soon as it answers
            repeat(60) {
                if (!scope.isActive || !p.isAlive) return@launch
                if (Engine.healthy(this@EngineService)) {
                    Engine.state = "running"
                    goForeground("Reading this phone's signal", running = true)
                    return@launch
                }
                delay(500)
            }
        }
        val log = Engine.logFile(this)
        runInterruptible {
            p.inputStream.bufferedReader().useLines { lines ->
                lines.forEach { line -> appendLog(log, line) }
            }
        }
        val code = p.waitFor()
        if (code != 0 && !userStop) Engine.error = "the engine stopped (exit code $code)"
        appendLog(log, "[app] engine exited with code $code")
        return code
    }

    /** Size-capped log: when it passes ~200 KB the old half is dropped. */
    private fun appendLog(f: File, line: String) {
        try {
            if (f.length() > 200_000) {
                val keep = f.readLines().takeLast(300)
                f.writeText(keep.joinToString("\n") + "\n")
            }
            f.appendText(line.take(400) + "\n")
        } catch (_: Exception) {
        }
    }

    // ------------------------------------------------------------------ bundled dashboard

    /** Copies assets/pulse-web into filesDir/web, only when the app version changed. */
    private fun prepareWeb() {
        val version = PackageInfoCompat.getLongVersionCode(packageManager.getPackageInfo(packageName, 0)).toString()
        val dest = File(filesDir, "web")
        val marker = File(dest, ".version")
        if (marker.isFile && marker.readText().trim() == version && File(dest, "index.html").isFile) return
        val tmp = File(filesDir, "web.tmp")
        tmp.deleteRecursively()
        copyAsset("pulse-web", tmp)
        File(tmp, ".version").writeText(version)
        dest.deleteRecursively()
        if (!tmp.renameTo(dest)) throw IllegalStateException("cannot move the dashboard into place")
    }

    private fun copyAsset(path: String, to: File) {
        val kids = assets.list(path) ?: emptyArray()
        if (kids.isEmpty()) {
            to.parentFile?.mkdirs()
            assets.open(path).use { i -> FileOutputStream(to).use { o -> i.copyTo(o) } }
        } else {
            to.mkdirs()
            kids.forEach { copyAsset("$path/$it", File(to, it)) }
        }
    }

    companion object {
        const val CHANNEL = "engine"
        const val NOTIF_ID = 7878
        const val ACTION_STOP = "app.rhcs.pulse.STOP"
        const val ACTION_START = "app.rhcs.pulse.START"
        const val ACTION_QUIT = "app.rhcs.pulse.QUIT"

        /** Starts (or restarts) the engine. */
        fun start(ctx: Context) {
            androidx.core.content.ContextCompat.startForegroundService(ctx, Intent(ctx, EngineService::class.java).setAction(ACTION_START))
        }

        /** Stops the engine; the service stays, showing "Stopped by you" and a Start button. */
        fun stop(ctx: Context) {
            ctx.startService(Intent(ctx, EngineService::class.java).setAction(ACTION_STOP))
        }
    }
}
