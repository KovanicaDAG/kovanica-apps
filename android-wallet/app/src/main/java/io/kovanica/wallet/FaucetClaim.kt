package io.kovanica.wallet

/** Testnet airdrop rules and the seed quiz. Mainnet is never allowed. */
object FaucetClaim {
    const val ATOMS: String = "100000000000"
    const val COOLDOWN_MS: Long = 60_000L

    fun allowed(chainId: Long): Boolean = chainId == 3L || chainId == 5L || chainId == 1337L

    fun remaining(now: Long, until: Long): Long = (until - now).coerceAtLeast(0L)
}

object SeedCheck {
    fun positions(wordCount: Int): List<Int> = if (wordCount >= 14) listOf(3, 8, 14) else listOf(3, 8, 12)

    fun matches(mnemonic: String, answers: List<String>): Boolean {
        val words = mnemonic.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
        val spots = positions(words.size)
        if (answers.size != spots.size) return false
        return spots.zip(answers).all { (position, answer) ->
            words.getOrNull(position - 1)?.equals(answer.trim(), ignoreCase = true) == true
        }
    }
}
