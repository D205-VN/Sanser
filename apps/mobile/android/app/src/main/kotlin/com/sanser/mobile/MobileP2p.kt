package com.sanser.mobile

import org.json.JSONArray
import org.json.JSONObject

/** Blocking checks run on the session worker; cancellation runs independently. */
internal object MobileP2p {
    init { System.loadLibrary("sanser_mobile") }
    private external fun request(json: String): String
    fun call(arguments: Map<String, Any?>): Map<String, Any?> {
        val response = JSONObject(request(JSONObject(arguments).toString()))
        check(response.optBoolean("ok")) { response.optString("error", "Direct failed") }
        @Suppress("UNCHECKED_CAST")
        return decode(response.getJSONObject("result")) as Map<String, Any?>
    }
    private fun decode(value: Any?): Any? = when(value) {
        null, JSONObject.NULL -> null
        is JSONObject -> value.keys().asSequence().associateWith { decode(value.get(it)) }
        is JSONArray -> (0 until value.length()).map { decode(value.get(it)) }
        else -> value
    }
}
