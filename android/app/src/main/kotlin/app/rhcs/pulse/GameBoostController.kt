package app.rhcs.pulse

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.net.InetSocketAddress
import java.net.Socket

/**
 * The work behind Game Boost, run by [EngineService] on its scope. Only runs while the user has
 * Game Boost switched on; polls nothing when it is off.
 *
 * While a game is in the foreground it measures TCP connect time to 1.1.1.1:443 separately over
 * Wi-Fi and over cellular (sockets bound to each Network), decides with hysteresis, and posts a
 * notification with a one-tap button to the system's internet panel. It cannot move the game's
 * traffic itself, force a cell tower or change the carrier's network mode.
 */
class GameBoostController(context: Context) {

    private val ctx: Context = context.applicationContext
    private var scope: CoroutineScope? = null
    private var job: Job? = null

    @Volatile private var wifiNet: Network? = null
    @Volatile private var cellNet: Network? = null
    private var wifiCb: ConnectivityManager.NetworkCallback? = null
    private var cellCb: ConnectivityManager.NetworkCallback? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var wake: PowerManager.WakeLock? = null

    private var lastEventTs = 0L
    private var fgPkg: String? = null
    private var streak = 0
    private var lastNotify = 0L
    private var notified = false

    /** Called once when the service starts: undo anything a crashed earlier run left behind. */
    fun start(scope: CoroutineScope) {
        this.scope = scope
        GameBoost.restoreDnd(ctx)
    }

    /** Starts or stops the loop to match the user's switch. Cheap; called on every service command. */
    fun sync() {
        val s = scope ?: return
        if (GameBoost.enabled(ctx)) {
            if (job?.isActive != true) job = s.launch { runLoop() }
        } else {
            job?.cancel()
            job = null
            cleanup()
        }
    }

    /** Service is ending: restore everything. */
    fun shutdown() {
        job?.cancel()
        job = null
        cleanup()
    }

    // ------------------------------------------------------------------ loop

    private suspend fun CoroutineScope.runLoop() {
        var current: String? = null
        var lastMeasure = 0L
        fgPkg = null
        lastEventTs = 0L
        try {
            while (isActive) {
                val pkg = if (GameBoost.hasUsageAccess(ctx)) pollForeground() else null
                val gamePkg = pkg?.takeIf { isGame(it) }
                if (gamePkg != current) {
                    if (current != null) endSession()
                    current = gamePkg
                    lastMeasure = 0L
                    if (gamePkg != null) {
                        beginSession(gamePkg)
                        delay(700) // let the network callbacks report what is available
                    }
                }
                val now = System.currentTimeMillis()
                if (current != null && now - lastMeasure >= 10_000L) {
                    lastMeasure = now
                    measureRound()
                }
                delay(3_000)
            }
        } finally {
            cleanup()
        }
    }

    private fun isGame(pkg: String): Boolean {
        if (pkg == ctx.packageName) return false
        return GameBoost.games(ctx).contains(pkg) || GameBoost.isGameApp(ctx, pkg)
    }

    /** The app now in the foreground, from usage events (event 1 = resumed, 2 = paused). */
    private fun pollForeground(): String? {
        val usm = ctx.getSystemService(Context.USAGE_STATS_SERVICE) as? UsageStatsManager ?: return fgPkg
        val now = System.currentTimeMillis()
        val from = if (lastEventTs == 0L) now - 30 * 60_000L else lastEventTs - 1_000L
        try {
            val events = usm.queryEvents(from, now) ?: return fgPkg
            val e = UsageEvents.Event()
            while (events.hasNextEvent()) {
                events.getNextEvent(e)
                when (e.eventType) {
                    1 -> fgPkg = e.packageName
                    2 -> if (e.packageName == fgPkg) fgPkg = null
                }
            }
            lastEventTs = now
        } catch (_: SecurityException) {
        } catch (_: Exception) {
        }
        return fgPkg
    }

    // ------------------------------------------------------------------ session

    private fun beginSession(pkg: String) {
        GameBoost.game = pkg
        GameBoost.gameLabel = GameBoost.labelOf(ctx, pkg)
        GameBoost.recKind = "none"
        GameBoost.recText = "Measuring…"
        streak = 0
        notified = false
        registerCallbacks()
        if (GameBoost.option(ctx, "dnd")) GameBoost.dndActive = GameBoost.applyDnd(ctx)
        if (GameBoost.option(ctx, "lowlat")) acquireLocks()
    }

    private fun endSession() {
        releaseLocks()
        GameBoost.restoreDnd(ctx)
        unregisterCallbacks()
        cancelNotification()
        GameBoost.resetLive()
        streak = 0
        notified = false
    }

    /** Idempotent: restores everything this feature changed. */
    @Synchronized
    private fun cleanup() {
        releaseLocks()
        GameBoost.restoreDnd(ctx)
        unregisterCallbacks()
        cancelNotification()
        GameBoost.resetLive()
    }

    // ------------------------------------------------------------------ locks (best effort)

    private fun acquireLocks() {
        try {
            val pm = ctx.getSystemService(Context.POWER_SERVICE) as PowerManager
            val w = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "pulse:gameboost")
            w.setReferenceCounted(false)
            w.acquire(2 * 60 * 60 * 1000L)
            wake = w
            GameBoost.wakeLockHeld = true
        } catch (_: Exception) {
        }
        try {
            val wm = ctx.getSystemService(Context.WIFI_SERVICE) as WifiManager
            if (wm.isWifiEnabled) {
                @Suppress("DEPRECATION")
                val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF
                val l = wm.createWifiLock(mode, "pulse:gameboost")
                l.setReferenceCounted(false)
                l.acquire()
                wifiLock = l
                GameBoost.wifiLockHeld = true
            }
        } catch (_: Exception) {
        }
    }

    private fun releaseLocks() {
        try {
            wake?.let { if (it.isHeld) it.release() }
        } catch (_: Exception) {
        }
        wake = null
        try {
            wifiLock?.let { if (it.isHeld) it.release() }
        } catch (_: Exception) {
        }
        wifiLock = null
        GameBoost.wakeLockHeld = false
        GameBoost.wifiLockHeld = false
    }

    // ------------------------------------------------------------------ networks

    private fun registerCallbacks() {
        if (wifiCb != null) return
        val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val wcb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                wifiNet = network
                GameBoost.wifiUp = true
            }

            override fun onLost(network: Network) {
                if (wifiNet == network) {
                    wifiNet = null
                    GameBoost.wifiUp = false
                    GameBoost.wifiMs = null
                }
            }
        }
        val ccb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                cellNet = network
                GameBoost.cellUp = true
            }

            override fun onLost(network: Network) {
                if (cellNet == network) {
                    cellNet = null
                    GameBoost.cellUp = false
                    GameBoost.cellMs = null
                }
            }
        }
        try {
            cm.registerNetworkCallback(request(NetworkCapabilities.TRANSPORT_WIFI), wcb)
            wifiCb = wcb
        } catch (_: Exception) {
        }
        try {
            cm.registerNetworkCallback(request(NetworkCapabilities.TRANSPORT_CELLULAR), ccb)
            cellCb = ccb
        } catch (_: Exception) {
        }
    }

    private fun request(transport: Int): NetworkRequest =
        NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addTransportType(transport)
            .build()

    private fun unregisterCallbacks() {
        val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        try {
            wifiCb?.let { cm.unregisterNetworkCallback(it) }
        } catch (_: Exception) {
        }
        try {
            cellCb?.let { cm.unregisterNetworkCallback(it) }
        } catch (_: Exception) {
        }
        wifiCb = null
        cellCb = null
        wifiNet = null
        cellNet = null
    }

    private fun activeTransport(): String? {
        return try {
            val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
            val caps = cm.activeNetwork?.let { cm.getNetworkCapabilities(it) }
            when {
                caps == null -> null
                caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> "wifi"
                caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "cellular"
                else -> null
            }
        } catch (e: Exception) {
            null
        }
    }

    /** Median TCP connect time (ms) of 3 tries to 1.1.1.1:443 through this network; null when fewer than 2 worked. */
    private fun tcpMedian(net: Network): Int? {
        val ms = ArrayList<Int>()
        repeat(3) {
            var s: Socket? = null
            try {
                s = net.socketFactory.createSocket()
                val t0 = System.nanoTime()
                s.connect(InetSocketAddress("1.1.1.1", 443), 2_500)
                ms.add(((System.nanoTime() - t0) / 1_000_000L).toInt())
            } catch (_: Exception) {
            } finally {
                try {
                    s?.close()
                } catch (_: Exception) {
                }
            }
            Thread.sleep(120)
        }
        if (ms.size < 2) return null
        ms.sort()
        return ms[ms.size / 2]
    }

    // ------------------------------------------------------------------ one measuring round

    private suspend fun measureRound() {
        val w = wifiNet
        val c = cellNet
        val (wm, cmm) = coroutineScope {
            val a = async { if (w != null) tcpMedian(w) else null }
            val b = async { if (c != null) tcpMedian(c) else null }
            a.await() to b.await()
        }
        GameBoost.wifiMs = wm
        GameBoost.cellMs = cmm
        GameBoost.measuredAt = System.currentTimeMillis()
        val active = activeTransport()
        GameBoost.active = active

        val activeName = if (active == "wifi") "Wi-Fi" else "Cellular"
        val otherName = if (active == "wifi") "Cellular" else "Wi-Fi"
        val activeMs = if (active == "wifi") wm else if (active == "cellular") cmm else null
        val otherMs = if (active == "wifi") cmm else if (active == "cellular") wm else null
        val otherUp = if (active == "wifi") c != null else w != null

        if (active == null) {
            streak = 0
            GameBoost.recKind = "none"
            GameBoost.recText = "No active Wi-Fi or cellular connection to compare."
        } else if (!otherUp) {
            streak = 0
            GameBoost.recKind = "stay"
            GameBoost.recText = "Only $activeName is connected, so there is nothing to compare. (Cellular can be absent while on Wi-Fi if the phone has switched mobile data off.)"
        } else if (otherMs == null) {
            streak = 0
            GameBoost.recKind = "stay"
            GameBoost.recText = "$otherName is connected but did not answer the test, so no switch is suggested."
        } else {
            val clearlyBetter = activeMs == null || (otherMs <= activeMs * 0.75 && activeMs - otherMs >= 15)
            streak = if (clearlyBetter) streak + 1 else 0
            val activeTxt = if (activeMs != null) "$activeMs ms" else "no answer"
            if (streak >= 3) {
                GameBoost.recKind = "switch"
                GameBoost.recText = "$otherName is faster right now: $otherMs ms against $activeTxt on $activeName."
                maybeNotify(otherName, otherMs, activeName, activeMs)
            } else if (streak > 0) {
                GameBoost.recKind = "stay"
                GameBoost.recText = "$otherName looks faster ($otherMs ms against $activeTxt). Checking again before suggesting a switch ($streak of 3)."
            } else {
                GameBoost.recKind = "stay"
                GameBoost.recText = "Stay on $activeName: $activeTxt against $otherMs ms on $otherName is not different enough to matter."
                if (notified) {
                    cancelNotification()
                    notified = false
                }
            }
        }
        updateHint(active)
    }

    /** If the signal is poor, point at a better spot, but only when the engine's own data says there is one. */
    private fun updateHint(active: String?) {
        val rsrp = Engine.lastRsrp
        if (active != "cellular" || rsrp == null || rsrp > -110) {
            GameBoost.hint = null
            return
        }
        val r = Engine.call(ctx, "GET", "/api/towers", null, 3000)
        var msg: String? = null
        if (r != null && r.first == 200) {
            try {
                val rec = JSONObject(r.second).optJSONObject("recommendation")
                if (rec != null && rec.optString("status") == "better") {
                    msg = rec.optString("message").takeIf { it.isNotBlank() }
                }
            } catch (_: Exception) {
            }
        }
        GameBoost.hint = if (msg != null) "Signal is weak ($rsrp dBm). $msg" else "Signal is weak ($rsrp dBm), but PULSE has no saved better spot or tower for here yet."
    }

    // ------------------------------------------------------------------ notification

    private fun maybeNotify(otherName: String, otherMs: Int, activeName: String, activeMs: Int?) {
        val now = System.currentTimeMillis()
        if (notified && now - lastNotify < 5 * 60_000L) return
        if (!NotificationManagerCompat.from(ctx).areNotificationsEnabled()) return
        val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "Game Boost suggestions", NotificationManager.IMPORTANCE_HIGH))
        }
        val intent = if (Build.VERSION.SDK_INT >= 29) Intent(Settings.Panel.ACTION_INTERNET_CONNECTIVITY) else Intent(Settings.ACTION_WIRELESS_SETTINGS)
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val pi = PendingIntent.getActivity(ctx, 21, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val game = GameBoost.gameLabel ?: "your game"
        val detail = "$otherName: $otherMs ms. $activeName: ${if (activeMs != null) "$activeMs ms" else "no answer"}. Measured to 1.1.1.1 over each network, three rounds in a row."
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(R.drawable.ic_launcher)
            .setContentTitle("$otherName is faster for $game right now")
            .setContentText(detail)
            .setStyle(NotificationCompat.BigTextStyle().bigText(detail + (GameBoost.hint?.let { "\n$it" } ?: "")))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .addAction(0, "Switch", pi)
            .build()
        try {
            nm.notify(NOTIF_ID, n)
            notified = true
            lastNotify = now
        } catch (_: SecurityException) {
        }
    }

    private fun cancelNotification() {
        try {
            (ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager).cancel(NOTIF_ID)
        } catch (_: Exception) {
        }
    }

    companion object {
        private const val CHANNEL = "gameboost"
        private const val NOTIF_ID = 7880
    }
}
