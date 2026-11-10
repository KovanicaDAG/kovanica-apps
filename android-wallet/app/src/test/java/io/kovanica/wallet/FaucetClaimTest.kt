package io.kovanica.wallet

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FaucetClaimTest {
    @Test
    fun faucet_is_only_for_dev_hybrid_and_local() {
        assertTrue(FaucetClaim.allowed(5))
        assertTrue(FaucetClaim.allowed(3))
        assertTrue(FaucetClaim.allowed(1337))
        assertFalse(FaucetClaim.allowed(1))
        assertFalse(FaucetClaim.allowed(4))
        assertEquals(30_000L, FaucetClaim.remaining(1_000L, 31_000L))
    }

    @Test
    fun seed_quiz_uses_three_positions() {
        val phrase = (1..24).joinToString(" ") { "w$it" }
        assertEquals(listOf(3, 8, 14), SeedCheck.positions(24))
        assertEquals(listOf(3, 8, 12), SeedCheck.positions(12))
        assertTrue(SeedCheck.matches(phrase, listOf("w3", "w8", "w14")))
        assertFalse(SeedCheck.matches(phrase, listOf("w3", "w8", "nope")))
    }
}
