package io.kovanica.wallet

import org.json.JSONArray
import org.json.JSONObject

/** Readable checks for an unsigned air-gap payload. No UI types. */
object AirgapReview {
    fun matches(payloadChain: Long, walletChain: Long): Boolean = payloadChain == walletChain

    fun refusal(payloadChain: Long, walletChain: Long): String =
        "This transaction is for chain $payloadChain. This phone is set to chain $walletChain. Signing is refused."

    fun lines(doc: JSONObject): List<String> {
        val call = doc.optString("kind") == "call"
        val amount = doc.optString("amount")
        val rows = mutableListOf(
            "Sender: ${doc.optString("from")}",
            if (call) "Contract: ${doc.optString("to")}" else "Recipient: ${doc.optString("to")}",
        )
        if (call) {
            val decoded = ContractCalls.describe(doc.optString("method"), doc.optString("args"), doc.optLong("chainId"))
            if (decoded != null) {
                rows += decoded
            } else {
                rows += "Method: ${doc.optString("method").ifBlank { "—" }}"
                rows += "Arguments: ${argsSummary(doc.optString("args"))}"
            }
            rows += "Value: ${doc.optString("value", "0")} atoms"
            rows += "Gas limit: ${doc.optString("gas", "0")}"
        } else {
            rows += "Amount: ${kvnc(amount)}"
        }
        rows += "Fee: ${doc.optString("fee")} atoms"
        rows += "Nonce: ${doc.optString("nonce")}"
        rows += "Chain: ${doc.optLong("chainId")}"
        return rows
    }

    fun retain(rounds: Set<Long>, tip: Long, window: Long = 14_400L): Set<Long> {
        if (window <= 0L) return setOf(tip)
        return rounds.filter { it <= tip && tip - it <= window }.toSet() + tip
    }

    fun footprint(bytes: Long, limitMb: Long = 64): String = "${humanBytes(bytes)} / $limitMb MiB limit"

    fun humanBytes(bytes: Long): String {
        if (bytes <= 0L) return "0 B"
        if (bytes < 1024L) return "$bytes B"
        val kb = (bytes + 512L) / 1024L
        if (kb < 1024L) return "$kb KB"
        val mb = (bytes + (512L * 1024L)) / (1024L * 1024L)
        return "$mb MiB"
    }

    fun peerCount(value: Any?): String? {
        return when (value) {
            is JSONArray -> value.length().toString()
            is JSONObject -> {
                val direct = if (value.has("peerCount")) value.opt("peerCount") else value.opt("peer_count")
                when (direct) {
                    is Number -> direct.toLong().toString()
                    is String -> direct.toLongOrNull()?.toString()
                    else -> {
                        val peers = value.opt("peers")
                        if (peers is JSONArray) peers.length().toString() else null
                    }
                }
            }
            else -> null
        }
    }

    fun medianIntervalMs(timestamps: List<Long>): Long? {
        val sorted = timestamps.filter { it > 0L }.distinct().sorted()
        if (sorted.size < 2) return null
        val gaps = sorted.zipWithNext { earlier, later -> later - earlier }.filter { it > 0L }.sorted()
        if (gaps.isEmpty()) return null
        return gaps[gaps.size / 2]
    }

    fun intervalLabel(timestamps: List<Long>, targetMs: Long): String {
        val median = medianIntervalMs(timestamps) ?: return "Recent block interval: not enough verified headers"
        val tenths = (median + 50L) / 100L
        val whole = tenths / 10L
        val frac = tenths % 10L
        val target = if (targetMs > 0L && targetMs % 1000L == 0L) "${targetMs / 1000L}s" else "${targetMs} ms"
        return "Recent block interval: $whole.${frac}s (target $target)"
    }

    fun intervalTone(timestamps: List<Long>, targetMs: Long): String {
        val median = medianIntervalMs(timestamps) ?: return "idle"
        val delta = kotlin.math.abs(median - targetMs)
        return if (delta <= 500L) "ok" else "drift"
    }

    private fun argsSummary(args: String): String {
        val trimmed = args.trim()
        if (trimmed.isEmpty()) return "none"
        return if (trimmed.length <= 80) trimmed else trimmed.take(80) + "…"
    }

    private fun kvnc(atoms: String): String {
        val value = atoms.toBigIntegerOrNull() ?: return "$atoms atoms"
        return "${Amounts.format(value, 9)} KVNC ($atoms atoms)"
    }
}
