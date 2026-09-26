package com.levan.procam

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.util.AttributeSet
import android.util.TypedValue
import android.view.MotionEvent
import android.view.View
import kotlin.math.exp
import kotlin.math.ln

/**
 * Horizontal zoom scale with a logarithmic axis: every doubling of zoom takes the same width,
 * so the ultrawide, wide and tele ranges each get a usable amount of travel.
 */
class ZoomBarView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {

    /** Called with the ratio under the finger while it is down. */
    var onDrag: ((Float) -> Unit)? = null

    private var minRatio = 1f
    private var maxRatio = 1f
    private var ratio = 1f
    private var marks: List<Float> = emptyList()

    private val density = resources.displayMetrics.density
    private val sidePad = 20 * density

    private val minorPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(110, 255, 255, 255)
        strokeWidth = 1 * density
    }
    private val majorPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.WHITE
        strokeWidth = 1.5f * density
    }
    private val labelPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.WHITE
        textSize = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, 11f, resources.displayMetrics)
        textAlign = Paint.Align.CENTER
    }
    private val accentPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = ACCENT
        strokeWidth = 2 * density
    }
    private val thumb = Path()

    fun setRange(min: Float, max: Float, marks: List<Float>) {
        minRatio = min
        maxRatio = maxOf(max, min * 1.0001f)
        this.marks = marks
        invalidate()
    }

    fun setRatio(r: Float) {
        ratio = r
        invalidate()
    }

    private val usableWidth get() = width - 2 * sidePad

    private fun xFor(r: Float): Float =
        sidePad + (ln(r / minRatio) / ln(maxRatio / minRatio)) * usableWidth

    private fun ratioFor(px: Float): Float {
        val t = ((px - sidePad) / usableWidth).coerceIn(0f, 1f)
        return minRatio * exp(t * ln(maxRatio / minRatio))
    }

    override fun onDraw(canvas: Canvas) {
        val baseY = height * 0.42f
        // Minor ticks every quarter stop.
        val stops = ln(maxRatio / minRatio) / LN2
        val quarterSteps = (stops * 4).toInt()
        for (i in 0..quarterSteps) {
            val px = sidePad + (i / 4f / stops) * usableWidth
            val len = if (i % 4 == 0) 7 * density else 4 * density
            canvas.drawLine(px, baseY - len, px, baseY + len, minorPaint)
        }
        for (m in marks) {
            val px = xFor(m)
            canvas.drawLine(px, baseY - 10 * density, px, baseY + 10 * density, majorPaint)
            canvas.drawText(formatRatio(m), px, height - 4 * density, labelPaint)
        }
        // Thumb: a line through the scale with a small triangle on top.
        val tx = xFor(ratio.coerceIn(minRatio, maxRatio))
        canvas.drawLine(tx, baseY - 14 * density, tx, baseY + 14 * density, accentPaint)
        thumb.reset()
        thumb.moveTo(tx - 6 * density, baseY - 20 * density)
        thumb.lineTo(tx + 6 * density, baseY - 20 * density)
        thumb.lineTo(tx, baseY - 13 * density)
        thumb.close()
        canvas.drawPath(thumb, accentPaint)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                parent?.requestDisallowInterceptTouchEvent(true)
                onDrag?.invoke(ratioFor(event.x))
            }
            MotionEvent.ACTION_MOVE -> onDrag?.invoke(ratioFor(event.x))
            MotionEvent.ACTION_UP -> performClick()
        }
        return true
    }

    override fun performClick(): Boolean = super.performClick()

    companion object {
        const val ACCENT = 0xFFFFC107.toInt()
        private const val LN2 = 0.6931472f

        fun formatRatio(r: Float): String = when {
            r < 1f -> String.format(java.util.Locale.US, "%.1f", r).removePrefix("0")
            r == r.toInt().toFloat() -> r.toInt().toString()
            else -> String.format(java.util.Locale.US, "%.1f", r)
        }
    }
}
