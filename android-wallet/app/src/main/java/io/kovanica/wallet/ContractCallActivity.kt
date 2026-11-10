package io.kovanica.wallet

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import io.kovanica.chain.ChainIds
import org.json.JSONArray
import org.json.JSONObject
import uniffi.kovanica_light.signCallHd
import uniffi.kovanica_light.signCallRaw
import uniffi.kovanica_light.walletAddress
import java.math.BigInteger
import java.util.concurrent.Executors
import kotlin.math.roundToInt

/**
 * Review and sign a canonical WASM contract call.
 *
 * The form matches the console: Token, HTLC, Vault, and Multisig. The seed
 * stays inside the Rust signer. This screen only passes the method, the args
 * hex, and the fee quote.
 */
class ContractCallActivity : Activity() {
    private val io = Executors.newSingleThreadExecutor()
    private var gone = false

    private lateinit var status: TextView
    private lateinit var review: TextView
    private lateinit var receipt: TextView
    private lateinit var contractField: EditText
    private lateinit var partyField: EditText
    private lateinit var amountField: EditText
    private lateinit var hashField: EditText
    private lateinit var timeField: EditText
    private lateinit var secretField: EditText
    private lateinit var recordField: EditText
    private lateinit var proposalField: EditText
    private lateinit var dataField: EditText
    private lateinit var passField: EditText
    private lateinit var bip39Field: EditText
    private lateinit var accountField: EditText
    private lateinit var actionRow: LinearLayout
    private lateinit var partyBox: View
    private lateinit var amountBox: View
    private lateinit var hashBox: View
    private lateinit var timeBox: View
    private lateinit var secretBox: View
    private lateinit var recordBox: View
    private lateinit var proposalBox: View
    private lateinit var dataBox: View
    private lateinit var funds: TextView
    private lateinit var faucetButton: Button
    private lateinit var exportButton: Button
    private lateinit var importButton: Button
    private lateinit var qrView: ImageView

    private var family = "token"
    private var action = "token_transfer"
    private var ready: ContractCalls.Built? = null
    private var quoted: Quoted? = null
    private var pendingExport: String? = null

    private data class Quoted(
        val sender: String,
        val canonical: String,
        val nonce: ULong,
        val fee: ULong,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        actionBar?.hide()
        window.setBackgroundDrawable(ColorDrawable(WalletUi.INK))
        window.statusBarColor = WalletUi.INK

        status = note()
        review = note()
        receipt = note()
        contractField = WalletUi.field(this, "Contract address", mono = true)
        partyField = WalletUi.field(this, "Address", mono = true)
        amountField = WalletUi.field(this, "Amount (atoms)", mono = true).apply {
            inputType = InputType.TYPE_CLASS_NUMBER
        }
        hashField = WalletUi.field(this, "Hash (32-byte hex)", mono = true)
        timeField = WalletUi.field(this, "Time (unix ms)", mono = true).apply {
            inputType = InputType.TYPE_CLASS_NUMBER
        }
        secretField = WalletUi.field(this, "Secret (hex or text)", mono = true)
        recordField = WalletUi.field(this, "Record id (32-byte hex)", mono = true)
        proposalField = WalletUi.field(this, "Proposal id", mono = true).apply {
            inputType = InputType.TYPE_CLASS_NUMBER
        }
        dataField = WalletUi.field(this, "Call data hex, optional", mono = true)
        passField = WalletUi.field(this, "Wallet password").apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        }
        bip39Field = WalletUi.field(this, "BIP-39 passphrase, if you use one").apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        }
        accountField = WalletUi.field(this, "HD account index", hdIndex().toString(), mono = true).apply {
            inputType = InputType.TYPE_CLASS_NUMBER
        }
        actionRow = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        funds = note().apply { visibility = View.GONE }
        faucetButton = WalletUi.button(this, "Request Testnet Faucet", filled = false) { requestFaucet() }.apply {
            visibility = View.GONE
        }
        exportButton = WalletUi.button(this, "Export unsigned call", filled = false) { exportUnsigned() }
        importButton = WalletUi.button(this, "Import signed call", filled = false) { importSigned() }
        qrView = ImageView(this).apply {
            visibility = View.GONE
            adjustViewBounds = true
        }

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(24), dp(20), dp(36))
            setBackgroundColor(WalletUi.INK)
        }
        root.addView(header())
        root.addStack(status, 18)
        root.addStack(familyCard(), 18)
        root.addStack(formCard(), 18)
        root.addStack(WalletUi.button(this, "Review call") { onReview() }, 18)
        root.addStack(review, 12)
        root.addStack(funds, 8)
        root.addStack(faucetButton, 8)
        root.addStack(exportButton, 12)
        root.addStack(qrView, 8)
        root.addStack(importButton, 8)
        root.addStack(WalletUi.button(this, "Confirm & sign") { onSign() }, 12)
        root.addStack(receipt, 12)
        root.addStack(WalletUi.button(this, "Back to wallet", filled = false) { finish() }, 10)
        setContentView(ScrollView(this).apply {
            isFillViewport = true
            setBackgroundColor(WalletUi.INK)
            addView(root)
        })
        paintActions()
        setStatus("Pick a contract, review the decoded call, then sign. The seed stays on this phone.", Tone.NEUTRAL)
    }

    override fun onStop() {
        if (::passField.isInitialized) passField.setText("")
        super.onStop()
    }

    override fun onDestroy() {
        gone = true
        io.shutdownNow()
        super.onDestroy()
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == SAVE_JSON) {
            if (resultCode == RESULT_OK) writeUnsigned(data?.data)
            return
        }
        if (requestCode != OPEN_JSON || resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        val text = runCatching {
            contentResolver.openInputStream(uri)?.bufferedReader()?.use { it.readText() }
        }.getOrNull()
        if (text.isNullOrBlank() || text.length > 256_000) {
            setStatus("That file is not a readable signed call.", Tone.ERROR)
            return
        }
        broadcastSigned(text)
    }

    private fun familyCard(): LinearLayout = WalletUi.card(this).apply {
        addStack(WalletUi.sectionLabel(this@ContractCallActivity, "Contract"))
        val row = LinearLayout(this@ContractCallActivity).apply { orientation = LinearLayout.HORIZONTAL }
        listOf("token", "htlc", "vault", "multisig").forEach { name ->
            row.addView(
                WalletUi.button(this@ContractCallActivity, name, filled = false) {
                    family = name
                    action = ContractCalls.actions(name).first().first
                    ready = null
                    review.text = ""
                    clearQuote()
                    paintActions()
                },
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(6) },
            )
        }
        addStack(row, 4)
        addStack(actionRow, 8)
    }

    private fun formCard(): LinearLayout = WalletUi.card(this).apply {
        addStack(WalletUi.sectionLabel(this@ContractCallActivity, "Call"))
        addStack(contractField, 4)
        partyBox = wrap(partyField)
        amountBox = wrap(amountField)
        hashBox = wrap(hashField)
        timeBox = wrap(timeField)
        secretBox = wrap(secretField)
        recordBox = wrap(recordField)
        proposalBox = wrap(proposalField)
        dataBox = wrap(dataField)
        listOf(partyBox, amountBox, hashBox, timeBox, secretBox, recordBox, proposalBox, dataBox).forEach { addStack(it, 8) }
        addStack(passField, 12)
        addStack(bip39Field, 8)
        addStack(accountField, 8)
        addStack(
            WalletUi.caption(
                this@ContractCallActivity,
                "Amounts are whole atoms. HTLC create and vault deposit pull that amount from your balance. Withdraw claims the vested balance and takes no amount.",
            ),
            12,
        )
    }

    private fun paintActions() {
        actionRow.removeAllViews()
        ContractCalls.actions(family).forEach { (id, label) ->
            actionRow.addView(
                WalletUi.button(this, label, filled = id == action) {
                    action = id
                    ready = null
                    review.text = ""
                    clearQuote()
                    paintActions()
                },
                LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
                    topMargin = dp(8)
                },
            )
        }
        val shown = ContractCalls.fields(action)
        partyBox.visibility = if ("party" in shown) View.VISIBLE else View.GONE
        amountBox.visibility = if ("amount" in shown) View.VISIBLE else View.GONE
        hashBox.visibility = if ("hash" in shown) View.VISIBLE else View.GONE
        timeBox.visibility = if ("time" in shown) View.VISIBLE else View.GONE
        secretBox.visibility = if ("secret" in shown) View.VISIBLE else View.GONE
        recordBox.visibility = if ("record" in shown) View.VISIBLE else View.GONE
        proposalBox.visibility = if ("proposal" in shown) View.VISIBLE else View.GONE
        dataBox.visibility = if ("data" in shown) View.VISIBLE else View.GONE
        partyField.hint = when (action) {
            "htlc_create" -> "Beneficiary (claimer)"
            "vault_create" -> "Beneficiary"
            "token_mint", "balance_of" -> "Account"
            else -> "Recipient"
        }
        timeField.hint = if (action == "vault_create") "Lock until (unix ms)" else "Timelock (unix ms)"
        recordField.hint = when (family) {
            "htlc" -> "Swap id (32-byte hex)"
            "vault" -> "Vault id (32-byte hex)"
            else -> "Multisig id (32-byte hex)"
        }
    }

    private fun onReview() {
        if (action == "balance_of") {
            readBalance()
            return
        }
        val chain = NetworkPrefs.chainId(this)
        if (chain == ChainIds.MAINNET) {
            setStatus("Mainnet is read-only. Contract calls stay off.", Tone.ERROR)
            return
        }
        try {
            val built = ContractCalls.build(draft(chain))
            ready = built
            quoted = null
            review.text = built.lines.joinToString("\n") + "\nMethod wire: ${built.method}\nArgs: ${built.argsHex}"
            setStatus("Review the decoded call, then confirm.", Tone.OK)
            quoteFunds(built)
        } catch (error: Exception) {
            ready = null
            review.text = ""
            setStatus(error.message ?: "That call is not valid.", Tone.ERROR)
        }
    }

    private fun onSign() {
        val built = ready
        if (built == null) {
            setStatus("Review the call before signing.", Tone.WARN)
            return
        }
        if (NetworkPrefs.chainId(this) == ChainIds.MAINNET) {
            setStatus("Mainnet is read-only. Contract calls stay off.", Tone.ERROR)
            return
        }
        if (passField.text.isNullOrEmpty()) {
            setStatus("Enter the wallet password. The phrase stays on this phone.", Tone.ERROR)
            return
        }
        AlertDialog.Builder(this)
            .setTitle("Confirm contract call")
            .setMessage(built.lines.joinToString("\n"))
            .setNegativeButton("Back", null)
            .setPositiveButton("Confirm & sign") { _, _ -> sign(built) }
            .show()
    }

    private fun sign(built: ContractCalls.Built) {
        val password = passField.text.toString()
        val bip39 = bip39Field.text.toString()
        val account = accountField.text.toString().trim().toUIntOrNull() ?: hdIndex()
        val contract = contractField.text.toString()
        val keystore = keystoreOrNull()
        if (keystore.isNullOrBlank()) {
            setStatus("Create or import a wallet on this phone first.", Tone.ERROR)
            return
        }
        val chain = NetworkPrefs.chainId(this)
        val rpc = NetworkPrefs.rpc(this)
        val networkId = NetworkPrefs.selected(this).id
        val amountText = amountField.text.toString().trim().ifEmpty { null }
        setStatus("Quoting the fee…", Tone.NEUTRAL)
        io.execute {
            try {
                val node = NodeClient(rpc)
                val sender = senderAddress(keystore)
                val nonce = parseQuantity(node.call("kovanica_getNonce", JSONArray().put(sender)))
                val rate = parseQuantity(node.call("kovanica_estimateFee", JSONArray()))
                val canonical = KvncAddress.canonical(KvncAddress.normalizeHex(contract, chain) ?: contract)
                    ?: error("That isn't a valid contract address.")
                val raw = if (isHd()) {
                    val fee = (rate * 250uL).coerceAtLeast(1uL)
                    signCallHd(
                        keystore,
                        password,
                        bip39,
                        account,
                        canonical,
                        built.method,
                        built.argsHex,
                        0uL,
                        ContractCalls.GAS_LIMIT,
                        fee,
                        nonce,
                        chain.toULong(),
                    )
                } else {
                    signCallRaw(
                        keystore,
                        password,
                        chain.toULong(),
                        canonical,
                        0uL,
                        built.method,
                        built.argsHex,
                        ContractCalls.GAS_LIMIT,
                        nonce,
                        rate,
                    )
                }
                submitAndWatch(node, raw, networkId, canonical, built.method, amountText)
            } catch (_: InterruptedException) {
                return@execute
            } catch (error: Exception) {
                post { setStatus(error.message ?: "The phone could not sign that call.", Tone.ERROR) }
            }
        }
    }

    private fun submitAndWatch(
        node: NodeClient,
        raw: String,
        networkId: String,
        contract: String,
        method: String,
        amount: String?,
    ) {
        val submitted = node.call("kovanica_sendRawTransaction", JSONArray().put(raw))
        val hash = submitted?.toString()?.removePrefix("0x").orEmpty()
        if (hash.isNotEmpty()) {
            HistoryStore(this).add(
                networkId,
                HistoryEntry(
                    kind = HistoryEntry.KIND_CONTRACT,
                    hash = hash,
                    networkId = networkId,
                    timestampMs = System.currentTimeMillis(),
                    contract = contract,
                    amount = amount,
                    symbol = "KVNC",
                    method = method,
                ),
            )
        }
        post { setStatus("Submitted. Waiting for the receipt.", Tone.OK) }
        var shown = ContractCalls.ReceiptView("pending", "", "", "", hash, "", emptyList())
        repeat(5) {
            if (gone) return
            val body = runCatching { node.call("kovanica_getTransactionReceipt", JSONArray().put(hash)) }.getOrNull()
            shown = ContractCalls.decodeReceipt(body)
            if (shown.hash.isBlank()) shown = shown.copy(hash = hash)
            val text = ContractCalls.formatReceipt(shown)
            post { receipt.text = text }
            if (shown.status != "pending") {
                if (hash.isNotEmpty()) {
                    HistoryStore(this).update(networkId, hash) { entry ->
                        entry.copy(
                            receiptCode = shown.code.ifBlank { null },
                            gasUsed = shown.gasUsed.ifBlank { null },
                            errorText = shown.error.ifBlank { null },
                            events = shown.logs.take(4).joinToString(", ") { log -> log.topics.ifBlank { "event" } }.ifBlank { null },
                            status = shown.status,
                        )
                    }
                }
                return
            }
            Thread.sleep(2_000)
        }
    }

    private fun quoteFunds(built: ContractCalls.Built) {
        val chain = NetworkPrefs.chainId(this)
        val keystore = keystoreOrNull()
        if (keystore.isNullOrBlank()) {
            setStatus("Review looks valid. Create or import a wallet to quote the fee.", Tone.WARN)
            return
        }
        val amountAtoms = amountField.text.toString()
        val contract = contractField.text.toString()
        val rpc = NetworkPrefs.rpc(this)
        io.execute {
            try {
                val node = NodeClient(rpc)
                val sender = senderAddress(keystore)
                val nonce = parseQuantity(node.call("kovanica_getNonce", JSONArray().put(sender)))
                val rate = parseQuantity(node.call("kovanica_estimateFee", JSONArray()))
                val fee = if (isHd()) (rate * 250uL).coerceAtLeast(1uL) else rate.coerceAtLeast(1uL)
                val canonical = KvncAddress.canonical(KvncAddress.normalizeHex(contract, chain) ?: contract)
                    ?: error("That isn't a valid contract address.")
                val balance = BigInteger(node.balance(sender).toString())
                val missing = ContractCalls.shortfall(balance, ContractCalls.callNeed(built.method, "0", fee.toString(), amountAtoms))
                val snap = Quoted(sender, canonical, nonce, fee)
                post {
                    if (ready?.argsHex != built.argsHex || ready?.method != built.method) return@post
                    quoted = snap
                    if (missing > BigInteger.ZERO) {
                        funds.text = "Balance is short by $missing atoms for this call value and fee."
                        funds.setTextColor(WalletUi.BAD)
                        funds.background = WalletUi.rounded(this, WalletUi.BAD_BG, 14, WalletUi.LINE, 1)
                        funds.setPadding(dp(14), dp(12), dp(14), dp(12))
                        funds.visibility = View.VISIBLE
                        faucetButton.visibility = if (ContractCalls.faucetQuick(chain)) View.VISIBLE else View.GONE
                    } else {
                        funds.visibility = View.GONE
                        faucetButton.visibility = View.GONE
                    }
                }
            } catch (_: InterruptedException) {
                return@execute
            } catch (error: Exception) {
                post { setStatus(error.message ?: "The fee quote failed.", Tone.WARN) }
            }
        }
    }

    private fun requestFaucet() {
        val chain = NetworkPrefs.chainId(this)
        if (!ContractCalls.faucetQuick(chain)) {
            setStatus("The inline faucet is only on chain 5 and chain 1337.", Tone.WARN)
            return
        }
        val left = FaucetClaim.remaining(System.currentTimeMillis(), prefs().getLong("faucet_until", 0L))
        if (left > 0L) {
            setStatus("Claim again in ${left / 1000}s.", Tone.WARN)
            return
        }
        val keystore = keystoreOrNull()
        if (keystore.isNullOrBlank()) {
            setStatus("Create or import a wallet on this phone first.", Tone.ERROR)
            return
        }
        val rpc = NetworkPrefs.rpc(this)
        val built = ready
        setStatus("Requesting testnet KVNC…", Tone.NEUTRAL)
        io.execute {
            try {
                val node = NodeClient(rpc)
                val sender = senderAddress(keystore)
                val result = node.call("kovanica_requestAirdrop", JSONArray().put(sender).put(FaucetClaim.ATOMS))
                prefs().edit().putLong("faucet_until", System.currentTimeMillis() + FaucetClaim.COOLDOWN_MS).apply()
                val hash = HistoryStore.extractTxHash(result?.toString() ?: "")
                post {
                    setStatus(
                        if (hash != null) "Faucet claim sent. TxHash: $hash" else "Faucet answered.",
                        Tone.OK,
                    )
                }
                if (built != null) quoteFunds(built)
            } catch (_: InterruptedException) {
                return@execute
            } catch (error: Exception) {
                post { setStatus(error.message ?: "The faucet did not respond.", Tone.ERROR) }
            }
        }
    }

    private fun exportUnsigned() {
        val built = ready
        val quote = quoted
        if (built == null || quote == null) {
            setStatus("Review the call and wait for the fee quote before exporting.", Tone.WARN)
            return
        }
        val chain = NetworkPrefs.chainId(this)
        val body = JSONObject()
            .put("v", 1)
            .put("role", "unsigned")
            .put("kind", "call")
            .put("wire", 6)
            .put("tag", 4)
            .put("chainId", chain)
            .put("networkName", "chain $chain")
            .put("from", quote.sender)
            .put("to", quote.canonical)
            .put("amount", "0")
            .put("fee", quote.fee.toString())
            .put("nonce", quote.nonce.toString())
            .put("method", built.method)
            .put("args", built.argsHex)
            .put("value", "0")
            .put("gas", ContractCalls.GAS_LIMIT.toString())
            .toString()
        pendingExport = body
        val bitmap = if (body.length <= 1_200) runCatching { qrBitmap(body, dp(220)) }.getOrNull() else null
        if (bitmap != null) {
            qrView.setImageBitmap(bitmap)
            qrView.visibility = View.VISIBLE
            setStatus("Unsigned call is in the QR. Save the JSON to move it to the cold phone.", Tone.OK)
        } else {
            qrView.setImageDrawable(null)
            qrView.visibility = View.GONE
            setStatus("This call is too large for a QR. Save the JSON file.", Tone.WARN)
        }
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/json"
            putExtra(Intent.EXTRA_TITLE, "kovanica-call-unsigned.json")
        }
        startActivityForResult(intent, SAVE_JSON)
    }

    private fun writeUnsigned(uri: android.net.Uri?) {
        val body = pendingExport
        if (uri == null || body.isNullOrBlank()) {
            setStatus("The unsigned file was not saved.", Tone.ERROR)
            return
        }
        val error = runCatching {
            contentResolver.openOutputStream(uri)?.use { stream ->
                stream.write(body.toByteArray(Charsets.UTF_8))
            } ?: error("The downloads folder did not accept the file.")
        }.exceptionOrNull()
        setStatus(error?.message ?: "Saved kovanica-call-unsigned.json. Nothing was broadcast.", if (error == null) Tone.OK else Tone.ERROR)
    }

    private fun importSigned() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("application/json", "text/plain", "application/octet-stream"))
        }
        startActivityForResult(intent, OPEN_JSON)
    }

    private fun broadcastSigned(text: String) {
        val doc = runCatching { JSONObject(text) }.getOrNull()
        if (doc == null || doc.optString("role") != "signed" || doc.optString("raw").isBlank()) {
            setStatus("That file is not a signed contract call.", Tone.ERROR)
            return
        }
        if (doc.optString("kind") != "call") {
            setStatus("That signature is not a contract call.", Tone.ERROR)
            return
        }
        val chain = NetworkPrefs.chainId(this)
        if (doc.has("chainId") && doc.getLong("chainId") != chain) {
            setStatus("This signature is for chain ${doc.getLong("chainId")}. This phone is chain $chain.", Tone.ERROR)
            return
        }
        val raw = doc.getString("raw")
        val method = doc.optString("method")
        val contract = doc.optString("to")
        val rpc = NetworkPrefs.rpc(this)
        val networkId = NetworkPrefs.selected(this).id
        setStatus("Broadcasting the signed call…", Tone.NEUTRAL)
        io.execute {
            try {
                submitAndWatch(NodeClient(rpc), raw, networkId, contract, method, null)
            } catch (_: InterruptedException) {
                return@execute
            } catch (error: Exception) {
                post { setStatus(error.message ?: "The phone could not broadcast that call.", Tone.ERROR) }
            }
        }
    }

    private fun clearQuote() {
        quoted = null
        pendingExport = null
        if (::funds.isInitialized) {
            funds.visibility = View.GONE
            faucetButton.visibility = View.GONE
            qrView.visibility = View.GONE
            qrView.setImageDrawable(null)
        }
    }

    private fun senderAddress(keystore: String): String {
        if (isHd()) {
            val active = prefs().getString("hd_active_address", null)
            if (!active.isNullOrBlank()) return active
        }
        return walletAddress(keystore)
    }

    private fun qrBitmap(contents: String, sizePx: Int): Bitmap {
        val hints = mapOf(
            EncodeHintType.CHARACTER_SET to "UTF-8",
            EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L,
            EncodeHintType.MARGIN to 1,
        )
        val matrix = QRCodeWriter().encode(contents, BarcodeFormat.QR_CODE, sizePx, sizePx, hints)
        val pixels = IntArray(sizePx * sizePx)
        for (y in 0 until sizePx) {
            for (x in 0 until sizePx) pixels[y * sizePx + x] = if (matrix.get(x, y)) 0xFF0C1412.toInt() else 0xFFF4F0E6.toInt()
        }
        return Bitmap.createBitmap(pixels, sizePx, sizePx, Bitmap.Config.ARGB_8888)
    }

    private fun readBalance() {
        val chain = NetworkPrefs.chainId(this)
        val contractHex = KvncAddress.normalizeHex(contractField.text.toString(), chain)
        val ownerHex = KvncAddress.normalizeHex(partyField.text.toString(), chain)
        if (contractHex == null || ownerHex == null) {
            setStatus("Enter a contract address and an owner address.", Tone.ERROR)
            return
        }
        val rpc = NetworkPrefs.rpc(this)
        setStatus("Reading token balance…", Tone.NEUTRAL)
        io.execute {
            val outcome = TokenClient(NodeClient(rpc)).read(contractHex)
            post {
                when (outcome) {
                    is TokenRead.Ok -> {
                        val amount = outcome.info.balanceOf(ownerHex)
                        review.text = "balance_of ${KvncAddress.display(ownerHex)}\n${amount} base units\n${outcome.info.symbol} · decimals ${outcome.info.decimals}"
                        setStatus("Balance read from contract storage. Nothing was signed.", Tone.OK)
                    }
                    TokenRead.NotAToken -> setStatus("That address has no token state.", Tone.WARN)
                    TokenRead.Unreadable -> setStatus("Storage at that address is not a token.", Tone.ERROR)
                    is TokenRead.Failed -> setStatus(outcome.message, Tone.ERROR)
                }
            }
        }
    }

    private fun draft(chain: Long) = ContractCalls.Draft(
        family = family,
        action = action,
        chainId = chain,
        party = partyField.text.toString(),
        hash = hashField.text.toString(),
        secret = secretField.text.toString(),
        recordId = recordField.text.toString(),
        timeMs = timeField.text.toString(),
        amount = amountField.text.toString(),
        proposalId = proposalField.text.toString(),
        callData = dataField.text.toString(),
    )

    private fun post(block: () -> Unit) {
        if (!gone) runOnUiThread { if (!gone) block() }
    }

    private fun prefs() = getSharedPreferences(NetworkPrefs.PREFS, MODE_PRIVATE)

    private fun keystoreOrNull(): String? = prefs().getString("keystore", null)

    private fun isHd(): Boolean = prefs().getBoolean("is_hd", false)

    private fun hdIndex(): UInt = prefs().getInt("hd_account_index", 0).coerceAtLeast(0).toUInt()

    private fun note(): TextView = TextView(this).apply {
        setTextColor(WalletUi.TEXT)
        textSize = 14f
        setLineSpacing(0f, 1.3f)
    }

    private fun wrap(child: View): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        addView(child)
    }

    private fun header(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        addView(TextView(this@ContractCallActivity).apply {
            text = "KVNC · CONTRACTS"
            setTextColor(WalletUi.ACCENT)
            textSize = 11f
            letterSpacing = 0.22f
            typeface = Typeface.DEFAULT_BOLD
        })
        addView(TextView(this@ContractCallActivity).apply {
            text = "Contract call"
            setTextColor(WalletUi.TEXT)
            textSize = 30f
            typeface = Typeface.DEFAULT_BOLD
            setPadding(0, dp(6), 0, 0)
        })
    }

    private enum class Tone { NEUTRAL, OK, WARN, ERROR }

    private fun setStatus(text: String, tone: Tone) {
        status.text = text
        val (foreground, background) = when (tone) {
            Tone.NEUTRAL -> WalletUi.MUTED to WalletUi.SURFACE
            Tone.OK -> WalletUi.GOOD to WalletUi.GOOD_BG
            Tone.WARN -> WalletUi.ACCENT to WalletUi.ACCENT_BG
            Tone.ERROR -> WalletUi.BAD to WalletUi.BAD_BG
        }
        status.setTextColor(foreground)
        status.background = WalletUi.rounded(this, background, 14, WalletUi.LINE, 1)
        status.setPadding(dp(14), dp(12), dp(14), dp(12))
    }

    private fun ViewGroup.addStack(child: View, top: Int = 0) {
        addView(
            child,
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
                topMargin = dp(top)
            },
        )
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).roundToInt().coerceAtLeast(1)

    companion object {
        private const val SAVE_JSON = 61
        private const val OPEN_JSON = 62
    }
}
