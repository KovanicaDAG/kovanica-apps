package io.kovanica.wallet

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.text.InputType
import android.util.Base64
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import java.io.ByteArrayOutputStream
import java.net.URL
import kotlin.math.roundToInt

/**
 * The wallet's single source of truth for colour and the small set of shared
 * widget builders. `MainActivity` and `MessagesActivity` each keep their own
 * private helpers from before; new wallet surfaces (this file, the receive
 * screen, the token rows) build from here so the palette cannot drift.
 *
 * XML-free and dependency-free — plain `android.widget.*` on the ink surface
 * the rest of the app already uses. Token marks are drawn here; the native
 * KUNA mark is the official artwork bundled in `assets/kuna-logo.webp`.
 */
object WalletUi {
    // Ink-toned surface. Explicit so the screen reads the same on any device theme.
    val INK = 0xFF0E1116.toInt()
    val SURFACE = 0xFF171C23.toInt()
    val SURFACE_2 = 0xFF1E242C.toInt()
    val LINE = 0xFF2A323C.toInt()
    val TEXT = 0xFFEDF1F6.toInt()
    val MUTED = 0xFF95A0AE.toInt()
    val ACCENT = 0xFFF0B429.toInt()
    val ACCENT_INK = 0xFF1A1E25.toInt()
    val ACCENT_BG = 0x1AF0B429.toInt()
    val GOOD = 0xFF5BD6A0.toInt()
    val GOOD_BG = 0x1A5BD6A0.toInt()
    val BAD = 0xFFFF7B84.toInt()
    val BAD_BG = 0x1AFF7B84.toInt()

    /** Press tint: the accent at about 18% alpha. */
    val RIPPLE = 0x2EF0B429.toInt()

    fun rounded(
        ctx: Context,
        fill: Int,
        radiusDp: Int,
        stroke: Int? = null,
        strokeDp: Int = 1,
    ): GradientDrawable = GradientDrawable().apply {
        setColor(fill)
        cornerRadius = dp(ctx, radiusDp).toFloat()
        if (stroke != null) setStroke(dp(ctx, strokeDp), stroke)
    }

    fun ripple(ctx: Context, fill: Int, radiusDp: Int): Drawable = RippleDrawable(
        ColorStateList.valueOf(RIPPLE),
        rounded(ctx, fill, radiusDp, LINE, 1),
        null,
    )

    fun dp(ctx: Context, value: Int): Int =
        (value * ctx.resources.displayMetrics.density).roundToInt()

    fun card(ctx: Context): LinearLayout = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        background = rounded(ctx, SURFACE, 18, LINE, 1)
        setPadding(dp(ctx, 16), dp(ctx, 16), dp(ctx, 16), dp(ctx, 16))
    }

    /**
     * Round coin mark. Native KUNA is the official artwork from [kunaCoin];
     * this disc is its fallback and the shape every token mark shares. A token
     * uses [tokenCoin], the first two letters of its symbol on a colour chosen
     * from the contract address.
     */
    fun coinMark(
        ctx: Context,
        letters: String,
        fill: Int,
        ink: Int = ACCENT_INK,
        sizeDp: Int = 36,
        description: String = letters,
    ): TextView {
        val mark = letters.trim().uppercase().filter { it.isLetterOrDigit() }.take(2).ifEmpty { "?" }
        val size = dp(ctx, sizeDp)
        return TextView(ctx).apply {
            text = mark
            contentDescription = description
            gravity = Gravity.CENTER
            includeFontPadding = false
            textSize = if (mark.length > 1) 12f else 15f
            typeface = Typeface.DEFAULT_BOLD
            setTextColor(ink)
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(fill)
            }
            minWidth = size
            minHeight = size
        }
    }

    /**
     * The KUNA mark: `assets/kuna-logo.webp`, the official artwork with its
     * backdrop clipped away, decoded once and shared by every row. A build
     * that lost the asset keeps the amber "K" disc so the native coin still
     * has a mark.
     */
    fun kunaCoin(ctx: Context, sizeDp: Int = 40): View {
        val art = kunaArt(ctx)
            ?: return coinMark(ctx, "K", ACCENT, ACCENT_INK, sizeDp, "KUNA")
        return ImageView(ctx).apply {
            contentDescription = "KUNA"
            scaleType = ImageView.ScaleType.FIT_CENTER
            setImageBitmap(art)
            minimumWidth = dp(ctx, sizeDp)
            minimumHeight = dp(ctx, sizeDp)
        }
    }

    /**
     * Bundled artwork, decoded at icon size and kept for the process. Read
     * through the application context so a cached bitmap never pins an
     * Activity.
     */
    private fun kunaArt(ctx: Context): Bitmap? {
        if (kunaArtLoaded) return kunaArtCache
        return synchronized(this) {
            if (!kunaArtLoaded) {
                kunaArtCache = runCatching {
                    ctx.applicationContext.assets.open(KUNA_ART).use { input ->
                        decodeBitmap(input.readBytes())
                    }
                }.getOrNull()
                kunaArtLoaded = true
            }
            kunaArtCache
        }
    }

    @Volatile
    private var kunaArtCache: Bitmap? = null

    @Volatile
    private var kunaArtLoaded = false

    /** Token mark. The same contract keeps the same fill. */
    fun tokenCoin(ctx: Context, symbol: String, contract: String, sizeDp: Int = 36): TextView =
        coinMark(ctx, symbol, tokenFill(contract), ACCENT_INK, sizeDp, symbol.ifBlank { "Token" })

    /**
     * Token mark with on-chain artwork when the token carries an icon:
     *
     * - `data:image/…` decodes immediately (that is what the creation screen
     *   uploads, so it always works offline);
     * - `https://…` loads in the background and shows the generated disc until
     *   it arrives;
     * - anything else, or a broken payload, falls back to the disc.
     */
    fun tokenMark(
        ctx: Context,
        symbol: String,
        contract: String,
        icon: String?,
        sizeDp: Int = 36,
    ): View {
        val art = icon?.trim().orEmpty()
        if (art.isEmpty()) return tokenCoin(ctx, symbol, contract, sizeDp)
        val inline = decodeDataUri(art)
        if (inline != null) return imageMark(ctx, inline, description = symbol)
        if (!art.startsWith("https://")) return tokenCoin(ctx, symbol, contract, sizeDp)
        val view = imageMark(ctx, null, description = symbol)
        loadRemoteIcon(view, art)
        return view
    }

    private fun imageMark(ctx: Context, bitmap: Bitmap?, description: String): ImageView =
        ImageView(ctx).apply {
            contentDescription = description.ifBlank { "Token" }
            scaleType = ImageView.ScaleType.CENTER_CROP
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(SURFACE_2)
            }
            clipToOutline = true
            if (bitmap != null) setImageBitmap(bitmap)
        }

    /** Decode a `data:image/…;base64,…` URI; `null` for anything else. */
    private fun decodeDataUri(art: String): Bitmap? {
        if (!art.startsWith("data:image/")) return null
        val comma = art.indexOf(',')
        if (comma <= 0) return null
        return runCatching {
            Base64.decode(art.substring(comma + 1), Base64.DEFAULT)
        }.getOrNull()?.let(::decodeBitmap)
    }

    /**
     * Decode at roughly icon size, never at full resolution: a token is
     * permissionless, so a 10 000 px picture in its state must not become a
     * 400 MB allocation in the wallet.
     */
    private fun decodeBitmap(bytes: ByteArray): Bitmap? = runCatching {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return@runCatching null
        val sample = maxOf(1, minOf(bounds.outWidth, bounds.outHeight) / ICON_PX)
        BitmapFactory.decodeByteArray(
            bytes,
            0,
            bytes.size,
            BitmapFactory.Options().apply { inSampleSize = sample },
        )
    }.getOrNull()

    /** Best-effort fetch for an `https://` icon; the disc stays on any failure. */
    private fun loadRemoteIcon(view: ImageView, url: String) {
        Thread {
            val bytes = runCatching {
                val connection = URL(url).openConnection()
                connection.connectTimeout = 5_000
                connection.readTimeout = 5_000
                connection.getInputStream().use { input ->
                    val out = ByteArrayOutputStream()
                    val buffer = ByteArray(8 * 1024)
                    while (out.size() < ICON_FETCH_LIMIT) {
                        val read = input.read(buffer)
                        if (read <= 0) break
                        out.write(buffer, 0, minOf(read, ICON_FETCH_LIMIT - out.size()))
                    }
                    out.toByteArray()
                }
            }.getOrNull() ?: return@Thread
            val bitmap = decodeBitmap(bytes) ?: return@Thread
            view.post { view.setImageBitmap(bitmap) }
        }.apply { isDaemon = true }.start()
    }

    /** Icon marks are drawn at 36–40 dp; decode anything larger at ~[ICON_PX]. */
    private const val ICON_PX = 96

    /** The official KUNA mark, shipped in `app/src/main/assets/`. */
    private const val KUNA_ART = "kuna-logo.webp"

    /** Refuse to buffer a remote icon larger than this. */
    private const val ICON_FETCH_LIMIT = 512 * 1024

    private val TOKEN_FILLS = intArrayOf(
        0xFF7EB6FF.toInt(),
        0xFF5BD6A0.toInt(),
        0xFFE39A4A.toInt(),
        0xFFC9A0FF.toInt(),
        0xFF4ECDC4.toInt(),
        0xFFFF8FA3.toInt(),
    )

    /** Stable colour for a contract. Unreadable addresses fall back to a hash. */
    fun tokenFill(contract: String): Int {
        val hex = contract.trim().lowercase().removePrefix("0x")
            .filter { it in '0'..'9' || it in 'a'..'f' }
        val seed = hex.take(2).toIntOrNull(16) ?: contract.hashCode().and(0xFF)
        return TOKEN_FILLS[seed % TOKEN_FILLS.size]
    }

    fun sectionLabel(ctx: Context, caption: String, bottomPadDp: Int = 10): TextView =
        TextView(ctx).apply {
            text = caption.uppercase()
            setTextColor(MUTED)
            textSize = 11f
            letterSpacing = 0.18f
            typeface = Typeface.DEFAULT_BOLD
            setPadding(0, 0, 0, dp(ctx, bottomPadDp))
        }

    fun caption(ctx: Context, text: String): TextView = TextView(ctx).apply {
        this.text = text
        setTextColor(MUTED)
        textSize = 12f
        setLineSpacing(0f, 1.3f)
    }

    fun mono(ctx: Context, text: String, size: Float = 12f): TextView = TextView(ctx).apply {
        this.text = text
        setTextColor(TEXT)
        typeface = Typeface.MONOSPACE
        textSize = size
        setTextIsSelectable(true)
        setLineSpacing(0f, 1.25f)
    }

    fun field(ctx: Context, hint: String, value: String = "", mono: Boolean = false): EditText =
        EditText(ctx).apply {
            this.hint = hint
            setText(value)
            setTextColor(TEXT)
            setHintTextColor(MUTED)
            textSize = 15f
            if (mono) typeface = Typeface.MONOSPACE
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            background = rounded(ctx, SURFACE_2, 12, LINE, 1)
            setPadding(dp(ctx, 14), dp(ctx, 13), dp(ctx, 14), dp(ctx, 13))
            maxLines = 1
        }

    fun button(
        ctx: Context,
        caption: String,
        filled: Boolean = true,
        action: () -> Unit,
    ): Button = Button(ctx).apply {
        text = caption
        isAllCaps = false
        textSize = 15f
        typeface = Typeface.DEFAULT_BOLD
        setTextColor(if (filled) ACCENT_INK else ACCENT)
        background = if (filled) rounded(ctx, ACCENT, 12) else rounded(ctx, SURFACE_2, 12, LINE, 1)
        setPadding(dp(ctx, 18), dp(ctx, 12), dp(ctx, 18), dp(ctx, 12))
        minHeight = dp(ctx, 48)
        stateListAnimator = null
        setOnClickListener { action() }
    }

    fun ViewGroup.addStack(child: View, top: Int = 0) {
        addView(
            child,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = dp(context, top) },
        )
    }
}
