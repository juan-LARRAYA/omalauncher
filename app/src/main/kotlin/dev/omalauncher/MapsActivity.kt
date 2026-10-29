package dev.omalauncher

import android.Manifest
import android.annotation.SuppressLint
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.WindowInsets
import android.webkit.GeolocationPermissions
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts

/**
 * Mapas: the maps app served by the guest (phone/apps/mapas in Juan's omarchi-kb: offline map, search and
 * routes), in a window of its own instead of a browser tab. Location comes from Android, asked for the first
 * time the page wants it, and only for the guest's own address. If the service doesn't answer, it asks the
 * guest to start it and tries again.
 */
class MapsActivity : ComponentActivity() {
    private val runtime get() = (application as App).runtime
    private lateinit var web: WebView
    private var retries = 0
    private var pendingLocation: GeolocationPermissions.Callback? = null

    private val askLocation =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
            pendingLocation?.invoke(ORIGIN, granted.values.any { it }, false)
            pendingLocation = null
        }

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        web = WebView(this)
        web.setBackgroundColor(Color.WHITE)
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        web.settings.setGeolocationEnabled(true)
        // The page runs edge to edge under the system bars: keep its search box and route sheet clear of them.
        web.setOnApplyWindowInsetsListener { v, insets ->
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.ime())
                v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            } else {
                @Suppress("DEPRECATION")
                v.setPadding(insets.systemWindowInsetLeft, insets.systemWindowInsetTop, insets.systemWindowInsetRight, insets.systemWindowInsetBottom)
            }
            insets
        }
        web.webViewClient =
            object : WebViewClient() {
                // Only the guest's page loads here; it links nowhere else.
                override fun shouldOverrideUrlLoading(
                    view: WebView,
                    request: WebResourceRequest,
                ) = !request.url.toString().startsWith(ORIGIN)

                override fun onReceivedError(
                    view: WebView,
                    request: WebResourceRequest,
                    error: WebResourceError,
                ) {
                    if (!request.isForMainFrame) return
                    if (retries++ < MAX_RETRIES) {
                        if (runtime.linuxUp) runtime.launch(START_SERVICE)
                        view.postDelayed({ view.loadUrl(ORIGIN) }, RETRY_MS)
                    } else {
                        view.loadData(UNAVAILABLE, "text/html; charset=utf-8", "utf-8")
                    }
                }

                override fun onPageFinished(
                    view: WebView,
                    url: String,
                ) {
                    if (url.startsWith(ORIGIN)) retries = 0
                }
            }
        web.webChromeClient =
            object : WebChromeClient() {
                override fun onGeolocationPermissionsShowPrompt(
                    origin: String,
                    callback: GeolocationPermissions.Callback,
                ) {
                    if (!ORIGIN.startsWith(origin.trimEnd('/'))) return callback.invoke(origin, false, false)
                    if (LOCATION.any { checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }) return callback.invoke(origin, true, false)
                    pendingLocation = callback
                    askLocation.launch(LOCATION)
                }
            }
        onBackPressedDispatcher.addCallback(
            this,
            object : OnBackPressedCallback(true) {
                override fun handleOnBackPressed() {
                    if (web.canGoBack()) web.goBack() else finish()
                }
            },
        )
        setContentView(web)
        if (savedInstanceState == null || web.restoreState(savedInstanceState) == null) web.loadUrl(ORIGIN)
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        web.saveState(outState)
    }

    override fun onResume() {
        super.onResume()
        web.onResume()
    }

    override fun onPause() {
        web.onPause()
        super.onPause()
    }

    override fun onDestroy() {
        web.destroy()
        super.onDestroy()
    }

    companion object {
        /** The guest's maps service (phone-services `mapas`), straight to its port: no browser, no proxy. */
        private const val ORIGIN = "http://127.0.0.1:8104/"
        private const val START_SERVICE = "/root/omarchi-kb/phone/apps/servicios/phone-services start mapas"
        private const val MAX_RETRIES = 10
        private const val RETRY_MS = 1500L
        private val LOCATION = arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION)
        private const val UNAVAILABLE =
            "<body style='font:18px sans-serif;padding:24px'><h2>Mapas no responde</h2>" +
                "<p>El servicio <b>mapas</b> del Linux no arrancó. Abrí el panel de servicios y tocá Iniciar.</p></body>"
    }
}
