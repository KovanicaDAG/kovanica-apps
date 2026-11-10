package io.kovanica.wallet

import io.kovanica.chain.ChainIds
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.math.BigInteger

class ContractCallsTest {
    private val party = KvncAddress.display("11".repeat(32))

    @Test
    fun token_transfer_encodes_address_and_u128_and_names_the_target() {
        val built = ContractCalls.build(
            ContractCalls.Draft(
                family = "token",
                action = "token_transfer",
                chainId = ChainIds.HYBRID,
                party = party,
                amount = "1000000000",
            ),
        )
        assertEquals("token_transfer", built.method)
        assertEquals(96, built.argsHex.length)
        assertTrue(built.lines.any { it.startsWith("Target: kvnc") && it.endsWith("dag") })
        assertTrue(built.lines.any { it.contains("1000000000 atoms") })
        assertFalse(built.lines.any { it.startsWith("Warning:") })
    }

    @Test
    fun htlc_and_vault_layouts_match_the_console() {
        val hash = "ab".repeat(32)
        val htlc = ContractCalls.build(
            ContractCalls.Draft(
                family = "htlc",
                action = "htlc_create",
                chainId = ChainIds.LOCAL,
                party = party,
                hash = hash,
                timeMs = "1700000000000",
                amount = "1000",
            ),
        )
        assertEquals("htlc_create", htlc.method)
        assertEquals(176, htlc.argsHex.length)
        assertTrue(htlc.lines.any { it.startsWith("Locktime:") && it.contains("1700000000000 ms") })

        val vault = ContractCalls.build(
            ContractCalls.Draft(
                family = "vault",
                action = "vault_create",
                chainId = ChainIds.HYBRID,
                party = party,
                timeMs = "1700000000000",
                amount = "1000",
            ),
        )
        assertEquals(120, vault.argsHex.length)
        assertTrue(vault.lines.any { it == "Schedule: Absolute" })

        val claim = ContractCalls.build(
            ContractCalls.Draft(family = "vault", action = "vault_claim", chainId = 5, recordId = hash),
        )
        assertEquals(64, claim.argsHex.length)
        assertTrue(claim.lines.any { it.startsWith("Vault id:") })
    }

    @Test
    fun mainnet_rejects_a_legacy_hex_address() {
        var failed = false
        try {
            ContractCalls.build(
                ContractCalls.Draft(
                    family = "token",
                    action = "token_mint",
                    chainId = ChainIds.MAINNET,
                    party = "0x" + "22".repeat(32),
                    amount = "1",
                ),
            )
        } catch (error: IllegalArgumentException) {
            failed = error.message?.contains("PRO-21") == true
        }
        assertTrue(failed)
    }

    @Test
    fun other_chains_carry_a_warning_badge() {
        val built = ContractCalls.build(
            ContractCalls.Draft(
                family = "multisig",
                action = "multisig_confirm",
                chainId = ChainIds.DEVNET,
                recordId = "cd".repeat(32),
                proposalId = "7",
            ),
        )
        assertEquals("multisig_confirm", built.method)
        assertTrue(built.lines.first().startsWith("Warning:"))
        assertTrue(built.lines.any { it == "Proposal id: 7" })
    }

    @Test
    fun receipt_decodes_success_revert_and_logs() {
        val pending = ContractCalls.decodeReceipt(null)
        assertEquals("pending", pending.status)

        val ok = JSONObject()
            .put("status", "0x1")
            .put("gasUsed", "0x5208")
            .put("blockNumber", "0x10")
            .put("transactionHash", "0xabc")
            .put("logs", JSONArray().put(JSONObject().put("topics", JSONArray().put("transfer")).put("blockNumber", "0x10").put("transactionHash", "0xabc")))
        val success = ContractCalls.decodeReceipt(ok)
        assertEquals("success", success.status)
        assertEquals("0x1", success.code)
        assertEquals("21000", success.gasUsed)
        assertEquals("16", success.height)
        assertEquals(1, success.logs.size)
        assertTrue(ContractCalls.formatReceipt(success).contains("Success"))

        val bad = JSONObject().put("status", "0x0").put("revertReason", "locked")
        val revert = ContractCalls.decodeReceipt(bad)
        assertEquals("revert", revert.status)
        assertEquals("0x0", revert.code)
        assertEquals("locked", revert.error)
    }

    @Test
    fun shortfall_faucet_and_describe_roundtrip() {
        assertEquals(BigInteger.valueOf(6000), ContractCalls.callNeed("htlc_create", "0", "1000", "5000"))
        assertEquals(BigInteger.valueOf(1000), ContractCalls.callNeed("token_transfer", "0", "1000", "5000"))
        assertEquals(BigInteger.valueOf(2000), ContractCalls.shortfall(BigInteger.valueOf(4000), BigInteger.valueOf(6000)))
        assertEquals(BigInteger.ZERO, ContractCalls.shortfall(BigInteger.valueOf(6000), BigInteger.valueOf(6000)))
        assertTrue(ContractCalls.faucetQuick(ChainIds.HYBRID))
        assertTrue(ContractCalls.faucetQuick(ChainIds.LOCAL))
        assertFalse(ContractCalls.faucetQuick(ChainIds.MAINNET))
        assertFalse(ContractCalls.faucetQuick(3))
        assertEquals("Token", ContractCalls.family("token_transfer"))
        assertEquals("HTLC", ContractCalls.family("htlc_create"))
        assertEquals("Vault", ContractCalls.family("vault_claim"))
        assertEquals("Multisig", ContractCalls.family("multisig_execute"))

        val built = ContractCalls.build(
            ContractCalls.Draft(
                family = "token",
                action = "token_transfer",
                chainId = ChainIds.HYBRID,
                party = party,
                amount = "1000000000",
            ),
        )
        val described = ContractCalls.describe(built.method, built.argsHex, ChainIds.HYBRID)
        assertNotNull(described)
        assertTrue(described!!.any { it.startsWith("Target: kvnc") && it.endsWith("dag") })
        assertTrue(described.any { it.contains("1000000000") })
        assertNull(ContractCalls.describe("withdraw", "aa", ChainIds.HYBRID))
    }
}
