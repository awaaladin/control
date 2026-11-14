package app.rhcs.pulse

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.ConnectException
import java.net.HttpURLConnection
import java.net.Inet4Address
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.Socket
import java.net.URL
import java.util.Collections

/**
 * Looks for the home system (rhcsd, port 7878) on every network this phone is on:
 * the Wi-Fi it joined, its own hotspot, USB tethering, and the phone itself (the Termux hub).
 *
 * Android does not let apps read the neighbour (ARP) table, so presence is detected by a quick
 * connection attempt: a device that actively refuses the connection is awake, one that answers is
 * checked for rhcsd, and one that stays silent (e.g. a Windows laptop with the port blocked) is not listed.
 */
object Finder {
    const val PORT = 7878

    data class Health(val version: String, val router: String?, val needsToken: Boolean)

    private data class Net(val name: String, val ip: String) {
        val prefix: String get() = ip.substringBeforeLast('.')
    }

    private sealed class Probe {
        class Hub(val health: Health) : Probe()
        object Alive : Probe()
        object None : Probe()
    }

    /** Asks `<base>/api/health`. Null when nothing that looks like rhcsd answers. */
    fun health(base: String, timeoutMs: Int): Health? {
        var c: HttpURLConnection? = null
        return try {
            c = (URL("$base/api/health").openConnection() as HttpURLConnection).apply {
                connectTimeout = timeoutMs
                readTimeout = timeoutMs
                instanceFollowRedirects = false
                setRequestProperty("Accept", "application/json")
            }
            val code = c.responseCode
            val stream = if (code in 200..299) c.inputStream else c.errorStream
            val body = stream?.bufferedReader()?.use { it.readText().take(8192) } ?: ""
            when {
                code == 401 || code == 403 -> if (body.contains("token", ignoreCase = true)) Health("?", null, true) else null
                code == 200 -> parseHealth(body)
                else -> null
            }
        } catch (e: Exception) {
            null
        } finally {
            c?.disconnect()
        }
    }

    private fun parseHealth(body: String): Health? {
        val j = JSONObject(body)
        val full = j.has("capabilities")
        // Through a tunnel an unauthenticated caller only gets {ok, version, auth_required:true}.
        val isHub = j.optBoolean("ok") && (full || j.has("auth_required"))
        if (!isHub) return null
        val router = j.optJSONObject("sources")?.optJSONObject("router")
        val line = router?.let { r ->
            val model = r.optString("model").ifEmpty { "router" }
            model + if (r.optBoolean("online")) " online" else " offline"
        }
        return Health(j.optString("version", "?"), line, needsToken = !full && j.optBoolean("auth_required"))
    }

    private fun localNetworks(): List<Net> {
        val out = mutableListOf<Net>()
        try {
            val ifs = NetworkInterface.getNetworkInterfaces() ?: return out
            for (nif in Collections.list(ifs)) {
                val n = nif.name.lowercase()
                if (!nif.isUp || nif.isLoopback) continue
                // Mobile data and VPN tunnels: scanning the carrier's network is pointless.
                if (n.startsWith("rmnet") || n.startsWith("ccmni") || n.startsWith("ppp") || n.startsWith("tun") ||
                    n.startsWith("dummy") || n.startsWith("v4-")
                ) continue
                for (a in Collections.list(nif.inetAddresses)) {
                    if (a is Inet4Address && !a.isLoopbackAddress && !a.isLinkLocalAddress) {
                        val ip = a.hostAddress ?: continue
                        out += Net(nif.name, ip)
                    }
                }
            }
        } catch (e: Exception) {
            // Some Android versions refuse to list interfaces; report "nothing found".
        }
        return out.distinctBy { it.ip }
    }

    private fun probe(ip: String): Probe {
        val s = Socket()
        return try {
            s.connect(InetSocketAddress(ip, PORT), 600)
            s.close()
            val h = health("http://$ip:$PORT", 1500)
            if (h != null) Probe.Hub(h) else Probe.Alive
        } catch (e: ConnectException) {
            if (e.message?.contains("ECONNREFUSED") == true) Probe.Alive else Probe.None
        } catch (e: Exception) {
            Probe.None
        } finally {
            try {
                s.close()
            } catch (_: Exception) {
            }
        }
    }

    /**
     * Scans. All callbacks run on the main thread.
     * [onHub]: ip, health, isThisPhone. [onAlive]: a device that is awake but is not rhcsd.
     */
    suspend fun scan(
        onStatus: (String) -> Unit,
        onHub: (String, Health, Boolean) -> Unit,
        onAlive: (String) -> Unit,
    ) {
        val nets = withContext(Dispatchers.IO) { localNetworks() }

        // The phone's own hub (rhcsd running in Termux). With the built-in engine this phone
        // already has its own first row, so a loopback hit would only be a duplicate of it.
        if (!Engine.supported()) {
            withContext(Dispatchers.IO) { health("http://127.0.0.1:$PORT", 800) }?.let { onHub("127.0.0.1", it, true) }
        }

        if (nets.isEmpty()) {
            onStatus("No Wi-Fi, hotspot or USB network found. Turn one on, then Scan again.")
            return
        }
        val own = nets.map { it.ip }.toSet()
        val targets = nets.flatMap { n -> (1..254).map { "${n.prefix}.$it" } }.filter { it !in own }.distinct()
        onStatus("Looking at ${targets.size} addresses on ${nets.joinToString { it.name + " " + it.prefix + ".x" }} …")

        val gate = Semaphore(96)
        coroutineScope {
            targets.map { ip ->
                launch(Dispatchers.IO) {
                    gate.withPermit {
                        when (val r = probe(ip)) {
                            is Probe.Hub -> withContext(Dispatchers.Main) { onHub(ip, r.health, false) }
                            Probe.Alive -> withContext(Dispatchers.Main) { onAlive(ip) }
                            Probe.None -> Unit
                        }
                    }
                }
            }.joinAll()
        }
    }
}
