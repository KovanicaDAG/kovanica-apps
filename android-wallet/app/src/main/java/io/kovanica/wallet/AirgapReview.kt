package io.kovanica.wallet

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
            rows += "Method: ${doc.optString("method").ifBlank { "—" }}"
            rows += "Arguments: ${argsSummary(doc.optString("args"))}"
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
        return rounds.filter { it <= tip && tip - it < window }.toSet() + tip
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
