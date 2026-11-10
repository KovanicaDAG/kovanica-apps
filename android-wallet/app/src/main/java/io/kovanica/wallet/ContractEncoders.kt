package io.kovanica.wallet

import java.math.BigInteger
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * A prepared contract call ready for `signCallHd` or review.
 */
data class EncodedCall(
    val method: String,
    val args: ByteArray,
    val gasLimit: Long = 100_000L,
    val value: Long = 0L,
) {
    val argsHex: String get() = TokenCodec.hex(args)

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is EncodedCall) return false
        return method == other.method &&
            args.contentEquals(other.args) &&
            gasLimit == other.gasLimit &&
            value == other.value
    }

    override fun hashCode(): Int {
        var result = method.hashCode()
        result = 31 * result + args.contentHashCode()
        result = 31 * result + gasLimit.hashCode()
        result = 31 * result + value.hashCode()
        return result
    }
}

/**
 * Canonical Bincode 1.x encoders for Kovanica smart contract entry points.
 * Matches `docs/CONTRACTS.md` and TypeScript `src/core/contracts.ts`.
 */
object ContractEncoders {

    fun u32le(v: Long): ByteArray =
        ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(v.toInt()).array()

    fun u64le(v: Long): ByteArray =
        ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putLong(v).array()

    fun u128le(v: BigInteger): ByteArray {
        require(v >= BigInteger.ZERO) { "u128 must be non-negative" }
        val raw = v.toByteArray()
        val out = ByteArray(16)
        var outIdx = 0
        for (i in raw.indices.reversed()) {
            if (outIdx < 16) {
                out[outIdx++] = raw[i]
            }
        }
        return out
    }

    fun bytesVec(bytes: ByteArray): ByteArray = u64le(bytes.size.toLong()) + bytes

    fun decodeKey(address: String, chainId: Long): ByteArray {
        val hex = KvncAddress.normalizeHex(address, chainId)
            ?: throw IllegalArgumentException("Invalid address: $address on chain $chainId")
        val bytes = TokenCodec.unhex(hex)
            ?: throw IllegalArgumentException("Failed to decode address hex: $hex")
        require(bytes.size == 32) { "Address key must be 32 bytes" }
        return bytes
    }

    fun hex32(hexStr: String, label: String): ByteArray {
        val clean = hexStr.trim().removePrefix("0x").removePrefix("0X").lowercase()
        require(clean.length == 64) { "$label must be 32 bytes (64 hex characters)" }
        val bytes = TokenCodec.unhex(clean)
            ?: throw IllegalArgumentException("$label is not valid hex")
        return bytes
    }

    fun secretBytes(secret: String): ByteArray {
        val trimmed = secret.trim()
        require(trimmed.isNotEmpty()) { "Secret must not be empty" }
        val clean = trimmed.removePrefix("0x").removePrefix("0X")
        if (clean.length % 2 == 0 && clean.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }) {
            TokenCodec.unhex(clean)?.let { return it }
        }
        return trimmed.toByteArray(Charsets.UTF_8)
    }

    // HTLC
    fun encodeHtlcCreate(
        claimer: String,
        amountAtoms: BigInteger,
        hashLockHex: String,
        timelockMs: Long,
        chainId: Long,
    ): EncodedCall {
        val claimerKey = decodeKey(claimer, chainId)
        val hash = hex32(hashLockHex, "hash")
        val args = claimerKey + u128le(amountAtoms) + hash + u64le(timelockMs)
        return EncodedCall("htlc_create", args)
    }

    fun encodeHtlcClaim(swapIdHex: String, secret: String): EncodedCall {
        val id = hex32(swapIdHex, "swap id")
        val sec = secretBytes(secret)
        val args = id + bytesVec(sec)
        return EncodedCall("htlc_claim", args)
    }

    fun encodeHtlcRefund(swapIdHex: String): EncodedCall {
        val id = hex32(swapIdHex, "swap id")
        return EncodedCall("htlc_refund", id)
    }

    // Vault
    fun encodeVaultCreate(
        beneficiary: String,
        amountAtoms: BigInteger,
        lockUntilMs: Long,
        chainId: Long,
    ): EncodedCall {
        val benKey = decodeKey(beneficiary, chainId)
        val args = benKey + u128le(amountAtoms) + u32le(0L) + u64le(lockUntilMs)
        return EncodedCall("vault_create", args)
    }

    fun encodeVaultClaim(vaultIdHex: String): EncodedCall {
        val id = hex32(vaultIdHex, "vault id")
        return EncodedCall("vault_claim", id)
    }

    fun encodeVaultCancel(vaultIdHex: String): EncodedCall {
        val id = hex32(vaultIdHex, "vault id")
        return EncodedCall("vault_cancel", id)
    }

    // Multisig
    fun encodeMultisigPropose(
        multisigIdHex: String,
        recipient: String,
        amountAtoms: BigInteger,
        callDataHex: String,
        chainId: Long,
    ): EncodedCall {
        val id = hex32(multisigIdHex, "multisig id")
        val recKey = decodeKey(recipient, chainId)
        val data = if (callDataHex.trim().isEmpty()) {
            ByteArray(0)
        } else {
            TokenCodec.unhex(callDataHex) ?: throw IllegalArgumentException("Invalid call data hex")
        }
        val args = id + recKey + u128le(amountAtoms) + bytesVec(data)
        return EncodedCall("multisig_propose", args)
    }

    fun encodeMultisigConfirm(multisigIdHex: String, proposalId: Long): EncodedCall {
        val id = hex32(multisigIdHex, "multisig id")
        val args = id + u64le(proposalId)
        return EncodedCall("multisig_confirm", args)
    }

    fun encodeMultisigExecute(multisigIdHex: String, proposalId: Long): EncodedCall {
        val id = hex32(multisigIdHex, "multisig id")
        val args = id + u64le(proposalId)
        return EncodedCall("multisig_execute", args)
    }

    // Token
    fun encodeTokenTransfer(recipient: String, tokenUnits: BigInteger, chainId: Long): EncodedCall {
        val recKey = decodeKey(recipient, chainId)
        val args = recKey + u128le(tokenUnits)
        return EncodedCall("token_transfer", args)
    }

    fun encodeTokenMint(account: String, tokenUnits: BigInteger, chainId: Long): EncodedCall {
        val accKey = decodeKey(account, chainId)
        val args = accKey + u128le(tokenUnits)
        return EncodedCall("token_mint", args)
    }
}
