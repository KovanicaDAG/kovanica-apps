package io.kovanica.wallet

import android.app.Activity
import android.graphics.Bitmap
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.os.Bundle
import android.text.InputType
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import org.json.JSONObject
import uniffi.kovanica_light.signCallHd
import uniffi.kovanica_light.signTransferHd
import java.util.concurrent.Executors

/**
 * Offline signer for a console air-gap QR. The payload is the unsigned preview.
 * This screen signs it with the phone's HD key and shows a QR of the signed
 * payload. It does not talk to a node and it does not broadcast.
 */
class AirgapSignActivity : Activity() {
    private val io = Executors.newSingleThreadExecutor()
    private lateinit var status: TextView
    private lateinit var qr: ImageView
    private lateinit var signedText: TextView
    private lateinit var passField: EditText
    private lateinit var bip39Field: EditText
    private lateinit var accountField: EditText

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        actionBar?.hide()
        window.setBackgroundDrawable(ColorDrawable(WalletUi.INK))
        window.statusBarColor = WalletUi.INK

        val payload = intent.getStringExtra(EXTRA_PAYLOAD).orEmpty()
        val doc = runCatching { JSONObject(payload) }.getOrNull()
        status = TextView(this).apply {
            setTextColor(WalletUi.TEXT)
            textSize = 14f
        }
        qr = ImageView(this)
        signedText = TextView(this).apply {
            setTextColor(WalletUi.MUTED)
            textSize = 12f
            typeface = Typeface.MONOSPACE
        }
        passField = field("Wallet password")
        bip39Field = field("BIP-39 passphrase, if you use one")
        accountField = field("Account index").apply {
            setText(hdAccountIndex().toString())
            inputType = InputType.TYPE_CLASS_NUMBER
        }

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(24), dp(20), dp(36))
            setBackgroundColor(WalletUi.INK)
        }
        root.addView(title())
        root.addView(status, gap())
        if (doc == null || doc.optInt("v") != 1 || doc.optString("role") != "unsigned") {
            status.text = "That code is not an unsigned Kovanica transaction."
        } else {
            status.text = summary(doc)
            root.addView(passField, gap())
            root.addView(bip39Field, gap())
            root.addView(accountField, gap())
            root.addView(WalletUi.button(this, "Sign offline") { sign(doc) }, gap())
        }
        root.addView(qr, gap())
        root.addView(signedText, gap())
        setContentView(ScrollView(this).apply { addView(root) })
    }

    private fun sign(doc: JSONObject) {
        val password = passField.text.toString()
        if (password.isEmpty()) {
            status.text = "Enter the wallet password. The phrase stays on this phone."
            return
        }
        if (!isHd()) {
            status.text = "Air-gap signing uses the HD key. This phone still has a legacy wallet."
            return
        }
        val keystore = getSharedPreferences(PREFS, MODE_PRIVATE).getString("keystore", null)
        if (keystore.isNullOrBlank()) {
            status.text = "Create or import a wallet on this phone first."
            return
        }
        status.text = "Signing…"
        val account = accountField.text.toString().trim().toUIntOrNull() ?: hdAccountIndex()
        val bip39 = bip39Field.text.toString()
        io.execute {
            try {
                val raw = if (doc.optString("kind") == "call") {
                    signCallHd(
                        keystore,
                        password,
                        bip39,
                        account,
                        doc.getString("to"),
                        doc.optString("method"),
                        doc.optString("args"),
                        doc.optString("value", "0").toULong(),
                        doc.optString("gas", "0").toULong(),
                        doc.getString("fee").toULong(),
                        doc.getString("nonce").toULong(),
                        doc.getLong("chainId").toULong(),
                    )
                } else {
                    signTransferHd(
                        keystore,
                        password,
                        bip39,
                        account,
                        doc.getString("to"),
                        doc.getString("amount").toULong(),
                        doc.getString("fee").toULong(),
                        doc.getString("nonce").toULong(),
                        doc.getLong("chainId").toULong(),
                    )
                }
                doc.put("role", "signed")
                doc.put("raw", raw.removePrefix("0x"))
                val signed = doc.toString()
                runOnUiThread {
                    status.text = "Signed. Scan this code back on the computer. Nothing was broadcast."
                    signedText.text = signed
                    qr.setImageBitmap(runCatching { qrBitmap(signed, dp(280)) }.getOrNull())
                }
            } catch (error: Exception) {
                runOnUiThread { status.text = error.message ?: "The phone could not sign that payload." }
            }
        }
    }

    private fun summary(doc: JSONObject): String {
        val kind = if (doc.optString("kind") == "call") "Contract call, wire 6" else "Transfer, wire 0"
        return "$kind\nFrom: ${doc.optString("from")}\nTo: ${doc.optString("to")}\nAmount: ${doc.optString("amount")} atoms\nFee: ${doc.optString("fee")} atoms\nChain ${doc.optLong("chainId")}"
    }

    private fun isHd(): Boolean = getSharedPreferences(PREFS, MODE_PRIVATE).getBoolean("is_hd", false)

    private fun hdAccountIndex(): UInt =
        getSharedPreferences(PREFS, MODE_PRIVATE).getInt("hd_account_index", 0).coerceAtLeast(0).toUInt()

    private fun title(): TextView = TextView(this).apply {
        text = "Air-gap sign"
        setTextColor(WalletUi.TEXT)
        textSize = 28f
        typeface = Typeface.DEFAULT_BOLD
    }

    private fun field(hint: String): EditText = EditText(this).apply {
        this.hint = hint
        setHintTextColor(WalletUi.MUTED)
        setTextColor(WalletUi.TEXT)
        setBackgroundColor(WalletUi.SURFACE)
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        setPadding(dp(14), dp(12), dp(14), dp(12))
    }

    private fun gap() = LinearLayout.LayoutParams(
        LinearLayout.LayoutParams.MATCH_PARENT,
        LinearLayout.LayoutParams.WRAP_CONTENT,
    ).apply { topMargin = dp(14) }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt().coerceAtLeast(1)

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

    companion object {
        const val EXTRA_PAYLOAD: String = "io.kovanica.wallet.AIRGAP_PAYLOAD"
        private const val PREFS: String = "kvnc"
    }
}
