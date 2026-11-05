package io.kovanica.light

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NetworkProfileTest {
    @Test
    fun testnetProfileUsesChain4() {
        assertEquals(4L, Networks.TESTNET.chainId)
        assertEquals("testnet", Networks.TESTNET.id)
        assertTrue(Networks.TESTNET.enabled)
        assertEquals("Testnet (chain 4)", Networks.TESTNET.label)
    }

    @Test
    fun theLiveTestnetHasAReachableFaucet() {
        assertEquals("https://seed3.kovanica.online/faucet", Networks.TESTNET.faucetUrl)
    }

    @Test
    fun otherProfilesKeepTheirChainIds() {
        assertEquals(1L, Networks.MAINNET.chainId)
        assertEquals(1337L, Networks.LOCAL.chainId)
    }

    @Test
    fun aFreshInstallOpensOnTheLiveTestnet() {
        assertEquals("testnet", Networks.DEFAULT.id)
        assertEquals(Networks.TESTNET, Networks.DEFAULT)
    }
}
