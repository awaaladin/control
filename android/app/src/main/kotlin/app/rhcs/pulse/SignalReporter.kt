package app.rhcs.pulse

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.BatteryManager
import android.os.Build
import android.os.Bundle
import android.os.Looper
import android.telephony.CellIdentityLte
import android.telephony.CellIdentityNr
import android.telephony.CellInfo
import android.telephony.CellInfoGsm
import android.telephony.CellInfoLte
import android.telephony.CellInfoNr
import android.telephony.CellInfoWcdma
import android.telephony.CellSignalStrengthLte
import android.telephony.CellSignalStrengthNr
import android.telephony.SubscriptionManager
import android.telephony.TelephonyManager
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executor
import kotlin.coroutines.resume

/**
 * Reads this phone's own radio (data SIM) and position and posts them to the engine on
 * 127.0.0.1: `/api/phone/push` every ~2 s, `/api/location/push` on every fix and every 30 s.
 * Android reports "unavailable" as Integer.MAX_VALUE; those become absent fields.
 */
class SignalReporter(private val ctx: Context) {

    private val unavailable = Int.MAX_VALUE
    private var lm: LocationManager? = null
    private var scope: CoroutineScope? = null

    @Volatile private var lastFix: Location? = null
    private var lastFixPushed = 0L

    private val listener = object : LocationListener {
        override fun onLocationChanged(l: Location) {
            val best = lastFix
            // a coarse network fix right after a good GPS fix would only make the position worse
            if (best != null && best.provider == LocationManager.GPS_PROVIDER && l.provider != best.provider &&
                l.time - best.time < 10_000 && best.hasAccuracy() && best.accuracy <= 30f
            ) return
            lastFix = l
            scope?.launch { pushFix(l) }
        }

        @Deprecated("Deprecated in Java")
        override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
        override fun onProviderEnabled(provider: String) {}
        override fun onProviderDisabled(provider: String) {}
    }

    fun start(scope: CoroutineScope) {
        this.scope = scope
        startLocation()
        scope.launch { phoneLoop() }
        scope.launch {
            while (isActive) {
                delay(30_000)
                lastFix?.let { if (System.currentTimeMillis() - it.time < 23 * 3_600_000L) pushFix(it) }
            }
        }
    }

    fun stop() {
        try {
            lm?.removeUpdates(listener)
        } catch (_: Exception) {
        }
    }

    private fun has(p: String) = ctx.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    // ------------------------------------------------------------------ location

    private fun startLocation() {
        if (!has(Manifest.permission.ACCESS_FINE_LOCATION) && !has(Manifest.permission.ACCESS_COARSE_LOCATION)) {
            Engine.reporterNote = "Location permission is off, so the phone's position and cells are not read."
            return
        }
        val m = ctx.getSystemService(Context.LOCATION_SERVICE) as LocationManager
        lm = m
        val fine = has(Manifest.permission.ACCESS_FINE_LOCATION)
        for (provider in listOf(LocationManager.NETWORK_PROVIDER, LocationManager.GPS_PROVIDER)) {
            if (provider == LocationManager.GPS_PROVIDER && !fine) continue
            try {
                if (!m.isProviderEnabled(provider)) continue
                m.getLastKnownLocation(provider)?.let { l ->
                    if (lastFix == null || l.time > lastFix!!.time) lastFix = l
                }
                m.requestLocationUpdates(provider, 5_000L, 10f, listener, Looper.getMainLooper())
            } catch (_: SecurityException) {
            } catch (_: IllegalArgumentException) {
            }
        }
        lastFix?.let { f -> scope?.launch { pushFix(f) } }
    }

    private fun pushFix(l: Location) {
        val o = JSONObject()
            .put("lat", l.latitude)
            .put("lon", l.longitude)
            .put("ts", l.time)
            .put("source", l.provider ?: "unknown")
        if (l.hasAccuracy() && l.accuracy > 0f) o.put("accuracy_m", l.accuracy.toDouble())
        Engine.call(ctx, "POST", "/api/location/push", o.toString())
    }

    // ------------------------------------------------------------------ cells

    private suspend fun phoneLoop() {
        val model = "${Build.MANUFACTURER} ${Build.MODEL}".trim()
        while (scope?.isActive != false) {
            if (Engine.state == "stopped") {
                delay(2_000)
                continue
            }
            try {
                val body = buildPush(model)
                val r = Engine.call(ctx, "POST", "/api/phone/push", body.toString())
                if (r != null && r.first !in 200..299) {
                    Engine.reporterNote = "The engine refused a reading: ${r.second.take(120)}"
                } else if (r != null && Engine.reporterNote?.startsWith("The engine refused") == true) {
                    Engine.reporterNote = null
                }
            } catch (e: Exception) {
                Engine.reporterNote = "Reading the phone failed: ${e.javaClass.simpleName}"
            }
            delay(2_000)
        }
    }

    private fun telephony(): TelephonyManager? {
        val base = ctx.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager ?: return null
        val id = try {
            SubscriptionManager.getDefaultDataSubscriptionId()
        } catch (e: Exception) {
            SubscriptionManager.INVALID_SUBSCRIPTION_ID
        }
        return if (id != SubscriptionManager.INVALID_SUBSCRIPTION_ID) base.createForSubscriptionId(id) else base
    }

    private suspend fun cellInfo(tm: TelephonyManager): List<CellInfo> {
        if (!has(Manifest.permission.ACCESS_FINE_LOCATION)) return emptyList()
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                val fresh = withTimeoutOrNull(1_500) {
                    suspendCancellableCoroutine<List<CellInfo>?> { cont ->
                        val exec = Executor { it.run() }
                        try {
                            tm.requestCellInfoUpdate(exec, object : TelephonyManager.CellInfoCallback() {
                                override fun onCellInfo(cellInfo: MutableList<CellInfo>) {
                                    if (cont.isActive) cont.resume(cellInfo)
                                }

                                override fun onError(errorCode: Int, detail: Throwable?) {
                                    if (cont.isActive) cont.resume(null)
                                }
                            })
                        } catch (e: Exception) {
                            if (cont.isActive) cont.resume(null)
                        }
                    }
                }
                if (!fresh.isNullOrEmpty()) return fresh
            }
            return tm.allCellInfo ?: emptyList()
        } catch (e: SecurityException) {
            return emptyList()
        }
    }

    /** One cell as the engine's JSON, plus whether it is the serving (registered) one. */
    private class Cell(val json: JSONObject, val registered: Boolean, val rat: String)

    private fun ok(v: Int, lo: Int, hi: Int) = v != unavailable && v in lo..hi

    private fun JSONObject.putIf(k: String, v: Int, lo: Int, hi: Int) {
        if (ok(v, lo, hi)) put(k, v)
    }

    private fun codeOrNull(s: String?, digits: IntRange): String? =
        s?.takeIf { it.length in digits && it.all { c -> c in '0'..'9' } }

    private fun lteCell(ci: CellInfoLte): Cell {
        val id: CellIdentityLte = ci.cellIdentity
        val ss: CellSignalStrengthLte = ci.cellSignalStrength
        val o = JSONObject().put("rat", "LTE")
        if (ok(id.ci, 1, 0x0FFFFFFF)) o.put("ci", id.ci)
        o.putIf("pci", id.pci, 0, 503)
        if (ok(id.tac, 1, 0xFFFE)) o.put("tac", id.tac)
        o.putIf("earfcn", id.earfcn, 0, 262143)
        val band = (if (Build.VERSION.SDK_INT >= 30) id.bands.firstOrNull() else null) ?: bandFromEarfcn(id.earfcn)
        if (band != null) o.put("band", band)
        val mcc = codeOrNull(if (Build.VERSION.SDK_INT >= 28) id.mccString else null, 3..3)
        val mnc = codeOrNull(if (Build.VERSION.SDK_INT >= 28) id.mncString else null, 2..3)
        if (mcc != null && mnc != null) o.put("mcc", mcc).put("mnc", mnc)
        if (Build.VERSION.SDK_INT >= 26) {
            o.putIf("rsrp", ss.rsrp, -160, -1)
            o.putIf("rsrq", ss.rsrq, -50, 20)
            o.putIf("sinr", ss.rssnr, -50, 60)
            if (Build.VERSION.SDK_INT >= 29) o.putIf("rssi", ss.rssi, -150, -1)
        } else {
            o.putIf("rsrp", ss.dbm, -160, -1)
        }
        return Cell(o, ci.isRegistered, "LTE")
    }

    private fun nrCell(ci: CellInfoNr): Cell {
        val id = ci.cellIdentity as CellIdentityNr
        val ss = ci.cellSignalStrength as CellSignalStrengthNr
        val o = JSONObject().put("rat", "NR")
        if (id.nci > 0 && id.nci != Long.MAX_VALUE) o.put("ci", id.nci)
        o.putIf("pci", id.pci, 0, 1007)
        if (ok(id.tac, 1, 0xFFFFFE)) o.put("tac", id.tac)
        o.putIf("earfcn", id.nrarfcn, 0, 4_000_000)
        val band = if (Build.VERSION.SDK_INT >= 30) id.bands.firstOrNull() else null
        if (band != null) o.put("band", band)
        val mcc = codeOrNull(id.mccString, 3..3)
        val mnc = codeOrNull(id.mncString, 2..3)
        if (mcc != null && mnc != null) o.put("mcc", mcc).put("mnc", mnc)
        o.putIf("rsrp", ss.ssRsrp, -160, -1)
        o.putIf("rsrq", ss.ssRsrq, -50, 20)
        o.putIf("sinr", ss.ssSinr, -50, 60)
        return Cell(o, ci.isRegistered, "NR")
    }

    /** 3G / 2G: only the overall strength is known; they are never listed as neighbours. */
    private fun legacyCell(info: CellInfo): Cell? {
        val (rat, dbm, mcc, mnc) = when (info) {
            is CellInfoWcdma -> {
                val id = info.cellIdentity
                Quad("WCDMA", info.cellSignalStrength.dbm, if (Build.VERSION.SDK_INT >= 28) id.mccString else null, if (Build.VERSION.SDK_INT >= 28) id.mncString else null)
            }
            is CellInfoGsm -> {
                val id = info.cellIdentity
                Quad("GSM", info.cellSignalStrength.dbm, if (Build.VERSION.SDK_INT >= 28) id.mccString else null, if (Build.VERSION.SDK_INT >= 28) id.mncString else null)
            }
            else -> return null
        }
        val o = JSONObject().put("rat", rat)
        if (ok(dbm, -160, -1)) o.put("rssi", dbm)
        val m1 = codeOrNull(mcc, 3..3)
        val m2 = codeOrNull(mnc, 2..3)
        if (m1 != null && m2 != null) o.put("mcc", m1).put("mnc", m2)
        return Cell(o, info.isRegistered, rat)
    }

    private data class Quad(val a: String, val b: Int, val c: String?, val d: String?)

    private suspend fun buildPush(model: String): JSONObject {
        val o = JSONObject().put("model", model).put("android", Build.VERSION.RELEASE ?: "")
        battery(o)
        val tm = telephony()
        if (tm != null && has(Manifest.permission.READ_PHONE_STATE)) {
            try {
                tm.networkOperatorName?.takeIf { it.isNotBlank() }?.let { o.put("operator", it) }
                networkType(tm.dataNetworkType)?.let { o.put("network_type", it) }
                slot()?.let { o.put("sim_slot", it) }
            } catch (_: SecurityException) {
            }
        }
        if (tm == null) return o

        val cells = cellInfo(tm).mapNotNull {
            when (it) {
                is CellInfoLte -> lteCell(it)
                is CellInfoNr -> if (Build.VERSION.SDK_INT >= 29) nrCell(it) else null
                else -> legacyCell(it)
            }
        }
        val serving = cells.firstOrNull { it.registered && it.rat == "LTE" }
            ?: cells.firstOrNull { it.registered && it.rat == "NR" }
            ?: cells.firstOrNull { it.registered }
        if (serving != null) {
            val j = serving.json
            val keys = j.keys()
            while (keys.hasNext()) {
                val k = keys.next()
                // the serving cell's identity is called cell_id in the engine's phone status
                o.put(if (k == "ci") "cell_id" else k, j.get(k))
            }
        }
        val neighbors = JSONArray()
        for (c in cells) {
            if (c === serving || c.rat == "WCDMA" || c.rat == "GSM") continue
            if (!c.json.has("pci") && !c.json.has("ci")) continue
            if (neighbors.length() >= 30) break
            neighbors.put(c.json)
        }
        o.put("neighbors", neighbors)
        return o
    }

    private fun slot(): Int? = try {
        val sm = ctx.getSystemService(Context.TELEPHONY_SUBSCRIPTION_SERVICE) as SubscriptionManager
        val id = SubscriptionManager.getDefaultDataSubscriptionId()
        sm.getActiveSubscriptionInfo(id)?.simSlotIndex?.takeIf { it in 0..7 }
    } catch (e: Exception) {
        null
    }

    private fun battery(o: JSONObject) {
        val i: Intent = ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) ?: return
        val level = i.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = i.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
        if (level >= 0 && scale > 0) o.put("battery_level", (level * 100 / scale).coerceIn(0, 100))
        val temp = i.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE)
        if (temp != Int.MIN_VALUE) o.put("battery_temp_c", temp / 10.0)
        val status = i.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
        if (status != -1) o.put("charging", status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL)
        val plugged = i.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0)
        o.put("usb_powered", plugged and BatteryManager.BATTERY_PLUGGED_USB != 0)
    }

    private fun networkType(t: Int): String? = when (t) {
        TelephonyManager.NETWORK_TYPE_LTE -> "LTE"
        19 -> "LTE_CA"
        20 -> "NR"
        TelephonyManager.NETWORK_TYPE_HSPAP -> "HSPA+"
        TelephonyManager.NETWORK_TYPE_HSPA, TelephonyManager.NETWORK_TYPE_HSDPA, TelephonyManager.NETWORK_TYPE_HSUPA -> "HSPA"
        TelephonyManager.NETWORK_TYPE_UMTS -> "UMTS"
        TelephonyManager.NETWORK_TYPE_EDGE -> "EDGE"
        TelephonyManager.NETWORK_TYPE_GPRS -> "GPRS"
        else -> null
    }

    /** LTE band from the downlink EARFCN, for phones that do not report bands (before Android 11). */
    private fun bandFromEarfcn(e: Int): Int? {
        if (e == unavailable || e < 0) return null
        val table = listOf(
            1 to 0..599, 2 to 600..1199, 3 to 1200..1949, 4 to 1950..2399, 5 to 2400..2649, 7 to 2750..3449,
            8 to 3450..3799, 12 to 5010..5179, 13 to 5180..5279, 17 to 5730..5849, 18 to 5850..5999,
            19 to 6000..6149, 20 to 6150..6449, 25 to 8040..8689, 26 to 8690..9039, 28 to 9210..9659,
            38 to 37750..38249, 39 to 38250..38649, 40 to 38650..39649, 41 to 39650..41589,
        )
        return table.firstOrNull { e in it.second }?.first
    }
}
