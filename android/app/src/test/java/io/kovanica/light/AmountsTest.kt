package io.kovanica.light

import java.math.BigInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Amounts are typed in token units and stored as integer base units. The send
 * screens rely on [Amounts.parse] rejecting anything that would silently round.
 */
class AmountsTest {
    private fun atoms(value: String): BigInteger = BigInteger(value)

    @Test
    fun parsesWholeAndFractionalUnits() {
        assertEquals(atoms("1000000000"), Amounts.parse("1", 9))
        assertEquals(atoms("1500000000"), Amounts.parse("1.5", 9))
        assertEquals(atoms("500000000"), Amounts.parse("0.5", 9))
        assertEquals(atoms("1234567890"), Amounts.parse("1.23456789", 9))
        assertEquals(atoms("100"), Amounts.parse("1", 2))
        assertEquals(atoms("0"), Amounts.parse("0", 9))
        assertEquals(atoms("7"), Amounts.parse("7", 0))
        assertEquals(atoms("1000"), Amounts.parse("1.000", 3))
    }

    @Test
    fun rejectsAmountsThatWouldRound() {
        assertNull(Amounts.parse("1.2345678901", 9))
        assertNull(Amounts.parse("1.5", 0))
    }

    @Test
    fun rejectsGarbage() {
        for (bad in listOf("", " ", ".", "-1", "+1", "1.2.3", "1,5", "1e9", "abc", "0x10", "1 000")) {
            assertNull("[$bad]", Amounts.parse(bad, 9))
        }
        assertNull(Amounts.parse("1", -1))
        assertNull(Amounts.parse("1", 31))
    }

    @Test
    fun formatAndParseRoundTrip() {
        val value = BigInteger("1234567890123456789")
        assertEquals(value, Amounts.parse(Amounts.format(value, 9), 9))
        assertEquals("12.34", Amounts.format(BigInteger("1234"), 2))
        assertEquals("12", Amounts.format(BigInteger("1200"), 2))
        assertEquals("1.000000001", Amounts.format(BigInteger("1000000001"), 9))
    }
}
