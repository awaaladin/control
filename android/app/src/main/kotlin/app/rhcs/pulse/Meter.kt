package app.rhcs.pulse

/**
 * What the pull-down notification shows: how strong this phone's mobile signal is right now, whether it
 * is getting better or worse, and (only while a game is open) the measured ping. Pure reading of values
 * that are already collected; it sends nothing over the network.
 */
object Meter {
    class View(val percent: Int, val label: String, val trend: String, val title: String, val line: String, val detail: String)

    private val history = ArrayDeque<Int>()
    private var lastAt = 0L

    /** Called every few seconds by the service: remembers the latest signal reading. */
    @Synchronized
    fun record() {
        val r = Engine.lastRsrp
        if (r == null || System.currentTimeMillis() - Engine.lastRsrpAt > 20_000L) return
        val now = System.currentTimeMillis()
        if (now - lastAt < 4_000L) return
        lastAt = now
        history.addLast(r)
        while (history.size > 12) history.removeFirst()
    }

    /** 4 = best .. 0 = worst, from signal STRENGTH (RSRP). */
    private fun strengthLevel(rsrp: Int) = when {
        rsrp >= -85 -> 4
        rsrp >= -95 -> 3
        rsrp >= -105 -> 2
        rsrp >= -115 -> 1
        else -> 0
    }

    /** Same scale from signal QUALITY (RSRQ): a strong signal on a crowded or noisy cell still lags. */
    private fun qualityLevel(rsrq: Int) = when {
        rsrq >= -10 -> 4
        rsrq >= -13 -> 3
        rsrq >= -15 -> 2
        rsrq >= -18 -> 1
        else -> 0
    }

    private val labels = arrayOf("Very weak", "Weak", "Fair", "Good", "Strong")

    @Synchronized
    private fun trend(): String {
        if (history.size < 6) return "Steady"
        val a = history.toList()
        val recent = a.takeLast(3).average()
        val before = a.dropLast(3).takeLast(3).average()
        return when {
            recent - before >= 3 -> "Getting better ▲"
            recent - before <= -3 -> "Getting worse ▼"
            else -> "Steady"
        }
    }

    @Synchronized
    fun view(ctx: android.content.Context): View {
        val r = Engine.lastRsrp?.takeIf { System.currentTimeMillis() - Engine.lastRsrpAt < 20_000L }
        val net = Engine.lastNetType ?: "mobile"
        val game = GameBoost.game
        val pingMs = if (game != null && System.currentTimeMillis() - GameBoost.measuredAt < 60_000L) {
            when (GameBoost.active) {
                "wifi" -> GameBoost.wifiMs
                "cellular" -> GameBoost.cellMs
                else -> null
            }
        } else null
        val pingTxt = when {
            game == null -> null
            pingMs != null -> "Ping $pingMs ms (${if (GameBoost.active == "wifi") "Wi-Fi" else "mobile data"})"
            GameBoost.measuredAt > 0 -> "Ping: no answer"
            else -> "Ping: measuring"
        }
        val tower = when {
            !GameBoost.enabled(ctx) -> null
            Privileged.state(ctx) == "ready" -> "Switch tower: ready"
            else -> "Switch tower: Shizuku is not running"
        }
        if (r == null) {
            val line = listOfNotNull("No mobile signal reading", pingTxt).joinToString(" · ")
            return View(0, "No reading", "", "Network: no signal reading", line, listOfNotNull(line, tower).joinToString("\n"))
        }
        val q = Engine.lastRsrq?.takeIf { it in -30..-1 }
        val sl = strengthLevel(r)
        val ql = q?.let { qualityLevel(it) } ?: 4
        val pctStrength = ((r + 140) * 100 / 70).coerceIn(0, 100)
        val pct = if (q != null) minOf(pctStrength, ((q + 20) * 100 / 17).coerceIn(0, 100)) else pctStrength
        val lb = labels[minOf(sl, ql)]
        val why = when {
            q != null && ql < sl -> "Signal is ${labels[sl].lowercase()} but the channel is noisy or crowded (quality $q dB), so lag is likely"
            else -> null
        }
        val tr = trend()
        val title = "Network: $lb · $r dBm" + if (tr != "Steady") " · $tr" else ""
        val line = listOfNotNull(net, tr, pingTxt).joinToString(" · ")
        val detail = listOfNotNull(
            "$lb on $net: strength $r dBm" + (q?.let { ", quality $it dB" } ?: "") + ", ${pct}%",
            why,
            "Trend: $tr",
            pingTxt,
            tower,
        ).joinToString("\n")
        return View(pct, lb, tr, title, line, detail)
    }
}
