package io.kovanica.wallet

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.math.BigInteger

class ContractEncodersTest {

    private val chainId = 2L // testnet (enforces canonical kvnc...dag)
    private val keyBytes = ByteArray(32) { 0x22 }
    private val keyHex = TokenCodec.hex(keyBytes)
    private val address = KvncAddress.display(keyHex)

    private val hashLockHex = "44".repeat(32)
    private val swapIdHex = "55".repeat(32)
    private val vaultIdHex = "66".repeat(32)
    private val msIdHex = "77".repeat(32)

    @Test
    fun testHtlcCreate() {
        val call = ContractEncoders.encodeHtlcCreate(
            claimer = address,
            amountAtoms = BigInteger.valueOf(1000L),
            hashLockHex = hashLockHex,
            timelockMs = 9L,
            chainId = chainId,
        )
        assertEquals("htlc_create", call.method)

        // Expected: key (32) + amount (u128le = 16) + hash (32) + timelockMs (u64le = 8) = 88 bytes
        assertEquals(88, call.args.size)
        val expected = keyBytes +
            ContractEncoders.u128le(BigInteger.valueOf(1000L)) +
            TokenCodec.unhex(hashLockHex)!! +
            ContractEncoders.u64le(9L)
        assertTrue(expected.contentEquals(call.args))
    }

    @Test
    fun testHtlcClaim() {
        val call = ContractEncoders.encodeHtlcClaim(
            swapIdHex = swapIdHex,
            secret = "preimage",
        )
        assertEquals("htlc_claim", call.method)
        val secretBytes = "preimage".toByteArray(Charsets.UTF_8)
        val expected = TokenCodec.unhex(swapIdHex)!! +
            ContractEncoders.u64le(secretBytes.size.toLong()) +
            secretBytes
        assertTrue(expected.contentEquals(call.args))
    }

    @Test
    fun testHtlcRefund() {
        val call = ContractEncoders.encodeHtlcRefund(swapIdHex = swapIdHex)
        assertEquals("htlc_refund", call.method)
        assertTrue(TokenCodec.unhex(swapIdHex)!!.contentEquals(call.args))
    }

    @Test
    fun testVaultCreate() {
        val call = ContractEncoders.encodeVaultCreate(
            beneficiary = address,
            amountAtoms = BigInteger.valueOf(5000L),
            lockUntilMs = 50L,
            chainId = chainId,
        )
        assertEquals("vault_create", call.method)
        // Expected: benKey (32) + amount (16) + u32le(0) (4) + u64le(50) (8) = 60 bytes
        assertEquals(60, call.args.size)
        val expected = keyBytes +
            ContractEncoders.u128le(BigInteger.valueOf(5000L)) +
            ContractEncoders.u32le(0L) +
            ContractEncoders.u64le(50L)
        assertTrue(expected.contentEquals(call.args))
    }

    @Test
    fun testVaultClaimAndCancel() {
        val claim = ContractEncoders.encodeVaultClaim(vaultIdHex)
        assertEquals("vault_claim", claim.method)
        assertTrue(TokenCodec.unhex(vaultIdHex)!!.contentEquals(claim.args))

        val cancel = ContractEncoders.encodeVaultCancel(vaultIdHex)
        assertEquals("vault_cancel", cancel.method)
        assertTrue(TokenCodec.unhex(vaultIdHex)!!.contentEquals(cancel.args))
    }

    @Test
    fun testMultisigProposeConfirmExecute() {
        val propose = ContractEncoders.encodeMultisigPropose(
            multisigIdHex = msIdHex,
            recipient = address,
            amountAtoms = BigInteger.valueOf(100L),
            callDataHex = "",
            chainId = chainId,
        )
        assertEquals("multisig_propose", propose.method)
        val expectedPropose = TokenCodec.unhex(msIdHex)!! +
            keyBytes +
            ContractEncoders.u128le(BigInteger.valueOf(100L)) +
            ContractEncoders.u64le(0L)
        assertTrue(expectedPropose.contentEquals(propose.args))

        val confirm = ContractEncoders.encodeMultisigConfirm(msIdHex, 3L)
        assertEquals("multisig_confirm", confirm.method)
        val expectedConfirm = TokenCodec.unhex(msIdHex)!! + ContractEncoders.u64le(3L)
        assertTrue(expectedConfirm.contentEquals(confirm.args))

        val execute = ContractEncoders.encodeMultisigExecute(msIdHex, 3L)
        assertEquals("multisig_execute", execute.method)
        assertTrue(expectedConfirm.contentEquals(execute.args))
    }

    @Test
    fun testTokenTransferAndMint() {
        val transfer = ContractEncoders.encodeTokenTransfer(
            recipient = address,
            tokenUnits = BigInteger.valueOf(5L),
            chainId = chainId,
        )
        assertEquals("token_transfer", transfer.method)
        val expectedTransfer = keyBytes + ContractEncoders.u128le(BigInteger.valueOf(5L))
        assertTrue(expectedTransfer.contentEquals(transfer.args))

        val mint = ContractEncoders.encodeTokenMint(
            account = address,
            tokenUnits = BigInteger.valueOf(8L),
            chainId = chainId,
        )
        assertEquals("token_mint", mint.method)
        val expectedMint = keyBytes + ContractEncoders.u128le(BigInteger.valueOf(8L))
        assertTrue(expectedMint.contentEquals(mint.args))
    }
}
