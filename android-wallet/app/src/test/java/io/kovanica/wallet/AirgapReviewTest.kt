package io.kovanica.wallet

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AirgapReviewTest {
    @Test
    fun refuses_a_payload_for_a_different_chain() {
        assertFalse(AirgapReview.matches(5, 4))
        assertEquals(
            "This transaction is for chain 5. This phone is set to chain 4. Signing is refused.",
            AirgapReview.refusal(5, 4),
        )
        assertTrue(AirgapReview.matches(5, 5))
    }

    @Test
    fun transfer_and_call_rows_name_the_fields_a_person_must_read() {
        val transfer = JSONObject()
            .put("kind", "transfer")
            .put("from", "kvncsender")
            .put("to", "kvncrecipient")
            .put("amount", "1000000000")
            .put("fee", "1000")
            .put("nonce", "7")
            .put("chainId", 5)
        val rows = AirgapReview.lines(transfer)
        assertTrue(rows.any { it == "Amount: 1 KVNC (1000000000 atoms)" })
        assertTrue(rows.any { it == "Fee: 1000 atoms" })
        assertTrue(rows.any { it.startsWith("Recipient:") })

        val call = JSONObject()
            .put("kind", "call")
            .put("from", "kvncsender")
            .put("to", "kvnccontract")
            .put("method", "withdraw")
            .put("args", "aa")
            .put("gas", "100000")
            .put("fee", "10")
            .put("nonce", "1")
            .put("chainId", 5)
        val callRows = AirgapReview.lines(call)
        assertTrue(callRows.any { it == "Method: withdraw" })
        assertTrue(callRows.any { it == "Arguments: aa" })
        assertTrue(callRows.any { it == "Gas limit: 100000" })
    }

    @Test
    fun pruning_window_keeps_fourteen_thousand_four_hundred_rounds() {
        val tip = 20_000L
        val kept = AirgapReview.retain(setOf(tip - 14_401L, tip - 14_400L, tip), tip)
        assertFalse(kept.contains(tip - 14_401L))
        assertTrue(kept.contains(tip - 14_400L))
        assertEquals(2, kept.size)
    }

    @Test
    fun footprint_uses_the_cache_limit() {
        assertEquals("0 B / 64 MiB limit", AirgapReview.footprint(0))
        assertEquals("240 KB / 64 MiB limit", AirgapReview.footprint(240L * 1024L))
    }

    @Test
    fun peer_count_reads_sync_status_or_a_peer_list() {
        assertEquals("3", AirgapReview.peerCount(JSONObject().put("peerCount", 3)))
        assertEquals("2", AirgapReview.peerCount(JSONArray().put("a").put("b")))
        assertNull(AirgapReview.peerCount(JSONObject()))
    }

    @Test
    fun recent_headers_produce_a_block_interval() {
        assertEquals(
            "Recent block interval: not enough verified headers",
            AirgapReview.intervalLabel(listOf(1_000L), 4_000L),
        )
        assertEquals(
            "Recent block interval: 4.0s (target 4s)",
            AirgapReview.intervalLabel(listOf(0L, 4_000L), 4_000L),
        )
        assertEquals("ok", AirgapReview.intervalTone(listOf(0L, 4_200L), 4_000L))
        assertEquals("drift", AirgapReview.intervalTone(listOf(0L, 8_000L), 4_000L))
        assertEquals("idle", AirgapReview.intervalTone(listOf(1_000L), 4_000L))
    }
}
