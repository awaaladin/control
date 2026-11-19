package app.rhcs.pulse

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** The "Switch tower" button on the lag popup. The tap is the person's confirmation. */
class TowerSwitchReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        val pending = goAsync()
        Thread({
            try {
                Privileged.repickFromPopup(context.applicationContext)
            } catch (_: Throwable) {
            } finally {
                pending.finish()
            }
        }, "pulse-tower-switch").start()
    }

    companion object {
        const val ACTION = "app.rhcs.pulse.SWITCH_TOWER"
    }
}
