package io.kovanica.wallet

import io.kovanica.chain.ChainIds
import org.json.JSONArray
import org.json.JSONObject
import java.math.BigInteger
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * Canonical WASM contract calls, matching `wallet-web` `buildContractCall`.
 *
 * Amounts are integer atoms. The webview never sees a seed; this object only
 * builds the method name, the bincode args, and the lines a person must read
 * before `signCallHd` / `signCallRaw`.
 */
object ContractCalls {
    const val GAS_LIMIT: ULong = 10_000_000uL

    data class Draft(
        val family: String,
        val action: String,
        val chainId: Long,
        val party: String = "",
        val hash: String = "",
        val secret: String = "",
        val recordId: String = "",
        val timeMs: String = "",
        val amount: String = "",
        val proposalId: String = "",
        val callData: String = "",
    )

    data class Built(
        val method: String,
        val argsHex: String,
        val lines: List<String>,
    )

    data class LogRow(
        val address: String,
        val topics: String,
        val data: String,
        val height: String,
        val hash: String,
    )

    data class ReceiptView(
        val status: String,
        val code: String,
        val gasUsed: String,
        val height: String,
        val hash: String,
        val error: String,
        val logs: List<LogRow>,
    )

    fun actions(family: String): List<Pair<String, String>> = when (family) {
        "token" -> listOf("balance_of" to "balance_of", "token_transfer" to "transfer", "token_mint" to "mint")
        "htlc" -> listOf("htlc_create" to "create", "htlc_claim" to "claim", "htlc_refund" to "refund")
        "vault" -> listOf("vault_create" to "deposit", "vault_claim" to "withdraw", "vault_cancel" to "cancel")
        "multisig" -> listOf("multisig_propose" to "propose", "multisig_confirm" to "confirm", "multisig_execute" to "execute")
        else -> emptyList()
    }

    fun signs(action: String): Boolean = action != "balance_of"

    fun fields(action: String): Set<String> = when (action) {
        "token_transfer", "token_mint" -> setOf("party", "amount")
        "balance_of" -> setOf("party")
        "htlc_create" -> setOf("party", "hash", "time", "amount")
        "htlc_claim" -> setOf("record", "secret")
        "htlc_refund", "vault_claim", "vault_cancel" -> setOf("record")
        "vault_create" -> setOf("party", "time", "amount")
        "multisig_propose" -> setOf("record", "party", "amount", "data")
        "multisig_confirm", "multisig_execute" -> setOf("record", "proposal")
        else -> emptySet()
    }

    fun build(draft: Draft): Built {
        val action = draft.action.trim()
        val chainId = draft.chainId
        val lines = mutableListOf<String>()
        warning(chainId)?.let { lines += it }
        val (method, args) = when (action) {
            "token_transfer" -> {
                val who = key(draft.party, chainId, "Recipient")
                val atoms = u128(draft.amount, "Amount")
                lines += listOf("Action: transfer", "Method: token_transfer", "Target: ${show(who)}", amountLine(atoms, token = true))
                "token_transfer" to concat(who, le(atoms, 16))
            }
            "token_mint" -> {
                val who = key(draft.party, chainId, "Account")
                val atoms = u128(draft.amount, "Amount")
                lines += listOf("Action: mint", "Method: token_mint", "Target: ${show(who)}", amountLine(atoms, token = true))
                "token_mint" to concat(who, le(atoms, 16))
            }
            "htlc_create" -> {
                val who = key(draft.party, chainId, "Beneficiary")
                val hash = hex32(draft.hash, "Hash")
                val atoms = u128(draft.amount, "Amount")
                val whenMs = u64(draft.timeMs, "Timelock")
                lines += listOf(
                    "Action: create",
                    "Method: htlc_create",
                    "Beneficiary: ${show(who)}",
                    "Hash: ${TokenCodec.hex(hash)}",
                    "Locktime: ${timeLabel(whenMs)}",
                    amountLine(atoms, token = false),
                )
                "htlc_create" to concat(who, le(atoms, 16), hash, le(whenMs, 8))
            }
            "htlc_claim" -> {
                val id = hex32(draft.recordId, "Swap id")
                val secret = secretBytes(draft.secret)
                lines += listOf("Action: claim", "Method: htlc_claim", "Swap id: ${TokenCodec.hex(id)}", "Secret: ${TokenCodec.hex(secret)}")
                "htlc_claim" to concat(id, bytesVec(secret))
            }
            "htlc_refund" -> {
                val id = hex32(draft.recordId, "Swap id")
                lines += listOf("Action: refund", "Method: htlc_refund", "Swap id: ${TokenCodec.hex(id)}")
                "htlc_refund" to id
            }
            "vault_create" -> {
                val who = key(draft.party, chainId, "Beneficiary")
                val atoms = u128(draft.amount, "Amount")
                val whenMs = u64(draft.timeMs, "Lock until")
                lines += listOf(
                    "Action: deposit",
                    "Method: vault_create",
                    "Beneficiary: ${show(who)}",
                    "Lock-until: ${timeLabel(whenMs)}",
                    amountLine(atoms, token = false),
                    "Schedule: Absolute",
                )
                "vault_create" to concat(who, le(atoms, 16), le(BigInteger.ZERO, 4), le(whenMs, 8))
            }
            "vault_claim" -> {
                val id = hex32(draft.recordId, "Vault id")
                lines += listOf(
                    "Action: withdraw",
                    "Method: vault_claim",
                    "Vault id: ${TokenCodec.hex(id)}",
                    "Amount: the vested balance. This call does not take an amount.",
                )
                "vault_claim" to id
            }
            "vault_cancel" -> {
                val id = hex32(draft.recordId, "Vault id")
                lines += listOf("Action: cancel", "Method: vault_cancel", "Vault id: ${TokenCodec.hex(id)}")
                "vault_cancel" to id
            }
            "multisig_propose" -> {
                val id = hex32(draft.recordId, "Multisig id")
                val who = key(draft.party, chainId, "Recipient")
                val atoms = u128(draft.amount, "Amount")
                val data = callData(draft.callData)
                lines += listOf(
                    "Action: propose",
                    "Method: multisig_propose",
                    "Multisig id: ${TokenCodec.hex(id)}",
                    "Operation: propose",
                    "Target: ${show(who)}",
                    amountLine(atoms, token = false),
                    "Call data: ${if (data.isEmpty()) "none" else TokenCodec.hex(data)}",
                )
                "multisig_propose" to concat(id, who, le(atoms, 16), bytesVec(data))
            }
            "multisig_confirm", "multisig_execute" -> {
                val id = hex32(draft.recordId, "Multisig id")
                val proposal = u64(draft.proposalId, "Proposal id")
                val operation = if (action == "multisig_confirm") "confirm" else "execute"
                lines += listOf(
                    "Action: $operation",
                    "Method: $action",
                    "Multisig id: ${TokenCodec.hex(id)}",
                    "Operation: $operation",
                    "Proposal id: $proposal",
                )
                action to concat(id, le(proposal, 8))
            }
            else -> throw IllegalArgumentException("That contract action is not one of the typed forms.")
        }
        return Built(method, TokenCodec.hex(args), lines)
    }

    fun warning(chainId: Long): String? {
        if (chainId == ChainIds.HYBRID || chainId == ChainIds.LOCAL) return null
        return "Warning: chain $chainId is not Hybrid PoW Testnet (5) or local (1337)."
    }

    fun decodeReceipt(value: Any?): ReceiptView {
        if (value == null || value === JSONObject.NULL) {
            return ReceiptView("pending", "", "", "", "", "", emptyList())
        }
        if (value is JSONArray) {
            return ReceiptView("pending", "", "", "", "", "", decodeLogs(value))
        }
        val obj = value as? JSONObject ?: return ReceiptView("pending", "", "", "", "", "", emptyList())
        val statusText = obj.optString("status").ifBlank { obj.optString("outcome") }.lowercase(Locale.US)
        val successFlag = if (obj.has("success")) obj.opt("success") else null
        val reverted = statusText == "0x0" || statusText == "0" || statusText.contains("fail") ||
            statusText.contains("revert") || successFlag == false
        val pending = !reverted && (statusText.contains("pend") || (statusText.isEmpty() && !obj.has("blockNumber") && !obj.has("blockHeight") && !obj.has("height")))
        val success = !reverted && !pending
        val code = when {
            reverted -> "0x0"
            success -> "0x1"
            else -> ""
        }
        val error = if (reverted) {
            firstText(obj, listOf("revertReason", "revert_reason", "error", "reason", "message"))
                ?: "The contract reverted."
        } else {
            ""
        }
        return ReceiptView(
            status = when {
                reverted -> "revert"
                pending -> "pending"
                else -> "success"
            },
            code = code,
            gasUsed = quantity(first(obj, listOf("gasUsed", "gas_used"))).orEmpty(),
            height = quantity(first(obj, listOf("blockNumber", "blockHeight", "block_height", "height", "round"))).orEmpty(),
            hash = firstText(obj, listOf("transactionHash", "txHash", "hash")).orEmpty().removePrefix("0x"),
            error = error,
            logs = decodeLogs(obj.opt("logs") ?: obj.opt("events")),
        )
    }

    fun decodeLogs(value: Any?): List<LogRow> {
        val array = when (value) {
            is JSONArray -> value
            is JSONObject -> value.optJSONArray("logs") ?: value.optJSONArray("events") ?: return emptyList()
            else -> return emptyList()
        }
        val out = ArrayList<LogRow>(array.length())
        for (i in 0 until array.length()) {
            val item = array.optJSONObject(i) ?: continue
            val topics = item.optJSONArray("topics")
            val topicText = if (topics == null) {
                ""
            } else {
                (0 until topics.length()).joinToString(", ") { topics.optString(it) }
            }
            out += LogRow(
                address = item.optString("address"),
                topics = topicText,
                data = item.optString("data"),
                height = quantity(first(item, listOf("blockNumber", "blockHeight", "height"))).orEmpty(),
                hash = item.optString("transactionHash").ifBlank { item.optString("hash") }.removePrefix("0x"),
            )
        }
        return out
    }

    fun formatReceipt(view: ReceiptView): String {
        val head = when (view.status) {
            "success" -> "status ${view.code.ifBlank { "0x1" }} Success"
            "revert" -> "status ${view.code.ifBlank { "0x0" }} Revert"
            else -> "status Pending"
        }
        val lines = mutableListOf(head)
        if (view.gasUsed.isNotBlank()) lines += "gasUsed ${view.gasUsed}"
        if (view.height.isNotBlank()) lines += "block ${view.height}"
        if (view.hash.isNotBlank()) lines += "tx ${view.hash}"
        if (view.error.isNotBlank()) lines += view.error
        if (view.logs.isNotEmpty()) {
            lines += "logs ${view.logs.size}"
            view.logs.take(3).forEach { log ->
                val topic = log.topics.ifBlank { "event" }
                lines += "$topic @ ${log.height.ifBlank { "?" }}"
            }
        }
        return lines.joinToString("\n")
    }

    private fun show(key: ByteArray): String = KvncAddress.display(TokenCodec.hex(key))

    private fun amountLine(atoms: BigInteger, token: Boolean): String {
        val label = if (token) "Amount" else "Amount"
        return "$label: ${Amounts.format(atoms, 9)} KVNC (${atoms} atoms)"
    }

    private fun key(raw: String, chainId: Long, label: String): ByteArray {
        val hex = KvncAddress.normalizeHex(raw, chainId)
            ?: throw IllegalArgumentException("$label must be a PRO-21 kvnc…dag address.")
        return TokenCodec.unhex(hex) ?: throw IllegalArgumentException("$label must be a PRO-21 kvnc…dag address.")
    }

    private fun hex32(raw: String, label: String): ByteArray {
        val clean = raw.trim().removePrefix("0x").removePrefix("0X").lowercase(Locale.US)
        if (!clean.matches(Regex("^[0-9a-f]{64}$"))) throw IllegalArgumentException("$label must be 32 bytes of hex.")
        return TokenCodec.unhex(clean) ?: throw IllegalArgumentException("$label must be 32 bytes of hex.")
    }

    private fun u128(raw: String, label: String): BigInteger = integer(raw, label, 128)

    private fun u64(raw: String, label: String): BigInteger = integer(raw, label, 64)

    private fun integer(raw: String, label: String, bits: Int): BigInteger {
        val text = raw.trim()
        if (!text.matches(Regex("^[0-9]+$"))) throw IllegalArgumentException("$label must be a whole number of atoms.")
        val value = BigInteger(text)
        if (value.bitLength() > bits) throw IllegalArgumentException("$label does not fit a $bits-bit integer.")
        return value
    }

    private fun secretBytes(raw: String): ByteArray {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) throw IllegalArgumentException("Enter the secret.")
        val hex = trimmed.removePrefix("0x").removePrefix("0X")
        if (hex.length % 2 == 0 && hex.matches(Regex("(?i)[0-9a-f]+"))) {
            return TokenCodec.unhex(hex) ?: throw IllegalArgumentException("Enter the secret.")
        }
        return trimmed.toByteArray(Charsets.UTF_8)
    }

    private fun callData(raw: String): ByteArray {
        val trimmed = raw.trim().removePrefix("0x").removePrefix("0X")
        if (trimmed.isEmpty()) return ByteArray(0)
        if (trimmed.length % 2 != 0 || !trimmed.matches(Regex("(?i)[0-9a-f]+"))) {
            throw IllegalArgumentException("Call data must be hex.")
        }
        return TokenCodec.unhex(trimmed) ?: throw IllegalArgumentException("Call data must be hex.")
    }

    private fun bytesVec(bytes: ByteArray): ByteArray = concat(le(BigInteger.valueOf(bytes.size.toLong()), 8), bytes)

    private fun le(value: BigInteger, width: Int): ByteArray {
        val out = ByteArray(width)
        val be = value.toByteArray()
        val n = minOf(be.size, width)
        for (i in 0 until n) out[i] = be[be.size - 1 - i]
        return out
    }

    private fun concat(vararg parts: ByteArray): ByteArray {
        val out = ByteArray(parts.sumOf { it.size })
        var offset = 0
        for (part in parts) {
            part.copyInto(out, offset)
            offset += part.size
        }
        return out
    }

    private fun timeLabel(ms: BigInteger): String {
        if (ms.bitLength() > 62) return "$ms ms"
        val format = SimpleDateFormat("yyyy-MM-dd HH:mm:ss 'UTC'", Locale.US)
        format.timeZone = TimeZone.getTimeZone("UTC")
        return "${format.format(Date(ms.toLong()))} ($ms ms)"
    }

    private fun first(obj: JSONObject, keys: List<String>): Any? {
        for (key in keys) if (obj.has(key) && !obj.isNull(key)) return obj.opt(key)
        return null
    }

    private fun firstText(obj: JSONObject, keys: List<String>): String? {
        val value = first(obj, keys) ?: return null
        val text = value.toString().trim()
        return text.ifEmpty { null }
    }

    private fun quantity(value: Any?): String? {
        return when (value) {
            null, JSONObject.NULL -> null
            is Number -> BigInteger.valueOf(value.toLong()).toString()
            is String -> {
                val text = value.trim()
                if (text.isEmpty()) null
                else if (text.startsWith("0x") || text.startsWith("0X")) {
                    val body = text.drop(2).ifEmpty { "0" }
                    if (!body.matches(Regex("(?i)[0-9a-f]+"))) null else BigInteger(body, 16).toString()
                } else if (text.matches(Regex("^[0-9]+$"))) text.trimStart('0').ifEmpty { "0" }
                else null
            }
            else -> null
        }
    }
}
