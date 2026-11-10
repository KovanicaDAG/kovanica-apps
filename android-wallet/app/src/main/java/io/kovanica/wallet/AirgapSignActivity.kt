package io.kovanica.wallet

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.text.InputType
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
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
    private lateinit var clearButton: Button
    private lateinit var exportButton: Button
    private lateinit var form: LinearLayout
    private var active: JSONObject? = null
    private val hideSigned = Runnable { clearSigned("Signed QR hidden. Nothing was broadcast.") }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        actionBar?.hide()
        window.setBackgroundDrawable(ColorDrawable(WalletUi.INK))
        window.statusBarColor = WalletUi.INK

        val payload = intent.getStringExtra(EXTRA_PAYLOAD).orEmpty()
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
        clearButton = WalletUi.button(this, "Clear signed QR", filled = false) { clearSigned("Signed QR cleared. Nothing was broadcast.") }
        clearButton.visibility = View.GONE
        exportButton = WalletUi.button(this, "Save signed JSON", filled = false) { exportSigned() }
        exportButton.visibility = View.GONE
        form = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            addView(passField, gap())
            addView(bip39Field, gap())
            addView(accountField, gap())
            addView(WalletUi.button(this@AirgapSignActivity, "Review & sign") { review() }, gap())
        }

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(24), dp(20), dp(36))
            setBackgroundColor(WalletUi.INK)
        }
        root.addView(title())
        root.addView(status, gap())
        root.addView(WalletUi.button(this, "Load unsigned JSON", filled = false) { pickUnsigned() }, gap())
        root.addView(form, gap())
        root.addView(qr, gap())
        root.addView(signedText, gap())
        root.addView(clearButton, gap())
        root.addView(exportButton, gap())
        setContentView(ScrollView(this).apply { addView(root) })
        present(payload, announce = payload.isNotBlank())
    }

    override fun onDestroy() {
        qr.removeCallbacks(hideSigned)
        io.shutdown()
        super.onDestroy()
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == SAVE_JSON) {
            if (resultCode == RESULT_OK) writeSigned(data?.data)
            if (exportButton.visibility == View.VISIBLE) qr.postDelayed(hideSigned, 60_000)
            return
        }
        if (requestCode != OPEN_JSON || resultCode != RESULT_OK) return
        val uri = data?.data ?: return
        val text = runCatching {
            contentResolver.openInputStream(uri)?.bufferedReader()?.use { it.readText() }
        }.getOrNull()
        if (text.isNullOrBlank() || text.length > 256_000) {
            active = null
            form.visibility = View.GONE
            status.text = "That file is not a readable unsigned transaction."
            return
        }
        present(text, announce = true)
    }

    private fun exportSigned() {
        val nonce = active?.optString("nonce").orEmpty().filter { it.isDigit() }.ifEmpty { "0" }
        val body = signedText.text?.toString().orEmpty()
        if (body.isBlank()) {
            status.text = "Sign the transaction before saving it."
            return
        }
        qr.removeCallbacks(hideSigned)
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/json"
            putExtra(Intent.EXTRA_TITLE, "kovanica-tx-signed-$nonce.json")
        }
        startActivityForResult(intent, SAVE_JSON)
    }

    private fun writeSigned(uri: android.net.Uri?) {
        val body = signedText.text?.toString().orEmpty()
        if (uri == null || body.isBlank()) {
            status.text = "The signed file was not saved."
            return
        }
        val error = runCatching {
            contentResolver.openOutputStream(uri)?.use { stream ->
                stream.write(body.toByteArray(Charsets.UTF_8))
            } ?: error("The downloads folder did not accept the file.")
        }.exceptionOrNull()
        status.text = error?.message ?: "Saved kovanica-tx-signed.json. Nothing was broadcast."
    }

    private fun pickUnsigned() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
            putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("application/json", "text/plain", "application/octet-stream"))
        }
        startActivityForResult(intent, OPEN_JSON)
    }

    private fun present(payload: String, announce: Boolean) {
        val doc = runCatching { JSONObject(payload) }.getOrNull()
        active = null
        form.visibility = View.GONE
        when {
            doc == null || doc.optInt("v") != 1 || doc.optString("role") != "unsigned" || !doc.has("chainId") -> {
                status.text = if (payload.isBlank()) {
                    "Scan a QR or load kovanica-tx-unsigned.json from Downloads."
                } else {
                    "That is not an unsigned Kovanica transaction."
                }
            }
            !AirgapReview.matches(doc.getLong("chainId"), NetworkPrefs.chainId(this)) -> {
                val message = AirgapReview.refusal(doc.getLong("chainId"), NetworkPrefs.chainId(this))
                status.text = message
                if (announce) Toast.makeText(this, message, Toast.LENGTH_LONG).show()
            }
            else -> {
                active = doc
                form.visibility = View.VISIBLE
                status.text = AirgapReview.lines(doc).joinToString("\n")
            }
        }
    }

    private fun review() {
        if (passField.text.toString().isEmpty()) {
            status.text = "Enter the wallet password. The phrase stays on this phone."
            return
        }
        val doc = active ?: return
        AlertDialog.Builder(this)
            .setTitle(if (doc.optString("kind") == "call") "Confirm contract call" else "Confirm transfer")
            .setMessage(AirgapReview.lines(doc).joinToString("\n"))
            .setNegativeButton("Back", null)
            .setPositiveButton("Confirm & sign") { _, _ -> gateBiometric { sign(doc) } }
            .show()
    }

    private fun gateBiometric(onConfirmed: () -> Unit) {
        if (Build.VERSION.SDK_INT < 29) {
            onConfirmed()
            return
        }
        val manager = getSystemService(BiometricManager::class.java)
        @Suppress("DEPRECATION")
        val available = manager?.canAuthenticate() == BiometricManager.BIOMETRIC_SUCCESS
        if (!available) {
            onConfirmed()
            return
        }
        val prompt = BiometricPrompt.Builder(this)
            .setTitle("Confirm this signature")
            .setSubtitle("The password you typed still decrypts the key.")
            .setNegativeButton("Cancel", mainExecutor) { _, _ ->
                status.text = "Signing cancelled."
            }
            .build()
        prompt.authenticate(
            CancellationSignal(),
            mainExecutor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    onConfirmed()
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    if (errorCode == BiometricPrompt.BIOMETRIC_ERROR_NO_BIOMETRICS ||
                        errorCode == BiometricPrompt.BIOMETRIC_ERROR_HW_NOT_PRESENT ||
                        errorCode == BiometricPrompt.BIOMETRIC_ERROR_HW_UNAVAILABLE
                    ) {
                        onConfirmed()
                    } else {
                        status.text = errString.toString()
                    }
                }
            },
        )
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
                runOnUiThread { showSigned(signed) }
            } catch (error: Exception) {
                runOnUiThread { status.text = error.message ?: "The phone could not sign that payload." }
            }
        }
    }

    private fun showSigned(signed: String) {
        status.text = "Signed. Scan this code back on the computer. Nothing was broadcast. It hides in 60 seconds."
        signedText.text = signed
        qr.setImageBitmap(runCatching { qrBitmap(signed, dp(280)) }.getOrNull())
        clearButton.visibility = View.VISIBLE
        exportButton.visibility = View.VISIBLE
        qr.removeCallbacks(hideSigned)
        qr.postDelayed(hideSigned, 60_000)
    }

    private fun clearSigned(message: String) {
        qr.removeCallbacks(hideSigned)
        qr.setImageDrawable(null)
        signedText.text = ""
        clearButton.visibility = View.GONE
        exportButton.visibility = View.GONE
        status.text = message
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
        private const val OPEN_JSON: Int = 41
        private const val SAVE_JSON: Int = 42
    }
}
