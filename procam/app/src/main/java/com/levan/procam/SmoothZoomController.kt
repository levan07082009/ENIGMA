package com.levan.procam

import android.view.Choreographer
import kotlin.math.abs
import kotlin.math.exp
import kotlin.math.ln
import kotlin.math.min
import kotlin.math.sign
import kotlin.math.sqrt

/**
 * Drives the camera zoom ratio smoothly, once per display frame.
 *
 * All motion happens in log space (x = ln(zoom ratio)). Equal steps there are equal perceived
 * magnification changes, so a constant speed in "stops per second" (doublings of zoom) looks
 * uniform from 0.7x to 20x instead of crawling at the wide end and racing at the tele end,
 * which is what a linear zoom does.
 *
 * Two motion modes share one state (position [x], velocity [v]), so switching between them
 * never produces a jump in zoom speed:
 *  - PROFILE: accelerate, cruise at a fixed speed, then brake to land exactly on a target.
 *    Used by the W/T rocker, the volume keys and the preset buttons.
 *  - FOLLOW: critically damped spring chasing a target that keeps moving.
 *    Used by pinch and the zoom bar, where the finger decides the speed.
 */
class SmoothZoomController(
    private val onZoom: (Float) -> Unit,
    /** Frame clock; swapped out in unit tests, which call [doFrame] themselves. */
    private val postFrame: (Choreographer.FrameCallback) -> Unit =
        { Choreographer.getInstance().postFrameCallback(it) },
    private val cancelFrame: (Choreographer.FrameCallback) -> Unit =
        { Choreographer.getInstance().removeFrameCallback(it) },
) : Choreographer.FrameCallback {

    private enum class Mode { IDLE, PROFILE, FOLLOW }

    private var mode = Mode.IDLE
    private var running = false
    private var lastFrameNs = 0L
    private var lastPublished = Float.NaN

    private var x = 0.0
    private var v = 0.0
    private var minX = 0.0
    private var maxX = 0.0

    private var target = 0.0
    private var maxSpeed = 0.0
    private var accel = 1.0

    /** Rocker and preset cruise speed, in stops (doublings of zoom) per second. */
    var speedStops = 0.5

    /** Seconds to reach cruise speed from rest, and to come back to rest. */
    var rampSeconds = 0.45

    /** Lag of the pinch / zoom-bar spring. Higher is smoother but less direct. */
    var followSeconds = 0.14

    val zoomRatio: Float get() = exp(x).toFloat()

    val isMoving: Boolean get() = mode != Mode.IDLE

    fun setRange(minRatio: Float, maxRatio: Float) {
        minX = ln(minRatio.toDouble())
        maxX = ln(maxRatio.toDouble()).coerceAtLeast(minX)
        x = x.coerceIn(minX, maxX)
        target = target.coerceIn(minX, maxX)
    }

    /** Pushes the current ratio to the camera again, e.g. after the use cases were rebound. */
    fun reapply() {
        lastPublished = Float.NaN
        publish()
    }

    /** Starts a rocker press: zoom in (+1) or out (-1) at [speedStops] until [releaseRocker]. */
    fun startRocker(direction: Int) {
        startProfile(if (direction > 0) maxX else minX, speedStops)
    }

    /** Ends a rocker press with a smooth stop instead of a sudden freeze. */
    fun releaseRocker() {
        if (mode != Mode.PROFILE) return
        val stoppingDistance = v * v / (2 * accel)
        target = (x + sign(v) * stoppingDistance).coerceIn(minX, maxX)
    }

    /** Glides to [ratio] with an ease-in / cruise / ease-out motion at [stopsPerSecond]. */
    fun glideTo(ratio: Float, stopsPerSecond: Double = speedStops) {
        startProfile(ln(ratio.toDouble()), stopsPerSecond)
    }

    /** Chases [ratio] with a spring. Call it repeatedly while a finger is moving. */
    fun follow(ratio: Float) {
        target = ln(ratio.toDouble()).coerceIn(minX, maxX)
        mode = Mode.FOLLOW
        ensureRunning()
    }

    /** Multiplies the spring target by [scale], for pinch gestures. */
    fun followBy(scale: Float) {
        val base = if (mode == Mode.FOLLOW) target else x
        follow(exp(base).toFloat() * scale)
    }

    fun stop() {
        mode = Mode.IDLE
        v = 0.0
        if (running) cancelFrame(this)
        running = false
    }

    private fun startProfile(targetX: Double, stopsPerSecond: Double) {
        target = targetX.coerceIn(minX, maxX)
        maxSpeed = stopsPerSecond * LN2
        accel = maxSpeed / rampSeconds
        mode = Mode.PROFILE
        ensureRunning()
    }

    private fun ensureRunning() {
        if (running) return
        running = true
        lastFrameNs = 0L
        postFrame(this)
    }

    override fun doFrame(frameTimeNanos: Long) {
        if (lastFrameNs != 0L) {
            // Clamp dt so a stalled frame does not turn into a visible jump.
            val dt = ((frameTimeNanos - lastFrameNs) / 1e9).coerceIn(0.0, 0.05)
            when (mode) {
                Mode.PROFILE -> stepProfile(dt)
                Mode.FOLLOW -> stepFollow(dt)
                Mode.IDLE -> Unit
            }
            publish()
        }
        lastFrameNs = frameTimeNanos
        if (mode != Mode.IDLE) {
            postFrame(this)
        } else {
            running = false
        }
    }

    private fun stepProfile(dt: Double) {
        val d = target - x
        if (abs(d) < EPSILON && abs(v) < accel * dt) {
            finish()
            return
        }
        // The fastest speed from which we can still brake to rest exactly on the target.
        val desired = sign(d) * min(maxSpeed, sqrt(2 * accel * abs(d)))
        val landing = sign(desired) == sign(v) && abs(desired) < abs(v)
        // The final approach may brake a little harder than the ramp so discrete frames never
        // overshoot; every other change of speed is limited to the ramp so it always feels soft.
        val dv = (if (landing) 2 * accel else accel) * dt
        v = if (desired > v) min(v + dv, desired) else maxOf(v - dv, desired)
        x += v * dt
        if (sign(target - x) != sign(d)) finish()
    }

    private fun stepFollow(dt: Double) {
        // Critically damped spring (the classic "SmoothDamp"): no overshoot, continuous velocity.
        val omega = 2.0 / followSeconds
        val k = omega * dt
        val decay = 1.0 / (1.0 + k + 0.48 * k * k + 0.235 * k * k * k)
        val change = x - target
        val temp = (v + omega * change) * dt
        v = (v - omega * temp) * decay
        x = (target + (change + temp) * decay).coerceIn(minX, maxX)
        if (abs(target - x) < EPSILON && abs(v) < 1e-3) finish()
    }

    private fun finish() {
        x = target
        v = 0.0
        mode = Mode.IDLE
    }

    private fun publish() {
        val r = zoomRatio
        if (lastPublished.isNaN() || abs(r - lastPublished) > r * 1e-5f) {
            lastPublished = r
            onZoom(r)
        }
    }

    private companion object {
        val LN2 = ln(2.0)
        const val EPSILON = 1e-4
    }
}
