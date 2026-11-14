package app.rhcs.pulse

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

/**
 * Finds out where cell towers stand. The engine lists the cells that need a position
 * (`GET /api/towers/lookups`); this asks beaconDB (and OpenCellID, if the user typed a key) and
 * posts each answer back (`POST /api/towers/location`). Only the cell's network codes and IDs are
 * sent, never this phone's position. The engine keeps the results; nothing is cached here.
 */
class LookupWorker(private val ctx: Context) {

    private val userAgent = "rhcs/0.1 (+https://github.com/awaaladin/control)"

    private sealed class Answer {
        class Found(val lat: Double, val lon: Double, val accuracy: Double?, val source: String) : Answer()
        object NotFound : Answer()
        class Failed(val why: String, val report: Boolean) : Answer()
    }

    fun start(scope: CoroutineScope) {
        scope.launch {
            delay(5_000)
            while (isActive) {
                var pause = 15_000L
                if (Engine.state == "stopped") {
                    delay(pause)
                    continue
                }
                try {
                    val r = Engine.call(ctx, "GET", "/api/towers/lookups", null)
                    val list = if (r != null && r.first == 200) JSONObject(r.second).optJSONArray("lookups") else null
                    if (list != null) {
                        for (i in 0 until list.length()) {
                            if (!isActive) break
                            if (!handle(list.getJSONObject(i))) {
                                pause = 120_000L // the network looks down: do not hammer
                                break
                            }
                            delay(1_500) // polite: at most one cell every 1.5 s
                        }
                    }
                } catch (_: Exception) {
                }
                delay(pause)
            }
        }
    }

    /** False when the databases could not be reached at all. */
    private fun handle(c: JSONObject): Boolean {
        val mcc = c.optString("mcc").toIntOrNull()
        val mnc = c.optString("mnc").toIntOrNull()
        val tac = c.optInt("tac", -1)
        val cell = c.optLong("cell_id", -1)
        if (mcc == null || mnc == null || tac < 0 || cell < 0) return true
        var a = beacon(mcc, mnc, tac, cell)
        val key = ctx.getSharedPreferences("pulse", Context.MODE_PRIVATE).getString("ocid", null)?.trim().orEmpty()
        if (a is Answer.NotFound && key.isNotEmpty()) {
            Thread.sleep(1_000)
            a = opencellid(key, mcc, mnc, tac, cell)
        }
        val base = JSONObject().put("mcc", mcc).put("mnc", mnc).put("tac", tac).put("cell_id", cell)
        when (a) {
            is Answer.Found -> {
                base.put("lat", a.lat).put("lon", a.lon).put("source", a.source)
                a.accuracy?.let { base.put("accuracy_m", it) }
            }
            Answer.NotFound -> base.put("found", false)
            is Answer.Failed -> {
                if (!a.report) return false
                base.put("error", a.why)
            }
        }
        Engine.call(ctx, "POST", "/api/towers/location", base.toString())
        return a !is Answer.Failed || a.report
    }

    private fun beacon(mcc: Int, mnc: Int, tac: Int, cell: Long): Answer {
        val body = JSONObject()
            .put("considerIp", false)
            .put(
                "cellTowers",
                JSONArray().put(
                    JSONObject().put("radioType", "lte").put("mobileCountryCode", mcc).put("mobileNetworkCode", mnc)
                        .put("locationAreaCode", tac).put("cellId", cell),
                ),
            )
        var c: HttpURLConnection? = null
        return try {
            c = (URL("https://api.beacondb.net/v1/geolocate").openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                connectTimeout = 10_000
                readTimeout = 15_000
                doOutput = true
                setRequestProperty("Content-Type", "application/json")
                setRequestProperty("User-Agent", userAgent)
                outputStream.use { it.write(body.toString().toByteArray()) }
            }
            val code = c.responseCode
            val text = (if (code in 200..299) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText().take(65_536) } ?: ""
            when {
                code == 404 -> Answer.NotFound
                code == 200 -> {
                    val j = JSONObject(text)
                    // A "fallback" answer is a guess from an area or IP, not this cell's position.
                    if (!j.isNull("fallback") && j.has("fallback")) return Answer.NotFound
                    val loc = j.optJSONObject("location") ?: return Answer.Failed("beaconDB answered without a position", true)
                    Answer.Found(loc.getDouble("lat"), loc.getDouble("lng"), if (j.has("accuracy")) j.optDouble("accuracy") else null, "beacondb")
                }
                code == 429 || code >= 500 -> Answer.Failed("beaconDB: HTTP $code", true)
                else -> Answer.Failed("beaconDB: HTTP $code", true)
            }
        } catch (e: java.io.IOException) {
            Answer.Failed("beaconDB could not be reached", false)
        } catch (e: org.json.JSONException) {
            Answer.Failed("unexpected answer from beaconDB", true)
        } finally {
            c?.disconnect()
        }
    }

    private fun opencellid(key: String, mcc: Int, mnc: Int, tac: Int, cell: Long): Answer {
        val url = "https://opencellid.org/cell/get?key=${URLEncoder.encode(key, "UTF-8")}&mcc=$mcc&mnc=$mnc&lac=$tac&cellid=$cell&radio=LTE&format=json"
        var c: HttpURLConnection? = null
        return try {
            c = (URL(url).openConnection() as HttpURLConnection).apply {
                connectTimeout = 10_000
                readTimeout = 15_000
                setRequestProperty("User-Agent", userAgent)
            }
            val code = c.responseCode
            val text = (if (code in 200..299) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText().take(65_536) } ?: ""
            val j = JSONObject(text)
            if (j.has("lat") && j.has("lon")) {
                val lat = j.getDouble("lat")
                val lon = j.getDouble("lon")
                if (lat == 0.0 && lon == 0.0) Answer.NotFound
                else Answer.Found(lat, lon, if (j.has("range")) j.optDouble("range") else null, "opencellid")
            } else if (j.optInt("code") == 1 || j.optString("error").contains("not found", ignoreCase = true)) {
                Answer.NotFound
            } else {
                // the key is never put in the message
                Answer.Failed("OpenCellID: ${j.optString("error", "no position in answer").take(80)}", true)
            }
        } catch (e: java.io.IOException) {
            Answer.Failed("OpenCellID could not be reached", false)
        } catch (e: org.json.JSONException) {
            Answer.Failed("unexpected answer from OpenCellID", true)
        } finally {
            c?.disconnect()
        }
    }
}
