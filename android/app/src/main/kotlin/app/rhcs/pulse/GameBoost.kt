package app.rhcs.pulse

import android.app.AppOpsManager
import android.app.NotificationManager
import android.content.Context
import android.content.pm.ApplicationInfo
import android.os.Build
import android.os.Process
import org.json.JSONArray
import org.json.JSONObject

/**
 * Phone Game Boost: settings, shared live state and the Do Not Disturb helpers. The work itself
 * (watching for a game, measuring, notifying) is in [GameBoostController], which runs inside
 * [EngineService].
 *
 * What it can and cannot do is stated honestly in the PULSE page: it cannot force a cell tower
 * or change the carrier's network mode, and it cannot make the game use a particular network.
 * It measures Wi-Fi and cellular separately, recommends, and offers a one-tap switch.
 */
object GameBoost {
    private const val PREFS = "gameboost"

    fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun enabled(ctx: Context): Boolean = prefs(ctx).getBoolean("enabled", false)

    /** Options "dnd" (silence notifications) and "lowlat" (keep Wi-Fi/CPU awake) default to on, but only act when allowed. */
    fun option(ctx: Context, name: String): Boolean = prefs(ctx).getBoolean("opt_$name", true)

    fun setOption(ctx: Context, name: String, on: Boolean) {
        if (name != "dnd" && name != "lowlat") return
        prefs(ctx).edit().putBoolean("opt_$name", on).apply()
    }

    fun games(ctx: Context): Set<String> = HashSet(prefs(ctx).getStringSet("games", emptySet()) ?: emptySet())

    fun setGame(ctx: Context, pkg: String, on: Boolean) {
        if (pkg.isBlank() || pkg.length > 200) return
        val s = games(ctx).toMutableSet()
        if (on) s.add(pkg) else s.remove(pkg)
        prefs(ctx).edit().putStringSet("games", s).apply()
    }

    // ------------------------------------------------------------------ access checks

    @Suppress("DEPRECATION")
    fun hasUsageAccess(ctx: Context): Boolean = try {
        val ops = ctx.getSystemService(Context.APP_OPS_SERVICE) as AppOpsManager
        val mode = if (Build.VERSION.SDK_INT >= 29) {
            ops.unsafeCheckOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), ctx.packageName)
        } else {
            ops.checkOpNoThrow(AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), ctx.packageName)
        }
        if (mode == AppOpsManager.MODE_DEFAULT) {
            ctx.checkCallingOrSelfPermission(android.Manifest.permission.PACKAGE_USAGE_STATS) == android.content.pm.PackageManager.PERMISSION_GRANTED
        } else {
            mode == AppOpsManager.MODE_ALLOWED
        }
    } catch (e: Exception) {
        false
    }

    fun hasDndAccess(ctx: Context): Boolean = try {
        (ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager).isNotificationPolicyAccessGranted
    } catch (e: Exception) {
        false
    }

    /** A game by the app's own declaration (category or the old game flag). Needs the app to be visible (see <queries>). */
    @Suppress("DEPRECATION")
    fun isGameApp(ctx: Context, pkg: String): Boolean = try {
        val info = ctx.packageManager.getApplicationInfo(pkg, 0)
        (Build.VERSION.SDK_INT >= 26 && info.category == ApplicationInfo.CATEGORY_GAME) ||
            (info.flags and ApplicationInfo.FLAG_IS_GAME) != 0
    } catch (e: Exception) {
        false
    }

    fun labelOf(ctx: Context, pkg: String): String = try {
        val pm = ctx.packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
    } catch (e: Exception) {
        pkg
    }

    // ------------------------------------------------------------------ Do Not Disturb

    /**
     * Switches to "priority only" if (and only if) access is granted and the phone is currently
     * not in any Do Not Disturb mode. The previous filter is saved first, so a crash still restores.
     */
    @Synchronized
    fun applyDnd(ctx: Context): Boolean {
        try {
            val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (!nm.isNotificationPolicyAccessGranted) return false
            val p = prefs(ctx)
            if (p.getBoolean("dnd_changed", false)) return true
            val cur = nm.currentInterruptionFilter
            if (cur != NotificationManager.INTERRUPTION_FILTER_ALL) return false // the user's own setting stays
            p.edit().putInt("dnd_prev", cur).putBoolean("dnd_changed", true).commit()
            try {
                nm.setInterruptionFilter(NotificationManager.INTERRUPTION_FILTER_PRIORITY)
            } catch (e: SecurityException) {
                p.edit().putBoolean("dnd_changed", false).commit()
                return false
            }
            return true
        } catch (e: Exception) {
            return false
        }
    }

    /** Puts the filter back if PULSE changed it. Safe to call any time, any number of times. */
    @Synchronized
    fun restoreDnd(ctx: Context) {
        try {
            val p = prefs(ctx)
            if (!p.getBoolean("dnd_changed", false)) return
            val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            try {
                // only undo our own change: if the user switched it to something else meanwhile, leave it
                if (nm.isNotificationPolicyAccessGranted &&
                    nm.currentInterruptionFilter == NotificationManager.INTERRUPTION_FILTER_PRIORITY
                ) {
                    nm.setInterruptionFilter(p.getInt("dnd_prev", NotificationManager.INTERRUPTION_FILTER_ALL))
                }
            } catch (e: SecurityException) {
            }
            p.edit().putBoolean("dnd_changed", false).commit()
        } catch (e: Exception) {
        }
    }

    // ------------------------------------------------------------------ live state

    @Volatile var game: String? = null
    @Volatile var gameLabel: String? = null
    @Volatile var wifiUp = false
    @Volatile var cellUp = false
    @Volatile var wifiMs: Int? = null
    @Volatile var cellMs: Int? = null
    @Volatile var active: String? = null
    @Volatile var recKind: String = "none"
    @Volatile var recText: String? = null
    @Volatile var hint: String? = null
    @Volatile var dndActive = false
    @Volatile var wifiLockHeld = false
    @Volatile var wakeLockHeld = false
    @Volatile var measuredAt = 0L

    fun resetLive() {
        game = null
        gameLabel = null
        wifiUp = false
        cellUp = false
        wifiMs = null
        cellMs = null
        active = null
        recKind = "none"
        recText = null
        hint = null
        dndActive = false
        wifiLockHeld = false
        wakeLockHeld = false
        measuredAt = 0L
    }

    /** Turns Game Boost on or off. Off restores everything it changed straight away. */
    fun setEnabled(ctx: Context, on: Boolean) {
        prefs(ctx).edit().putBoolean("enabled", on).commit()
        if (on) {
            EngineService.boostSync(ctx, true)
        } else {
            resetLive()
            restoreDnd(ctx)
            EngineService.boostSync(ctx, false)
        }
    }

    /** Everything the PULSE page shows, as JSON. */
    fun statusJson(ctx: Context): String {
        val on = enabled(ctx)
        val usage = hasUsageAccess(ctx)
        val g = game
        val state = when {
            !on -> "off"
            !usage -> "needs_usage"
            g == null -> "waiting"
            else -> "boosting"
        }
        val o = JSONObject()
        o.put("available", true)
        o.put("sdk", Build.VERSION.SDK_INT)
        o.put("enabled", on)
        o.put("state", state)
        o.put("usage_access", usage)
        o.put("dnd_access", hasDndAccess(ctx))
        o.put("opt_dnd", option(ctx, "dnd"))
        o.put("opt_lowlat", option(ctx, "lowlat"))
        o.put("dnd_active", dndActive)
        o.put("wifi_lock", wifiLockHeld)
        o.put("wake_lock", wakeLockHeld)
        o.put("low_latency_api", Build.VERSION.SDK_INT >= 29)
        o.put("service_alive", EngineService.alive)
        if (g != null) o.put("game", JSONObject().put("pkg", g).put("label", gameLabel ?: g))
        val nets = JSONObject()
        nets.put("wifi", net(wifiUp, wifiMs, active == "wifi"))
        nets.put("cellular", net(cellUp, cellMs, active == "cellular"))
        o.put("networks", nets)
        if (measuredAt > 0) o.put("measured_age_s", (System.currentTimeMillis() - measuredAt) / 1000)
        o.put("recommendation", JSONObject().put("kind", recKind).put("text", recText ?: JSONObject.NULL))
        o.put("hint", hint ?: JSONObject.NULL)
        val rsrp = Engine.lastRsrp
        if (rsrp != null) o.put("rsrp", rsrp)
        val arr = JSONArray()
        games(ctx).forEach { arr.put(it) }
        o.put("games", arr)
        return o.toString()
    }

    private fun net(up: Boolean, ms: Int?, inUse: Boolean): JSONObject {
        val n = JSONObject().put("available", up).put("in_use", inUse)
        if (ms != null) n.put("ms", ms)
        return n
    }

    /** Launchable apps (the ones this app can see), games first, for the "choose games" list. */
    fun appsJson(ctx: Context): String {
        val pm = ctx.packageManager
        val intent = android.content.Intent(android.content.Intent.ACTION_MAIN).addCategory(android.content.Intent.CATEGORY_LAUNCHER)
        val chosen = games(ctx)
        val seen = HashSet<String>()
        val rows = ArrayList<JSONObject>()
        try {
            for (ri in pm.queryIntentActivities(intent, 0)) {
                val pkg = ri.activityInfo?.packageName ?: continue
                if (pkg == ctx.packageName || !seen.add(pkg)) continue
                val label = try {
                    ri.loadLabel(pm).toString()
                } catch (e: Exception) {
                    pkg
                }
                rows.add(JSONObject().put("pkg", pkg).put("label", label).put("game", isGameApp(ctx, pkg)).put("chosen", chosen.contains(pkg)))
            }
        } catch (e: Exception) {
        }
        rows.sortWith(compareByDescending<JSONObject> { it.getBoolean("game") || it.getBoolean("chosen") }.thenBy { it.getString("label").lowercase() })
        val a = JSONArray()
        rows.take(400).forEach { a.put(it) }
        return a.toString()
    }
}
