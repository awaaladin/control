package app.rhcs.pulse

import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import android.webkit.JavascriptInterface
import org.json.JSONObject

/**
 * The `PulseApp` object the PULSE web page can call (Game Boost on this phone). It only answers
 * when the page comes from the built-in engine on this phone; a page from another system on the
 * network gets "available: false" and cannot change anything.
 *
 * JavascriptInterface methods run on a WebView background thread, so anything that opens a
 * screen is posted to the main thread.
 */
class PulseBridge(
    private val ctx: Context,
    private val trusted: () -> Boolean,
    private val onMain: (() -> Unit) -> Unit,
) {
    private val denied = JSONObject().put("available", false).toString()

    /** The engine's access token for this phone, so the page never asks the person for it. Only for the built-in engine's own page. */
    @JavascriptInterface
    fun getToken(): String = if (trusted()) Engine.token(ctx) else ""

    @JavascriptInterface
    fun engineState(): String =
        if (!trusted()) denied
        else JSONObject().put("available", true).put("state", Engine.state).put("stopped_by_user", Engine.stoppedByUser).put("error", Engine.error ?: "").toString()

    @JavascriptInterface
    fun startEngine() {
        if (!trusted()) return
        Engine.setEnabled(ctx, true)
        Engine.stoppedByUser = false
        EngineService.start(ctx)
    }

    @JavascriptInterface
    fun stopEngine() {
        if (!trusted()) return
        EngineService.stop(ctx)
        Engine.setEnabled(ctx, false)
    }

    @JavascriptInterface
    fun getStatus(): String = if (trusted()) GameBoost.statusJson(ctx) else denied

    @JavascriptInterface
    fun setEnabled(on: Boolean) {
        if (trusted()) GameBoost.setEnabled(ctx, on)
    }

    @JavascriptInterface
    fun setOption(name: String, on: Boolean) {
        if (trusted()) GameBoost.setOption(ctx, name, on)
    }

    @JavascriptInterface
    fun setGame(pkg: String, on: Boolean) {
        if (trusted()) GameBoost.setGame(ctx, pkg, on)
    }

    @JavascriptInterface
    fun listApps(): String = if (trusted()) GameBoost.appsJson(ctx) else "[]"

    /**
     * Privileged mode. [action] is one of a fixed list (request_permission, repick, mobile_only,
     * wifi_back); anything else is refused. Never a command string. Returns {"ok":..,"message":..}.
     */
    @JavascriptInterface
    fun privAction(action: String, confirmed: Boolean): String {
        if (!trusted()) return JSONObject().put("ok", false).put("message", "not allowed").toString()
        return try {
            Privileged.perform(ctx, action, confirmed)
        } catch (e: Throwable) {
            JSONObject().put("ok", false).put("message", "failed: ${e.javaClass.simpleName}").toString()
        }
    }

    @JavascriptInterface
    fun openShizukuDownload() {
        open(Intent(Intent.ACTION_VIEW, android.net.Uri.parse("https://shizuku.rikka.app/download/")))
    }

    @JavascriptInterface
    fun openShizuku() {
        if (!trusted()) return
        val i = ctx.packageManager.getLaunchIntentForPackage("moe.shizuku.privileged.api")
        if (i != null) open(i)
    }

    @JavascriptInterface
    fun openWifiSettings() {
        open(Intent(Settings.ACTION_WIFI_SETTINGS))
    }

    @JavascriptInterface
    fun openUsageAccess() {
        open(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS))
    }

    @JavascriptInterface
    fun openDndAccess() {
        open(Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS))
    }

    @JavascriptInterface
    fun openInternetPanel() {
        open(if (Build.VERSION.SDK_INT >= 29) Intent(Settings.Panel.ACTION_INTERNET_CONNECTIVITY) else Intent(Settings.ACTION_WIRELESS_SETTINGS))
    }

    private fun open(i: Intent) {
        if (!trusted()) return
        onMain {
            try {
                ctx.startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            } catch (_: Exception) {
            }
        }
    }
}
