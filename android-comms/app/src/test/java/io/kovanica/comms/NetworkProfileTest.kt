package io.kovanica.comms

import io.kovanica.chain.ChainIds
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NetworkProfileTest {
    @Test
    fun testnetProfileIsTheLiveChain4() {
        assertEquals("testnet", Networks.TESTNET.id)
        assertEquals(ChainIds.TESTNET, Networks.TESTNET.chainId)
        assertEquals(4L, Networks.TESTNET.chainId)
        assertEquals("https://rpc-testnet.kovanica.online/rpc", Networks.TESTNET.rpcUrl)
    }

    @Test
    fun aFreshInstallOpensOnTheTestnet() {
        assertEquals("testnet", Networks.DEFAULT.id)
        assertEquals(Networks.TESTNET, Networks.DEFAULT)
    }

    @Test
    fun localStaysSelectableForDevelopment() {
        assertEquals(1337L, Networks.LOCAL.chainId)
        assertTrue(Networks.ALL.contains(Networks.LOCAL))
    }

    @Test
    fun byRpcMatchesTheBuiltInEndpointsAndIgnoresATrailingSlash() {
        assertEquals(Networks.TESTNET, Networks.byRpc("https://rpc-testnet.kovanica.online/rpc"))
        assertEquals(Networks.TESTNET, Networks.byRpc(" https://rpc-testnet.kovanica.online/rpc/ "))
        assertEquals(Networks.LOCAL, Networks.byRpc("http://10.0.2.2:8545/rpc"))
        assertNull(Networks.byRpc("https://example.invalid/rpc"))
    }
}
