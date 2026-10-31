package dev.omalauncher

/** Mapas' engine in Rust (mapas-core): offline search, car routes and public transport. Every answer is JSON text. */
object MapasCore {
    init {
        System.loadLibrary("mapas_core")
    }

    external fun open(dir: String)

    external fun search(
        q: String,
        lat: Double,
        lon: Double,
    ): String

    external fun car(
        lat1: Double,
        lon1: Double,
        lat2: Double,
        lon2: Double,
    ): String

    /** [day]: 0 is Monday; [now]: seconds since midnight. */
    external fun transit(
        lat1: Double,
        lon1: Double,
        lat2: Double,
        lon2: Double,
        day: Int,
        now: Double,
    ): String

    /** Starts guiding along the last [car] route; false if there's none. */
    external fun guideStart(): Boolean

    /** A new GPS fix: the next turn, what's left, and whether the route was left. */
    external fun guideUpdate(
        lat: Double,
        lon: Double,
    ): String
}
