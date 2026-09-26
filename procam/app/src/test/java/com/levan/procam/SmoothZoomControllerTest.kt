package com.levan.procam

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import kotlin.math.abs
import kotlin.math.log2

class SmoothZoomControllerTest {

    private val frameNs = 1_000_000_000L / 120 // Pixel 6 Pro display rate
    private var now = 1_000_000_000L
    private val ratios = mutableListOf<Float>()
    private lateinit var zoom: SmoothZoomController

    @Before
    fun setUp() {
        zoom = SmoothZoomController(onZoom = { ratios += it }, postFrame = {}, cancelFrame = {})
        zoom.setRange(0.7f, 20f)
    }

    /** Runs frames until the controller comes to rest (or [maxSeconds] pass). */
    private fun runFrames(maxSeconds: Double = 60.0) {
        val limit = (maxSeconds * 120).toInt()
        repeat(limit) {
            zoom.doFrame(now)
            now += frameNs
            if (!zoom.isMoving) return
        }
    }

    private fun runFor(seconds: Double) {
        repeat((seconds * 120).toInt()) {
            zoom.doFrame(now)
            now += frameNs
        }
    }

    /** Zoom speed per frame in stops per second. */
    private fun speeds(): List<Double> {
        val stops = ratios.map { log2(it.toDouble()) }
        return stops.zipWithNext { a, b -> (b - a) * 120 }
    }

    @Test
    fun glideLandsExactlyOnTargetWithoutOvershoot() {
        zoom.speedStops = 0.5
        zoom.glideTo(4f)
        runFrames()
        assertFalse(zoom.isMoving)
        assertEquals(4f, zoom.zoomRatio, 1e-4f)
        assertTrue("never passes the target", ratios.all { it <= 4f + 1e-4f })
        assertTrue("monotonic", ratios.zipWithNext().all { (a, b) -> b >= a })
    }

    @Test
    fun glideCruisesAtChosenSpeedAndTakesExpectedTime() {
        zoom.speedStops = 0.5
        zoom.glideTo(4f) // two stops
        runFrames()
        val peak = speeds().max()
        assertEquals(0.5, peak, 0.01)
        // Two stops at 0.5 stops/s is 4 s of cruise, plus one ramp time for the soft ends.
        val seconds = ratios.size / 120.0
        assertEquals(4.0 + zoom.rampSeconds, seconds, 0.1)
    }

    @Test
    fun speedChangesGraduallyNoJerks() {
        zoom.speedStops = 1.0
        zoom.glideTo(10f)
        runFrames()
        val maxDelta = speeds().zipWithNext { a, b -> abs(b - a) }.max()
        // Ramp accel is speed / rampSeconds; landing may use twice that. Per frame at 120 Hz:
        val allowed = 2 * (1.0 / zoom.rampSeconds) / 120 * 1.05
        assertTrue("speed jump $maxDelta > $allowed", maxDelta <= allowed)
    }

    @Test
    fun rockerReleaseStopsSmoothly() {
        zoom.speedStops = 0.5
        zoom.startRocker(+1)
        runFor(2.0)
        val speedAtRelease = speeds().last()
        assertEquals(0.5, speedAtRelease, 0.01)
        zoom.releaseRocker()
        val before = ratios.size
        runFrames()
        assertFalse(zoom.isMoving)
        val stopping = speeds().drop(before - 1)
        // It keeps moving forward while slowing down, rather than freezing in one frame.
        assertTrue(stopping.size > 20)
        assertTrue(stopping.all { it >= -1e-9 })
        assertTrue(stopping.zipWithNext().all { (a, b) -> b <= a + 1e-9 })
    }

    @Test
    fun rockerBrakesToAStopAtTheEndOfTheRange() {
        zoom.speedStops = 1.0
        zoom.startRocker(+1)
        runFrames()
        assertFalse(zoom.isMoving)
        assertEquals(20f, zoom.zoomRatio, 1e-3f)
        assertTrue(ratios.all { it <= 20f + 1e-3f })
        assertTrue("slows down before the end", speeds().takeLast(10).all { it < 0.5 })
    }

    @Test
    fun reversingDirectionKeepsSpeedContinuous() {
        zoom.speedStops = 1.0
        zoom.startRocker(+1)
        runFor(1.0)
        zoom.startRocker(-1)
        runFor(2.0)
        val maxDelta = speeds().zipWithNext { a, b -> abs(b - a) }.max()
        val allowed = 2 * (1.0 / zoom.rampSeconds) / 120 * 1.05
        assertTrue("speed jump $maxDelta > $allowed", maxDelta <= allowed)
        assertTrue("now zooming out", speeds().last() < 0)
    }

    @Test
    fun followConvergesWithoutOvershoot() {
        zoom.follow(3f)
        runFrames()
        assertFalse(zoom.isMoving)
        assertEquals(3f, zoom.zoomRatio, 1e-3f)
        assertTrue(ratios.all { it <= 3f + 1e-3f })
    }

    @Test
    fun targetsAreClampedToTheCameraRange() {
        zoom.glideTo(100f, stopsPerSecond = 8.0)
        runFrames()
        assertEquals(20f, zoom.zoomRatio, 1e-3f)
        zoom.follow(0.1f)
        runFrames()
        assertEquals(0.7f, zoom.zoomRatio, 1e-3f)
    }
}
