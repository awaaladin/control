package app.rhcs.pulse

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.webkit.GeolocationPermissions
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.ListView
import android.widget.ProgressBar
import android.widget.TextView
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * PULSE for Android: the full PULSE dashboard in a WebView, loaded straight from the home
 * system, plus a "Find my system" screen that scans the phone's networks for it.
 */
class MainActivity : AppCompatActivity() {

    private data class Row(val title: String, val sub: String, val url: String, val isHub: Boolean)

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var scanJob: Job? = null

    private lateinit var web: WebView
    private lateinit var finder: View
    private lateinit var findButton: View
    private lateinit var status: TextView
    private lateinit var progress: ProgressBar
    private lateinit var list: ListView
    private lateinit var closeButton: Button
    private lateinit var addr: EditText
    private lateinit var token: EditText

    private val prefs by lazy { getSharedPreferences("pulse", Context.MODE_PRIVATE) }
    private var currentHub: String? = null

    private val hubRows = mutableListOf<Row>()
    private val otherRows = mutableListOf<Row>()
    private val shown = mutableListOf<Row>()
    private lateinit var adapter: ArrayAdapter<Row>

    private var pendingGeo: Pair<String, GeolocationPermissions.Callback>? = null
    private var pendingMic: PermissionRequest? = null

    private val permissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        pendingGeo?.let { (origin, cb) -> cb.invoke(origin, result.values.any { it }, false) }
        pendingGeo = null
        pendingMic?.let { req ->
            if (result[Manifest.permission.RECORD_AUDIO] == true) req.grant(req.resources) else req.deny()
        }
        pendingMic = null
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        web = findViewById(R.id.web)
        finder = findViewById(R.id.finder)
        findButton = findViewById(R.id.findButton)
        status = findViewById(R.id.status)
        progress = findViewById(R.id.progress)
        list = findViewById(R.id.list)
        closeButton = findViewById(R.id.closeButton)
        addr = findViewById(R.id.addr)
        token = findViewById(R.id.token)

        adapter = object : ArrayAdapter<Row>(this, 0, shown) {
            override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
                val v = convertView ?: LayoutInflater.from(context).inflate(R.layout.item_row, parent, false)
                val row = getItem(position)!!
                v.findViewById<TextView>(R.id.rowTitle).text = row.title
                v.findViewById<TextView>(R.id.rowSub).text = row.sub
                return v
            }
        }
        list.adapter = adapter
        list.setOnItemClickListener { _, _, position, _ ->
            val row = shown[position]
            if (row.isHub) openHub(row.url, null) else tryAddress(row.url, null)
        }

        findViewById<Button>(R.id.scanButton).setOnClickListener { startScan() }
        closeButton.setOnClickListener { hideFinder() }
        findViewById<Button>(R.id.connectButton).setOnClickListener { connectTyped() }
        addr.setOnEditorActionListener { _, actionId, _ ->
            if (actionId == EditorInfo.IME_ACTION_GO) {
                connectTyped()
                true
            } else false
        }
        findButton.setOnClickListener {
            showFinder(null)
            startScan()
        }

        setupWeb()

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                when {
                    finder.visibility == View.VISIBLE && currentHub != null -> hideFinder()
                    finder.visibility != View.VISIBLE && web.canGoBack() -> web.goBack()
                    else -> {
                        isEnabled = false
                        onBackPressedDispatcher.onBackPressed()
                    }
                }
            }
        })

        startup()
    }

    // ------------------------------------------------------------------ web view

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWeb() {
        val s = web.settings
        s.javaScriptEnabled = true
        s.domStorageEnabled = true
        s.mediaPlaybackRequiresUserGesture = false
        s.setGeolocationEnabled(true)
        s.cacheMode = WebSettings.LOAD_DEFAULT
        s.userAgentString = s.userAgentString + " PulseApp/0.1"
        web.setBackgroundColor(0xFF05080C.toInt())
        if ((applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0) WebView.setWebContentsDebuggingEnabled(true)

        web.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val uri = request.url
                val hub = currentHub?.let { Uri.parse(it) }
                if (hub != null && uri.host == hub.host && uri.port == hub.port) return false
                // Anything outside the home system (e.g. the OpenCellID sign-up link) opens in the browser.
                if (uri.scheme == "http" || uri.scheme == "https") {
                    try {
                        startActivity(Intent(Intent.ACTION_VIEW, uri))
                    } catch (_: Exception) {
                    }
                }
                return true
            }

            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (request.isForMainFrame) {
                    showFinder("Can't reach ${request.url.host}. Scanning for your system…")
                    startScan()
                }
            }
        }

        web.webChromeClient = object : WebChromeClient() {
            override fun onGeolocationPermissionsShowPrompt(origin: String, callback: GeolocationPermissions.Callback) {
                if (granted(Manifest.permission.ACCESS_FINE_LOCATION) || granted(Manifest.permission.ACCESS_COARSE_LOCATION)) {
                    callback.invoke(origin, true, false)
                } else {
                    pendingGeo = origin to callback
                    permissions.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
                }
            }

            override fun onPermissionRequest(request: PermissionRequest) {
                runOnUiThread {
                    val wantsMic = request.resources.contains(PermissionRequest.RESOURCE_AUDIO_CAPTURE)
                    if (wantsMic && !granted(Manifest.permission.RECORD_AUDIO)) {
                        pendingMic = request
                        permissions.launch(arrayOf(Manifest.permission.RECORD_AUDIO))
                    } else {
                        request.grant(request.resources)
                    }
                }
            }
        }
    }

    private fun granted(p: String) = ContextCompat.checkSelfPermission(this, p) == PackageManager.PERMISSION_GRANTED

    // ------------------------------------------------------------------ finder

    private fun startup() {
        val saved = prefs.getString("hub", null)
        if (saved == null) {
            showFinder("Looking for your system…")
            startScan()
            return
        }
        showFinder("Reconnecting to $saved …")
        scope.launch {
            val h = withContext(Dispatchers.IO) { Finder.health(saved, 1500) }
            if (h != null) {
                openHub(saved, null)
            } else {
                status.text = "Can't reach $saved. Scanning your networks…"
                startScan()
            }
        }
    }

    private fun showFinder(message: String?) {
        finder.visibility = View.VISIBLE
        findButton.visibility = View.GONE
        closeButton.visibility = if (currentHub != null) View.VISIBLE else View.GONE
        if (message != null) status.text = message
    }

    private fun hideFinder() {
        finder.visibility = View.GONE
        findButton.visibility = View.VISIBLE
        hideKeyboard()
    }

    private fun hideKeyboard() {
        val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        imm.hideSoftInputFromWindow(addr.windowToken, 0)
    }

    private fun refreshList() {
        shown.clear()
        shown.addAll(hubRows)
        shown.addAll(otherRows.sortedBy { r -> r.url.substringAfter("//").substringBefore(":").substringAfterLast('.').toIntOrNull() ?: 0 })
        adapter.notifyDataSetChanged()
    }

    private fun startScan() {
        scanJob?.cancel()
        hubRows.clear()
        otherRows.clear()
        refreshList()
        progress.visibility = View.VISIBLE
        status.text = "Scanning your networks…"
        scanJob = scope.launch {
            Finder.scan(
                onStatus = { status.text = it },
                onHub = { ip, h, isPhone ->
                    val title = if (isPhone) "This phone (Termux hub)" else "Home system · $ip"
                    val sub = buildString {
                        append("rhcsd ").append(h.version)
                        h.router?.let { append(" · ").append(it) }
                        if (h.needsToken) append(" · needs an access token")
                        append("\nTap to open")
                    }
                    hubRows.add(Row(title, sub, "http://$ip:${Finder.PORT}", true))
                    refreshList()
                },
                onAlive = { ip ->
                    otherRows.add(Row("Device · $ip", "Responding, but not the home system. Tap to try it anyway.", "http://$ip:${Finder.PORT}", false))
                    refreshList()
                },
            )
            progress.visibility = View.GONE
            val openOnly = hubRows.size == 1 && currentHub == null && !hubRows[0].sub.contains("needs an access token")
            val message = when {
                hubRows.isEmpty() && otherRows.isEmpty() ->
                    "No system found. Is rhcsd running on the laptop, and is the laptop on this Wi-Fi or hotspot?"
                hubRows.isEmpty() ->
                    "No system found, but ${otherRows.size} other device(s) responded. Is rhcsd running on the laptop?"
                openOnly -> "Found it. Opening…"
                else -> "Found ${hubRows.size} system(s). Tap one to open it."
            }
            status.text = message
            if (openOnly) openHub(hubRows[0].url, null)
        }
    }

    // ------------------------------------------------------------------ connecting

    /** "192.168.0.5", "192.168.0.5:7878", "https://x.trycloudflare.com" -> a base URL without a path. */
    private fun normalizeHub(raw: String): String? {
        var s = raw.trim().trimEnd('/')
        if (s.isEmpty()) return null
        if (!s.contains("://")) s = "http://$s"
        val uri = try {
            Uri.parse(s)
        } catch (e: Exception) {
            return null
        }
        val host = uri.host ?: return null
        val scheme = uri.scheme ?: "http"
        val port = when {
            uri.port != -1 -> ":${uri.port}"
            scheme == "http" -> ":${Finder.PORT}"
            else -> ""
        }
        return "$scheme://$host$port"
    }

    private fun connectTyped() {
        var text = addr.text.toString().trim()
        var tok: String? = token.text.toString().trim().ifEmpty { null }
        // A pairing link: https://…/#hub=<address>&token=<token>
        val hashAt = text.indexOf('#')
        if (hashAt >= 0) {
            val params = text.substring(hashAt + 1).split("&").mapNotNull { p ->
                val i = p.indexOf('=')
                if (i > 0) p.substring(0, i) to Uri.decode(p.substring(i + 1)) else null
            }.toMap()
            text = params["hub"] ?: text.substring(0, hashAt)
            params["token"]?.let { tok = it }
        }
        val url = normalizeHub(text)
        if (url == null) {
            status.text = "That doesn't look like an address. Try something like 192.168.0.245:7878"
            return
        }
        hideKeyboard()
        tryAddress(url, tok)
    }

    private fun tryAddress(url: String, tok: String?) {
        status.text = "Trying $url …"
        scope.launch {
            val h = withContext(Dispatchers.IO) { Finder.health(url, 2500) }
            if (h != null) openHub(url, tok) else status.text = "Nothing answered at $url. Is it running, and are you on the same network?"
        }
    }

    private fun openHub(url: String, tok: String?) {
        val t = tok ?: prefs.getString("t:$url", null)
        prefs.edit().putString("hub", url).apply()
        if (tok != null) prefs.edit().putString("t:$url", tok).apply()
        currentHub = url
        progress.visibility = View.GONE
        hideFinder()
        // The pairing fragment lets PULSE store the access token for this system (ignored if not needed).
        val target = if (t.isNullOrEmpty()) url else "$url/#hub=${Uri.encode(url)}&token=${Uri.encode(t)}"
        web.loadUrl(target)
    }

    override fun onDestroy() {
        scope.cancel()
        web.destroy()
        super.onDestroy()
    }
}
