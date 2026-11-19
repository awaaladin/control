package app.rhcs.pulse

import android.content.Context
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.provider.Settings
import android.telephony.TelephonyManager
import org.json.JSONObject
import rikka.shizuku.Shizuku

/**
 * Privileged mode through the Shizuku app (optional, the user installs and starts it, then allows
 * PULSE). Only commands from a fixed allow-list can run; the web page can only name an action,
 * never send a command string. Every change is remembered in preferences and undone later
 * ([restoreAll]): when the game ends, Game Boost is switched off, the service stops, or the app
 * starts again.
 */
object Privileged {
    private const val SHIZUKU_PKG = "moe.shizuku.privileged.api"
    private const val REQ_CODE = 4711
    private const val GAME_QUIET_MS = 120_000L
    private const val AUTO_GAP_MS = 10 * 60_000L
    private const val AUTO_POOR_MS = 60_000L

    /** The only commands that can ever run. */
    private val COMMANDS: Map<String, Array<String>> = mapOf(
        "wifi_on" to arrayOf("svc", "wifi", "enable"),
        "wifi_off" to arrayOf("svc", "wifi", "disable"),
        "data_on" to arrayOf("svc", "data", "enable"),
        "data_off" to arrayOf("svc", "data", "disable"),
        "airplane_on" to arrayOf("cmd", "connectivity", "airplane-mode", "enable"),
        "airplane_off" to arrayOf("cmd", "connectivity", "airplane-mode", "disable"),
    )

    @Volatile var busy = false
    @Volatile var lastResult: String? = null
    private var poorSince = 0L

    private fun prefs(ctx: Context) = GameBoost.prefs(ctx)

    fun autoEnabled(ctx: Context): Boolean = prefs(ctx).getBoolean("priv_auto", false)

    fun setAuto(ctx: Context, on: Boolean) {
        prefs(ctx).edit().putBoolean("priv_auto", on).apply()
        poorSince = 0L
    }

    // ------------------------------------------------------------------ Shizuku state

    private fun installed(ctx: Context): Boolean = try {
        ctx.packageManager.getPackageInfo(SHIZUKU_PKG, 0)
        true
    } catch (e: Exception) {
        false
    }

    /** "not_installed" | "not_running" | "needs_permission" | "ready" */
    fun state(ctx: Context): String {
        val running = try {
            Shizuku.pingBinder()
        } catch (e: Throwable) {
            false
        }
        if (!running) return if (installed(ctx)) "not_running" else "not_installed"
        return try {
            if (Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED) "ready" else "needs_permission"
        } catch (e: Throwable) {
            "not_running"
        }
    }

    fun requestPermission() {
        try {
            if (Shizuku.pingBinder() && !Shizuku.shouldShowRequestPermissionRationale()) Shizuku.requestPermission(REQ_CODE)
        } catch (_: Throwable) {
        }
    }

    private fun deniedForever(): Boolean = try {
        Shizuku.pingBinder() && Shizuku.shouldShowRequestPermissionRationale()
    } catch (e: Throwable) {
        false
    }

    // ------------------------------------------------------------------ running a command

    /** Runs one allow-listed command through Shizuku; blocks up to ~10 s. Returns success and a short message. */
    private fun run(key: String): Pair<Boolean, String> {
        val cmd = COMMANDS[key] ?: return false to "command not allowed"
        return try {
            // newProcess is private in current Shizuku API versions, but still the simplest way to run one shell command.
            val m = Shizuku::class.java.getDeclaredMethod("newProcess", Array<String>::class.java, Array<String>::class.java, String::class.java)
            m.isAccessible = true
            val p = m.invoke(null, cmd, null, null) as Process
            var code: Int? = null
            val end = System.currentTimeMillis() + 10_000L
            while (code == null && System.currentTimeMillis() < end) {
                try {
                    code = p.exitValue()
                } catch (e: IllegalThreadStateException) {
                    Thread.sleep(100)
                }
            }
            val text = try {
                val a = p.inputStream.readBytes()
                val b = p.errorStream.readBytes()
                String(a + b).trim().take(200)
            } catch (e: Exception) {
                ""
            }
            try {
                p.destroy()
            } catch (_: Exception) {
            }
            when {
                code == null -> false to "timed out"
                code == 0 -> true to text
                else -> false to "exit $code ${text}".trim()
            }
        } catch (e: Throwable) {
            val c = e.cause ?: e
            false to "${c.javaClass.simpleName}: ${c.message ?: ""}".trim()
        }
    }

    // ------------------------------------------------------------------ safety checks

    private fun airplaneOn(ctx: Context): Boolean = try {
        Settings.Global.getInt(ctx.contentResolver, Settings.Global.AIRPLANE_MODE_ON, 0) == 1
    } catch (e: Exception) {
        false
    }

    /** null = unknown */
    private fun callIdle(ctx: Context): Boolean? = try {
        (ctx.getSystemService(Context.TELEPHONY_SERVICE) as TelephonyManager).callState == TelephonyManager.CALL_STATE_IDLE
    } catch (e: Throwable) {
        null
    }

    @Suppress("DEPRECATION")
    private fun cellularPresent(ctx: Context): Boolean = try {
        val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        cm.allNetworks.any { n ->
            val c = cm.getNetworkCapabilities(n)
            c != null && c.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) && c.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        }
    } catch (e: Exception) {
        false
    }

    /** Why a tower re-pick must not run right now, or null when it may. */
    fun repickBlock(ctx: Context, auto: Boolean): String? {
        if (state(ctx) != "ready") return "Privileged mode is not ready."
        if (busy) return "Another privileged action is still running."
        if (GameBoost.game != null || System.currentTimeMillis() - GameBoost.lastGameSeen < GAME_QUIET_MS) {
            return "A game is open or was open a moment ago. Re-picking drops ALL connections for 5 to 10 seconds, so it is blocked while gaming."
        }
        if (airplaneOn(ctx)) return "Airplane mode is already on."
        val idle = callIdle(ctx)
        if (idle == false) return "A phone call is in progress."
        if (auto && idle == null) return "PULSE cannot tell whether a call is active."
        if (PulseVpn.running) return "The game VPN is running."
        return null
    }

    // ------------------------------------------------------------------ actions (named, never raw commands)

    /** Result for the page: {"ok":bool,"message":"..."} . Long work continues on a background thread. */
    fun perform(ctx: Context, action: String, confirmed: Boolean): String {
        val app = ctx.applicationContext
        fun res(ok: Boolean, msg: String) = JSONObject().put("ok", ok).put("message", msg).toString()
        when (action) {
            "request_permission" -> {
                requestPermission()
                return res(true, if (deniedForever()) "Open the Shizuku app, Authorized apps, and allow PULSE there." else "Asking Shizuku. Accept the prompt.")
            }
            "repick" -> {
                if (!confirmed) return res(false, "Needs confirmation.")
                val why = repickBlock(app, false)
                if (why != null) return res(false, why)
                startRepick(app)
                return res(true, "Re-picking: connections drop for about 5 to 10 seconds.")
            }
            "mobile_only" -> {
                if (!confirmed) return res(false, "Needs confirmation.")
                if (state(app) != "ready") return res(false, "Privileged mode is not ready.")
                if (busy) return res(false, "Another privileged action is still running.")
                if (!cellularPresent(app)) return res(false, "No mobile data connection found. Turn mobile data on first, otherwise you would have no internet.")
                bg("mobile_only") {
                    prefs(app).edit().putBoolean("wifi_off_by_us", true).commit()
                    val r = run("wifi_off")
                    if (!r.first) prefs(app).edit().putBoolean("wifi_off_by_us", false).commit()
                    lastResult = if (r.first) "Wi-Fi is off; the phone uses mobile data. It comes back when your game closes, Game Boost is turned off, or you tap Use Wi-Fi again." else "Could not turn Wi-Fi off: ${r.second}"
                }
                return res(true, "Turning Wi-Fi off.")
            }
            "wifi_back" -> {
                if (state(app) != "ready") return res(false, "Privileged mode is not ready. Turn Wi-Fi on in the phone's own settings instead.")
                if (busy) return res(false, "Another privileged action is still running.")
                bg("wifi_back") {
                    val r = run("wifi_on")
                    if (r.first) prefs(app).edit().putBoolean("wifi_off_by_us", false).commit()
                    lastResult = if (r.first) "Wi-Fi is back on." else "Could not turn Wi-Fi on: ${r.second}"
                }
                return res(true, "Turning Wi-Fi on.")
            }
        }
        return res(false, "Unknown action.")
    }

    private fun bg(name: String, work: () -> Unit) {
        busy = true
        Thread({
            try {
                work()
            } catch (e: Throwable) {
                lastResult = "$name failed: ${e.javaClass.simpleName}"
            } finally {
                busy = false
            }
        }, "pulse-priv-$name").apply { isDaemon = true }.start()
    }

    private fun startRepick(app: Context) {
        bg("repick") {
            prefs(app).edit().putBoolean("airplane_changed", true).commit()
            val on = run("airplane_on")
            if (!on.first) {
                prefs(app).edit().putBoolean("airplane_changed", false).commit()
                lastResult = "Could not switch airplane mode on: ${on.second}"
                return@bg
            }
            Thread.sleep(4_000)
            var off = run("airplane_off")
            if (!off.first) off = run("airplane_off")
            if (off.first) prefs(app).edit().putBoolean("airplane_changed", false).commit()
            lastResult = if (off.first) "Re-pick done at ${java.text.DateFormat.getTimeInstance().format(java.util.Date())}. The phone chose a tower again; it may be the same one." else "Airplane mode may still be ON. Turn it off in the quick settings. (${off.second})"
        }
    }

    /**
     * Puts back whatever PULSE changed (Wi-Fi turned off, airplane mode left on). Blocking; call on a
     * background thread. If Shizuku is not available the flags stay and it is tried again next time.
     */
    @Synchronized
    fun restoreAll(ctx: Context) {
        if (busy) return // a re-pick is in progress and finishes by itself
        val p = prefs(ctx)
        val wifi = p.getBoolean("wifi_off_by_us", false)
        val air = p.getBoolean("airplane_changed", false)
        if (!wifi && !air) return
        if (state(ctx) != "ready") return
        if (air) {
            if (!airplaneOn(ctx) || run("airplane_off").first) p.edit().putBoolean("airplane_changed", false).commit()
        }
        if (wifi) {
            if (run("wifi_on").first) p.edit().putBoolean("wifi_off_by_us", false).commit()
        }
    }

    fun restoreAllAsync(ctx: Context) {
        val app = ctx.applicationContext
        val p = prefs(app)
        if (!p.getBoolean("wifi_off_by_us", false) && !p.getBoolean("airplane_changed", false)) return
        Thread({
            try {
                restoreAll(app)
            } catch (_: Throwable) {
            }
        }, "pulse-priv-restore").apply { isDaemon = true }.start()
    }

    // ------------------------------------------------------------------ optional auto mode

    /** Called by the Game Boost loop every few seconds while no game is open. */
    fun autoTick(ctx: Context, cellularActive: Boolean) {
        if (!autoEnabled(ctx)) {
            poorSince = 0L
            return
        }
        val now = System.currentTimeMillis()
        val rsrp = Engine.lastRsrp
        if (rsrp == null || now - Engine.lastRsrpAt > 20_000L || rsrp > -110 || !cellularActive) {
            poorSince = 0L
            return
        }
        if (poorSince == 0L) poorSince = now
        if (now - poorSince < AUTO_POOR_MS) return
        val last = prefs(ctx).getLong("priv_auto_last", 0L)
        if (now - last < AUTO_GAP_MS) return
        if (repickBlock(ctx, true) != null) return
        prefs(ctx).edit().putLong("priv_auto_last", now).commit()
        poorSince = 0L
        startRepick(ctx.applicationContext)
    }

    fun json(ctx: Context): JSONObject {
        val st = state(ctx)
        val o = JSONObject()
        o.put("state", st)
        o.put("denied_forever", st == "needs_permission" && deniedForever())
        o.put("auto", autoEnabled(ctx))
        o.put("busy", busy)
        o.put("last", lastResult ?: JSONObject.NULL)
        o.put("wifi_off_by_us", prefs(ctx).getBoolean("wifi_off_by_us", false))
        o.put("airplane_pending", prefs(ctx).getBoolean("airplane_changed", false))
        val why = if (st == "ready") repickBlock(ctx, false) else null
        o.put("repick_blocked", why ?: JSONObject.NULL)
        return o
    }
}
