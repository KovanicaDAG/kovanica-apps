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

    @Test
    fun node_store_local_db_path_is_scoped_under_files_dir() {
        val filesDir = java.io.File("/tmp/kovanica-test-files")
        val context = object : android.content.ContextWrapper(null) {
            override fun getFilesDir(): java.io.File = filesDir
        }
        val path = NodeStore.localDbPath(context)
        assertTrue(path.endsWith("mobile_node.redb"))
        assertTrue(path.startsWith(filesDir.absolutePath))
    }

    @Test
    fun node_store_tip_round_defaults_to_zero_on_missing_db() {
        val filesDir = java.io.File("/tmp/kovanica-nonexistent-" + System.currentTimeMillis())
        val context = object : android.content.ContextWrapper(null) {
            override fun getFilesDir(): java.io.File = filesDir
        }
        assertEquals(0L, NodeStore.tipRound(context))
    }

    @Test
    fun finds_a_pow_header_from_block_with_decoded_fields() {
        val block = JSONObject(
            """
            {"author":0,"number":"0xa","round":"0xa","chain_id":5,"target_bits":"0x217fffff","parents":[]}
            """.trimIndent(),
        )
        val header = NodePow.findHeader(block)
        assertTrue(header != null)
        assertEquals(5, header?.optInt("chain_id"))
    }

    @Test
    fun node_store_latest_state_root_null_on_missing_db() {
        val filesDir = java.io.File("/tmp/kovanica-m4-nonexistent-" + System.currentTimeMillis())
        val context = object : android.content.ContextWrapper(null) {
            override fun getFilesDir(): java.io.File = filesDir
        }
        assertEquals(null, NodeStore.latestStateRoot(context))
    }

    @Test
    fun node_store_simulate_call_graceful_on_missing_db() {
        val filesDir = java.io.File("/tmp/kovanica-m4-sim-" + System.currentTimeMillis())
        val context = object : android.content.ContextWrapper(null) {
            override fun getFilesDir(): java.io.File = filesDir
        }
        val sim = NodeStore.simulateCall(
            context,
            "kvnc1111111111111111111111111111111111111111111111111111111111dag",
            "kvnc2222222222222222222222222222222222222222222222222222222222dag",
            "test",
            "0x"
        )
        // If native library is loaded in JVM test environment, it returns a failed simulation result;
        // if uniffi native lib is not loaded, runCatching caught it and returned null. In either case, it must not throw!
        assertTrue(sim == null || !sim.success)
    }
}
