package io.kovanica.wallet

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NodePowTest {
    @Test
    fun policy_requires_unmetered_wifi_and_charging_only_when_enabled() {
        assertFalse(NodePow.syncAllowed(true, wifi = true, unmetered = true, charging = false))
        assertFalse(NodePow.syncAllowed(true, wifi = false, unmetered = true, charging = true))
        assertFalse(NodePow.syncAllowed(true, wifi = true, unmetered = false, charging = true))
        assertTrue(NodePow.syncAllowed(true, wifi = true, unmetered = true, charging = true))
        assertTrue(NodePow.syncAllowed(false, wifi = false, unmetered = false, charging = false))
    }

    @Test
    fun finds_a_nested_pow_header_and_ignores_a_bare_parent_reference() {
        val dag = JSONObject(
            """
            {"tip":{"chainId":5,"round":2,"targetBits":524287,"parents":[{"author":0,"round":1,"digest":"aa"}]}}
            """.trimIndent(),
        )
        val header = NodePow.findHeader(dag)
        assertEquals(2, header?.optInt("round"))
        assertEquals("", NodePow.parentJson(header!!))
    }
}
