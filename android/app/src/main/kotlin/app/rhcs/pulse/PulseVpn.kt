package app.rhcs.pulse

import android.content.Context
import android.content.Intent
import android.net.Network
import android.net.VpnService
import android.os.ParcelFileDescriptor
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * Shared state and start/stop for the per-app game VPN. The VPN carries traffic of ONE app (the
 * game being boosted) and nothing else; a user-space forwarder ([VpnForwarder]) re-sends that
 * traffic through the chosen network (Wi-Fi or cellular). It never starts by itself: the user turns
 * it on in the PULSE panel and accepts Android's VPN dialog once.
 */
object PulseVpn {
    @Volatile var running = false
    @Volatile var pkg: String? = null
    @Volatile var netName: String? = null
    @Volatile var network: Network? = null
    @Volatile var lastError: String? = null
    @Volatile var killReason: String? = null
    @Volatile var service: PulseVpnService? = null

    /** The controller refreshes this while it watches the game; the forwarder stops itself if it goes quiet. */
    @Volatile var lastBeat = 0L

    val tx = AtomicLong()
    val rx = AtomicLong()
    val flows = AtomicInteger()

    /** True when the user has accepted Android's VPN dialog for this app. */
    fun consentGranted(ctx: Context): Boolean = try {
        VpnService.prepare(ctx) == null
    } catch (e: Exception) {
        false
    }

    /** Starts the VPN for [pkg] over [net]. Returns false when consent is missing or Android refuses. */
    @Synchronized
    fun start(ctx: Context, pkg: String, net: Network, name: String): Boolean {
        if (running) return true
        if (!consentGranted(ctx)) return false
        this.pkg = pkg
        network = net
        netName = name
        lastError = null
        killReason = null
        lastBeat = System.currentTimeMillis()
        tx.set(0)
        rx.set(0)
        flows.set(0)
        running = true
        return try {
            ctx.applicationContext.startService(Intent(ctx.applicationContext, PulseVpnService::class.java).setAction(PulseVpnService.ACTION_START))
            true
        } catch (e: Exception) {
            running = false
            lastError = "Android would not start the VPN: ${e.javaClass.simpleName}"
            false
        }
    }

    /** Stops the VPN at once (game traffic returns to the normal network). Safe to call any time. */
    @Synchronized
    fun stop(ctx: Context, reason: String?) {
        if (reason != null) killReason = reason
        val wasRunning = running
        running = false
        try {
            service?.teardown()
        } catch (_: Exception) {
        }
        if (wasRunning || service != null) {
            try {
                ctx.applicationContext.stopService(Intent(ctx.applicationContext, PulseVpnService::class.java))
            } catch (_: Exception) {
            }
        }
    }

    /** New connections go over [net] from now on; connections already open stay where they are. */
    fun switchNetwork(net: Network, name: String) {
        network = net
        netName = name
        try {
            service?.setUnderlying(net)
        } catch (_: Exception) {
        }
    }

    fun json(ctx: Context): JSONObject {
        val o = JSONObject()
        o.put("wanted", GameBoost.option(ctx, "vpn", false))
        o.put("consent", consentGranted(ctx))
        o.put("running", running)
        o.put("network", netName ?: JSONObject.NULL)
        o.put("flows", flows.get())
        o.put("tx_bytes", tx.get())
        o.put("rx_bytes", rx.get())
        o.put("note", GameBoost.vpnNote ?: JSONObject.NULL)
        o.put("stopped_because", killReason ?: JSONObject.NULL)
        o.put("error", lastError ?: JSONObject.NULL)
        return o
    }
}

class PulseVpnService : VpnService() {
    private var pfd: ParcelFileDescriptor? = null
    private var fwd: VpnForwarder? = null
    private var closed = false

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            teardown()
            stopSelf()
            return START_NOT_STICKY
        }
        val pkg = PulseVpn.pkg
        val net = PulseVpn.network
        if (fwd != null) return START_NOT_STICKY
        if (pkg == null || net == null || !PulseVpn.running) {
            teardown()
            stopSelf()
            return START_NOT_STICKY
        }
        closed = false
        try {
            val b = Builder()
                .setSession("PULSE game route")
                .addAddress(VpnForwarder.CLIENT_ADDR, 32)
                .addRoute("0.0.0.0", 0)
                .addDnsServer("1.1.1.1")
                .addDnsServer("8.8.8.8")
                .setMtu(VpnForwarder.MTU)
            // Only the game goes through the VPN. Every other app is untouched.
            b.addAllowedApplication(pkg)
            val p = b.establish() ?: throw IllegalStateException("Android refused to open the VPN (permission missing)")
            pfd = p
            setUnderlying(net)
            PulseVpn.service = this
            val f = VpnForwarder(this, p) { reason -> PulseVpn.stop(applicationContext, reason) }
            fwd = f
            f.start()
        } catch (e: Throwable) {
            PulseVpn.lastError = "VPN could not start: ${e.message ?: e.javaClass.simpleName}"
            PulseVpn.running = false
            teardown()
            stopSelf()
        }
        return START_NOT_STICKY
    }

    fun setUnderlying(net: Network) {
        try {
            setUnderlyingNetworks(arrayOf(net))
        } catch (_: Exception) {
        }
    }

    /** Idempotent: ends the forwarder and closes the tunnel. */
    @Synchronized
    fun teardown() {
        if (closed) return
        closed = true
        PulseVpn.running = false
        try {
            fwd?.close()
        } catch (_: Exception) {
        }
        fwd = null
        try {
            pfd?.close()
        } catch (_: Exception) {
        }
        pfd = null
        if (PulseVpn.service === this) PulseVpn.service = null
        PulseVpn.flows.set(0)
    }

    override fun onRevoke() {
        // The user (or another VPN app) took the VPN away.
        PulseVpn.killReason = "Android ended the VPN (another VPN started or you switched it off)."
        teardown()
        stopSelf()
        super.onRevoke()
    }

    override fun onDestroy() {
        teardown()
        super.onDestroy()
    }

    companion object {
        const val ACTION_START = "app.rhcs.pulse.VPN_START"
        const val ACTION_STOP = "app.rhcs.pulse.VPN_STOP"
    }
}
