package io.kovanica.wallet

import org.json.JSONArray
import org.json.JSONObject

/** Pure checks for the mobile node service. No Android types, so unit tests can call them. */
object NodePow {
    fun syncAllowed(enforceWifiAndCharging: Boolean, wifi: Boolean, unmetered: Boolean, charging: Boolean): Boolean {
        if (!enforceWifiAndCharging) return true
        return wifi && unmetered && charging
    }

    const val MAX_BATTERY_TEMP_TENTHS = 360 // 36.0°C thermal protection gate

    fun isThermalSafe(batteryTempTenths: Int): Boolean {
        if (batteryTempTenths <= 0) return true // simulator / mock without battery sensor
        return batteryTempTenths < MAX_BATTERY_TEMP_TENTHS
    }

    fun findHeader(value: Any?): JSONObject? {
        when (value) {
            is JSONObject -> {
                if (isHeader(value)) return value
                val keys = value.keys()
                while (keys.hasNext()) {
                    findHeader(value.opt(keys.next()))?.let { return it }
                }
            }
            is JSONArray -> {
                for (index in 0 until value.length()) {
                    findHeader(value.opt(index))?.let { return it }
                }
            }
        }
        return null
    }

    fun parentJson(header: JSONObject): String {
        val parents = header.optJSONArray("parents") ?: return ""
        for (index in 0 until parents.length()) {
            findHeader(parents.opt(index))?.let { return it.toString() }
        }
        return ""
    }

    private fun isHeader(value: JSONObject): Boolean {
        val hasRound = value.has("round")
        val hasTarget = value.has("target_bits") || value.has("targetBits")
        val hasChain = value.has("chain_id") || value.has("chainId")
        return hasRound && hasTarget && hasChain
    }
}
