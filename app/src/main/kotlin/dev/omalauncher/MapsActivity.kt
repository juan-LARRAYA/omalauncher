package dev.omalauncher

import android.Manifest
import android.annotation.SuppressLint
import android.content.pm.PackageManager
import android.graphics.Color as AndroidColor
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import java.io.File
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import org.maplibre.android.MapLibre
import org.maplibre.android.camera.CameraPosition
import org.maplibre.android.camera.CameraUpdateFactory
import org.maplibre.android.geometry.LatLng
import org.maplibre.android.geometry.LatLngBounds
import org.maplibre.android.location.LocationComponentActivationOptions
import org.maplibre.android.maps.MapLibreMap
import org.maplibre.android.maps.MapView
import org.maplibre.android.maps.Style
import org.maplibre.android.style.expressions.Expression
import org.maplibre.android.style.layers.CircleLayer
import org.maplibre.android.style.layers.LineLayer
import org.maplibre.android.style.layers.Property
import org.maplibre.android.style.layers.PropertyFactory
import org.maplibre.android.style.sources.GeoJsonSource

private data class Place(val name: String, val detail: String, val lat: Double, val lon: Double)

private class Step(val title: String, val detail: String, val color: Color?)

private class Trip(val time: String, val summary: String, val note: String, val steps: List<Step>)

private val Ink = Color(0xFF1B1D1C)
private val Muted = Color(0xFF5D6662)
private val Line = Color(0xFFD9DEDC)
private val Accent = Color(0xFF1F6F5C)
private val FromColor = "#2b6cb0"
private val ToColor = "#c2410c"

/**
 * Mapas, a real app: the map is drawn by MapLibre Native from files on the phone, and search and routes come from
 * mapas-core, in Rust, reading the same files. No browser, no server, no network: the style has no remote URL.
 * The data is what `mapas-datos` built in the guest's home; Linux doesn't need to be running. It runs in a process
 * of its own (:mapas), so a crash here never takes the launcher and Linux down with it.
 */
class MapsActivity : ComponentActivity() {
    private val data by lazy { File(filesDir, "arch/root/.local/share/mapas") }
    private val logFile by lazy { File(filesDir, "arch/root/.cache/mapas-nativo.log") }
    private lateinit var mapView: MapView
    private var map: MapLibreMap? = null
    private var loadedStyle: Style? = null
    private var locationOn = false

    private var origin by mutableStateOf<Place?>(null)
    private var destination by mutableStateOf<Place?>(null)
    private var fromText by mutableStateOf("")
    private var toText by mutableStateOf("")
    private var mode by mutableStateOf("auto")
    private var trip by mutableStateOf<Trip?>(null)
    private var message by mutableStateOf<String?>(null)
    private var pressed by mutableStateOf<LatLng?>(null)
    private val results = mutableStateListOf<Place>()
    private var activeField = "hasta"

    private val askLocation =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
            if (granted.values.any { it }) enableLocation()
        }

    private fun log(text: String) {
        runCatching {
            logFile.parentFile?.mkdirs()
            logFile.appendText("${LocalDateTime.now(ZONE).format(STAMP)} $text\n")
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        log("abre: datos en $data")
        val missing = REQUIRED.filterNot { File(data, it).exists() }
        if (missing.isNotEmpty()) {
            log("faltan: $missing")
            setContent { Box(Modifier.fillMaxSize().background(Color.White).safeDrawingPadding().padding(24.dp)) { BasicText("Faltan los datos del mapa: ${missing.joinToString()}. Se arman en Linux con mapas-datos.") } }
            return
        }
        MapasCore.open(data.path)
        MapLibre.getInstance(this)
        mapView = MapView(this)
        mapView.onCreate(savedInstanceState)
        mapView.getMapAsync(::onMap)
        setContent { Screen() }
        if (hasLocation()) enableLocation() else askLocation.launch(LOCATION)
    }

    private fun hasLocation() = LOCATION.any { checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }

    private fun onMap(m: MapLibreMap) {
        map = m
        m.uiSettings.isAttributionEnabled = false // shown as text: its dialog needs an AppCompat theme
        m.uiSettings.isLogoEnabled = false
        m.uiSettings.isCompassEnabled = false
        m.uiSettings.isRotateGesturesEnabled = false
        m.uiSettings.isTiltGesturesEnabled = false
        m.cameraPosition = CameraPosition.Builder().target(LatLng(-34.6075, -58.4370)).zoom(12.0).build()
        m.addOnMapLongClickListener { pressed = it; true }
        m.addOnMapClickListener { pressed = null; results.clear(); false }
        mapView.addOnDidFailLoadingMapListener { log("el mapa no cargó: $it"); message = "El mapa no cargó: $it" }
        val json = File(data, "style.json").readText().replace("{{DATOS}}", data.path)
        m.setStyle(Style.Builder().fromJson(json)) { style ->
            log("estilo cargado: ${style.layers.size} capas")
            style.addSource(GeoJsonSource(ROUTE))
            style.addSource(GeoJsonSource(POINTS))
            style.addLayer(
                LineLayer("ruta-borde", ROUTE).withFilter(Expression.neq(Expression.get("tipo"), "pie")).withProperties(
                    PropertyFactory.lineColor(AndroidColor.WHITE), PropertyFactory.lineWidth(9f),
                    PropertyFactory.lineCap(Property.LINE_CAP_ROUND), PropertyFactory.lineJoin(Property.LINE_JOIN_ROUND),
                ),
            )
            style.addLayer(
                LineLayer("ruta", ROUTE).withFilter(Expression.neq(Expression.get("tipo"), "pie")).withProperties(
                    PropertyFactory.lineColor(Expression.toColor(Expression.get("color"))), PropertyFactory.lineWidth(5f),
                    PropertyFactory.lineCap(Property.LINE_CAP_ROUND), PropertyFactory.lineJoin(Property.LINE_JOIN_ROUND),
                ),
            )
            style.addLayer(
                LineLayer("ruta-pie", ROUTE).withFilter(Expression.eq(Expression.get("tipo"), "pie")).withProperties(
                    PropertyFactory.lineColor(AndroidColor.rgb(93, 102, 98)), PropertyFactory.lineWidth(4f),
                    PropertyFactory.lineCap(Property.LINE_CAP_ROUND), PropertyFactory.lineDasharray(arrayOf(0.1f, 2f)),
                ),
            )
            style.addLayer(
                CircleLayer("puntos", POINTS).withProperties(
                    PropertyFactory.circleRadius(8f), PropertyFactory.circleColor(Expression.toColor(Expression.get("color"))),
                    PropertyFactory.circleStrokeColor(AndroidColor.WHITE), PropertyFactory.circleStrokeWidth(3f),
                ),
            )
            loadedStyle = style
            enableLocation()
        }
    }

    @SuppressLint("MissingPermission")
    private fun enableLocation() {
        val (m, style) = (map ?: return) to (loadedStyle ?: return)
        if (locationOn || !hasLocation()) return
        runCatching {
            val options = LocationComponentActivationOptions.builder(this, style).useDefaultLocationEngine(true).build()
            m.locationComponent.activateLocationComponent(options)
            m.locationComponent.isLocationComponentEnabled = true
            locationOn = true
        }.onFailure { log("ubicación: $it") }
    }

    private fun myPlace(): Place? =
        runCatching { map?.locationComponent?.lastKnownLocation }.getOrNull()?.let { Place("Mi ubicación", "", it.latitude, it.longitude) }

    private fun goToMe() {
        val me = myPlace()
        if (me == null) {
            message = "Todavía no tengo tu ubicación."
            if (!hasLocation()) askLocation.launch(LOCATION)
            return
        }
        map?.animateCamera(CameraUpdateFactory.newLatLngZoom(LatLng(me.lat, me.lon), 16.0))
    }

    private fun showPoints() {
        val features = JSONArray()
        for ((p, color) in listOf(origin to FromColor, destination to ToColor)) {
            if (p != null) features.put(feature("Point", JSONArray().put(p.lon).put(p.lat), "punto", color))
        }
        loadedStyle?.getSourceAs<GeoJsonSource>(POINTS)?.setGeoJson(collection(features))
    }

    private fun clearTrip() {
        trip = null
        loadedStyle?.getSourceAs<GeoJsonSource>(ROUTE)?.setGeoJson(collection(JSONArray()))
    }

    private fun choose(field: String, place: Place, scope: CoroutineScope) {
        results.clear()
        if (field == "desde") {
            origin = place
            fromText = place.name
        } else {
            destination = place
            toText = place.name
        }
        showPoints()
        map?.animateCamera(CameraUpdateFactory.newLatLng(LatLng(place.lat, place.lon)))
        route(scope)
    }

    private fun route(scope: CoroutineScope) {
        val to = destination ?: return
        val from = origin ?: myPlace()
        if (from == null) {
            message = "No tengo tu ubicación: escribí desde dónde salís."
            return
        }
        val transit = mode == "transporte"
        scope.launch {
            message = if (transit) "Buscando colectivos, subtes y trenes…" else "Calculando la ruta…"
            val now = LocalDateTime.now(ZONE)
            val text =
                withContext(Dispatchers.IO) {
                    if (transit) {
                        MapasCore.transit(from.lat, from.lon, to.lat, to.lon, now.dayOfWeek.value - 1, now.toLocalTime().toSecondOfDay().toDouble())
                    } else {
                        MapasCore.car(from.lat, from.lon, to.lat, to.lon)
                    }
                }
            val r = JSONObject(text)
            if (r.has("error")) {
                log("ruta: ${r.getString("error")}")
                clearTrip()
                message = if (transit) "No encontré un viaje en transporte público a esta hora." else "No encontré un camino en auto."
                return@launch
            }
            message = null
            if (transit) showTransit(r, now) else showCar(r, now)
            fit(r.getJSONArray("coords"))
        }
    }

    private fun showCar(r: JSONObject, now: LocalDateTime) {
        val secs = r.getLong("segundos")
        val features = JSONArray().put(feature("LineString", r.getJSONArray("coords"), "auto", "#1f6f5c"))
        loadedStyle?.getSourceAs<GeoJsonSource>(ROUTE)?.setGeoJson(collection(features))
        val steps = r.getJSONArray("pasos").objects().map { Step(it.getString("calle"), distance(it.getLong("metros")), null) }
        trip = Trip(duration(secs), "${distance(r.getLong("metros"))} en auto · llegás ${now.plusSeconds(secs).format(CLOCK)}",
            "Sin tránsito ni giros prohibidos: el tiempo es aproximado.", steps)
    }

    private fun showTransit(r: JSONObject, now: LocalDateTime) {
        val secs = r.getLong("segundos")
        val legs = r.getJSONArray("tramos").objects()
        val features = JSONArray()
        val steps =
            legs.map { t ->
                val kind = t.getString("tipo")
                val color = t.optString("color").takeIf { it.isNotEmpty() }?.let { "#$it" } ?: MODE_COLOR[kind] ?: "#1f6f5c"
                features.put(feature("LineString", t.getJSONArray("coords"), if (kind == "pie") "pie" else "linea", color))
                val time = duration(t.getLong("segundos"))
                if (kind == "pie") {
                    val to = t.getString("hasta").let { if (it == "destino") "el destino" else it }
                    Step("Caminá ${distance(t.getLong("metros"))}", "hasta $to · $time", null)
                } else {
                    Step("$kind ${t.getString("linea")}",
                        "hacia ${t.getString("destino")} · de ${t.getString("desde")} a ${t.getString("hasta")}, ${t.getInt("paradas")} paradas · esperás ~${duration(t.getLong("espera"))}, viajás $time",
                        Color(AndroidColor.parseColor(color)))
                }
            }
        loadedStyle?.getSourceAs<GeoJsonSource>(ROUTE)?.setGeoJson(collection(features))
        val changes = legs.count { it.getString("tipo") != "pie" } - 1
        val extra = if (changes > 0) " · $changes combinación${if (changes > 1) "es" else ""}" else ""
        trip = Trip(duration(secs), "en transporte · llegás ${now.plusSeconds(secs).format(CLOCK)}$extra",
            "Esperas según la frecuencia de cada línea. Datos abiertos de la Ciudad (colectivos 2019, trenes 2020, subte 2023): aproximados.", steps)
    }

    private fun fit(coords: JSONArray) {
        val points = (0 until coords.length()).map { coords.getJSONArray(it).let { c -> LatLng(c.getDouble(1), c.getDouble(0)) } }
        if (points.size < 2) return
        val bounds = LatLngBounds.Builder().includes(points).build()
        val height = resources.displayMetrics.heightPixels
        map?.animateCamera(CameraUpdateFactory.newLatLngBounds(bounds, 80, (height * 0.26).toInt(), 80, (height * 0.42).toInt()))
    }

    @Composable
    private fun Screen() {
        val scope = rememberCoroutineScope()
        BackHandler(enabled = trip != null || results.isNotEmpty() || pressed != null) {
            pressed = null
            if (results.isNotEmpty()) results.clear() else clearTrip()
        }
        Box(Modifier.fillMaxSize()) {
            AndroidView(factory = { mapView }, modifier = Modifier.fillMaxSize())
            Column(Modifier.fillMaxSize().safeDrawingPadding()) {
                Column(Modifier.padding(10.dp).fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(Color.White).padding(horizontal = 12.dp, vertical = 4.dp)) {
                    Field("desde", fromText, "Mi ubicación", Color(0xFF2B6CB0)) { fromText = it }
                    Field("hasta", toText, "¿Adónde vas?", Color(0xFFC2410C)) { toText = it }
                    Row(Modifier.padding(vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Mode("auto", "Auto", Modifier.weight(1f), scope)
                        Mode("transporte", "Transporte", Modifier.weight(1f), scope)
                    }
                    if (results.isNotEmpty()) {
                        Column(Modifier.heightIn(max = 320.dp).verticalScroll(rememberScrollState())) {
                            for (p in results) {
                                Column(Modifier.fillMaxWidth().clickable { choose(activeField, p, scope) }.padding(vertical = 9.dp)) {
                                    BasicText(p.name, style = TextStyle(fontSize = 16.sp, color = Ink))
                                    BasicText(p.detail, style = TextStyle(fontSize = 13.sp, color = Muted))
                                }
                            }
                        }
                    }
                }
                Box(Modifier.weight(1f).fillMaxWidth()) {
                    BasicText("© OpenStreetMap · Protomaps", Modifier.align(Alignment.BottomStart).padding(8.dp), style = TextStyle(fontSize = 11.sp, color = Ink))
                    BasicText("◎", Modifier.align(Alignment.BottomEnd).padding(14.dp).size(52.dp).clip(CircleShape).background(Color.White).clickable { goToMe() }.padding(10.dp),
                        style = TextStyle(fontSize = 24.sp, color = Accent))
                    pressed?.let { point ->
                        Column(Modifier.align(Alignment.Center).clip(RoundedCornerShape(10.dp)).background(Color.White).padding(6.dp)) {
                            for ((field, text) in listOf("hasta" to "Ir acá", "desde" to "Salir de acá")) {
                                BasicText(text, Modifier.clickable {
                                    pressed = null
                                    choose(field, Place("%.5f, %.5f".format(point.latitude, point.longitude), "", point.latitude, point.longitude), scope)
                                }.padding(horizontal = 18.dp, vertical = 11.dp), style = TextStyle(fontSize = 16.sp, color = Ink))
                            }
                        }
                    }
                    message?.let { text ->
                        LaunchedEffect(text) {
                            delay(5000)
                            if (message == text) message = null
                        }
                        BasicText(text, Modifier.align(Alignment.BottomCenter).padding(bottom = 74.dp, start = 16.dp, end = 16.dp).clip(RoundedCornerShape(10.dp)).background(Ink).padding(12.dp),
                            style = TextStyle(fontSize = 15.sp, color = Color.White))
                    }
                }
                trip?.let { t -> Sheet(t) }
            }
        }
    }

    @Composable
    private fun Field(
        name: String,
        value: String,
        hint: String,
        dot: Color,
        onChange: (String) -> Unit,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Box(Modifier.size(12.dp).clip(CircleShape).background(dot))
            BasicTextField(
                value = value,
                onValueChange = {
                    activeField = name
                    onChange(it)
                    if (it.isBlank()) {
                        if (name == "desde") origin = null else destination = null
                        showPoints()
                    }
                },
                singleLine = true,
                textStyle = TextStyle(fontSize = 17.sp, color = Ink),
                cursorBrush = SolidColor(Accent),
                modifier = Modifier.weight(1f).padding(vertical = 12.dp),
                decorationBox = { inner ->
                    Box {
                        if (value.isEmpty()) BasicText(hint, style = TextStyle(fontSize = 17.sp, color = Muted))
                        inner()
                    }
                },
            )
        }
        val query = value.trim()
        LaunchedEffect(query) {
            val chosen = (if (name == "desde") origin else destination)?.name
            if (activeField != name || query.length < 3 || query == chosen) return@LaunchedEffect
            delay(200)
            val center = map?.cameraPosition?.target ?: LatLng(-34.6075, -58.4370)
            val found = withContext(Dispatchers.IO) { MapasCore.search(query, center.latitude, center.longitude) }
            if (found.trimStart().startsWith("{")) {
                log("búsqueda: $found")
                return@LaunchedEffect
            }
            results.clear()
            results.addAll(JSONArray(found).objects().map { Place(it.getString("nombre"), it.getString("tipo"), it.getDouble("lat"), it.getDouble("lon")) })
            if (results.isEmpty()) message = "Sin resultados en el mapa del teléfono."
        }
    }

    @Composable
    private fun Mode(
        id: String,
        label: String,
        modifier: Modifier,
        scope: CoroutineScope,
    ) {
        val on = mode == id
        Box(modifier.clip(RoundedCornerShape(10.dp)).background(if (on) Accent else Color.White).border(1.dp, if (on) Accent else Line, RoundedCornerShape(10.dp))
            .clickable { mode = id; route(scope) }.padding(vertical = 8.dp), contentAlignment = Alignment.Center) {
            BasicText(label, style = TextStyle(fontSize = 15.sp, color = if (on) Color.White else Ink))
        }
    }

    @Composable
    private fun Sheet(t: Trip) {
        Column(Modifier.fillMaxWidth().heightIn(max = 300.dp).clip(RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp)).background(Color.White).verticalScroll(rememberScrollState()).padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                BasicText(t.time, style = TextStyle(fontSize = 23.sp, color = Accent, fontWeight = FontWeight.Bold))
                BasicText(t.summary, Modifier.weight(1f), style = TextStyle(fontSize = 15.sp, color = Muted))
                BasicText("✕", Modifier.clickable { clearTrip() }.padding(8.dp), style = TextStyle(fontSize = 18.sp, color = Muted))
            }
            BasicText(t.note, Modifier.padding(vertical = 6.dp), style = TextStyle(fontSize = 13.sp, color = Muted))
            for (s in t.steps) {
                Row(Modifier.padding(vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.size(width = 4.dp, height = 34.dp).background(s.color ?: Line))
                    Column {
                        BasicText(s.title, style = TextStyle(fontSize = 16.sp, color = Ink))
                        BasicText(s.detail, style = TextStyle(fontSize = 13.sp, color = Muted))
                    }
                }
            }
        }
    }

    override fun onStart() {
        super.onStart()
        if (::mapView.isInitialized) mapView.onStart()
    }

    override fun onResume() {
        super.onResume()
        if (::mapView.isInitialized) mapView.onResume()
    }

    override fun onPause() {
        if (::mapView.isInitialized) mapView.onPause()
        super.onPause()
    }

    override fun onStop() {
        if (::mapView.isInitialized) mapView.onStop()
        super.onStop()
    }

    override fun onLowMemory() {
        super.onLowMemory()
        if (::mapView.isInitialized) mapView.onLowMemory()
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        if (::mapView.isInitialized) mapView.onSaveInstanceState(outState)
    }

    override fun onDestroy() {
        if (::mapView.isInitialized) mapView.onDestroy()
        super.onDestroy()
    }

    private companion object {
        val ZONE: ZoneId = ZoneId.of("America/Argentina/Buenos_Aires")
        val CLOCK: DateTimeFormatter = DateTimeFormatter.ofPattern("HH:mm")
        val STAMP: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss")
        const val ROUTE = "ruta"
        const val POINTS = "extremos"
        val REQUIRED = listOf("style.json", "buenos-aires.pmtiles", "busqueda.sqlite", "grafo-auto.bin", "transporte.bin")
        val LOCATION = arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION)
        val MODE_COLOR = mapOf("Subte" to "#2b6cb0", "Tren" to "#7c3aed", "Colectivo" to "#c2410c", "Tranvía" to "#0f766e")

        fun JSONArray.objects(): List<JSONObject> = (0 until length()).map { getJSONObject(it) }

        fun feature(
            type: String,
            coordinates: JSONArray,
            kind: String,
            color: String,
        ): JSONObject =
            JSONObject().put("type", "Feature").put("properties", JSONObject().put("tipo", kind).put("color", color))
                .put("geometry", JSONObject().put("type", type).put("coordinates", coordinates))

        fun collection(features: JSONArray): String = JSONObject().put("type", "FeatureCollection").put("features", features).toString()

        fun distance(m: Long): String = if (m < 1000) "${(m + 5) / 10 * 10} m" else "%.1f km".format(m / 1000.0).replace('.', ',')

        fun duration(seconds: Long): String {
            val m = maxOf(1, (seconds + 30) / 60)
            return if (m < 60) "$m min" else "${m / 60} h ${m % 60} min"
        }
    }
}
