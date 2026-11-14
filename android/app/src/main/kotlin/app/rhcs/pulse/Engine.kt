package app.rhcs.pulse

import android.content.Context
import android.os.Build
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.SecureRandom

/**
 * Shared facts about the built-in engine (rhcsd running inside this app): where it listens, the
 * per-install access token, its log, and a tiny authenticated HTTP client used by the reporter and
 * the lookup worker. Everything runs in one process, so the status fields are plain variables.
 */
object Engine {
    const val PORT = 7878
    const val BASE = "http://127.0.0.1:$PORT"
    const val LOCAL = "local"

    /** "stopped" | "starting" | "running" | "error" */
    @Volatile var state: String = "stopped"
    @Volatile var error: String? = null
    @Volatile var reporterNote: String? = null
    /** The user (or the Stop button in PULSE) ended the engine: it is not restarted until Start. */
    @Volatile var stoppedByUser: Boolean = false

    /** The user's last choice from the consent screen or the on/off switch: true = engine on. */
    fun enabled(ctx: Context): Boolean = ctx.getSharedPreferences("engine", Context.MODE_PRIVATE).getBoolean("enabled", false)

    /** Whether the consent question was answered yet. */
    fun asked(ctx: Context): Boolean = ctx.getSharedPreferences("engine", Context.MODE_PRIVATE).contains("enabled")

    fun setEnabled(ctx: Context, on: Boolean) {
        ctx.getSharedPreferences("engine", Context.MODE_PRIVATE).edit().putBoolean("enabled", on).apply()
    }

    /** The engine is a 64-bit ARM binary. */
    fun supported(): Boolean = Build.SUPPORTED_ABIS.contains("arm64-v8a")

    private const val ALPHABET = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"

    /** Random per-install token (40 characters), created once and kept in private preferences. Never logged. */
    @Synchronized
    fun token(ctx: Context): String {
        val p = ctx.getSharedPreferences("engine", Context.MODE_PRIVATE)
        p.getString("token", null)?.takeIf { it.length >= 32 }?.let { return it }
        val rnd = SecureRandom()
        val t = buildString { repeat(40) { append(ALPHABET[rnd.nextInt(ALPHABET.length)]) } }
        p.edit().putString("token", t).apply()
        return t
    }

    fun logFile(ctx: Context) = File(ctx.filesDir, "rhcsd.log")

    /** The last [n] lines of the engine's output, for the status panel. */
    fun tailLog(ctx: Context, n: Int = 6): List<String> = try {
        val f = logFile(ctx)
        if (!f.isFile) emptyList() else f.readLines().takeLast(n).map { it.take(160) }
    } catch (e: Exception) {
        emptyList()
    }

    /** One authenticated call to the engine. Null when it could not be reached. */
    fun call(ctx: Context, method: String, path: String, body: String?, timeoutMs: Int = 4000): Pair<Int, String>? {
        var c: HttpURLConnection? = null
        return try {
            c = (URL(BASE + path).openConnection() as HttpURLConnection).apply {
                requestMethod = method
                connectTimeout = timeoutMs
                readTimeout = timeoutMs
                instanceFollowRedirects = false
                setRequestProperty("x-rhcs-token", token(ctx))
                setRequestProperty("Accept", "application/json")
                if (body != null) {
                    doOutput = true
                    setRequestProperty("Content-Type", "application/json")
                    outputStream.use { it.write(body.toByteArray()) }
                }
            }
            val code = c.responseCode
            val stream = if (code in 200..299) c.inputStream else c.errorStream
            code to (stream?.bufferedReader()?.use { it.readText().take(262_144) } ?: "")
        } catch (e: Exception) {
            null
        } finally {
            c?.disconnect()
        }
    }

    /** True once the engine answers /api/health with the token. */
    fun healthy(ctx: Context): Boolean = call(ctx, "GET", "/api/health", null, 1500)?.first == 200
}
