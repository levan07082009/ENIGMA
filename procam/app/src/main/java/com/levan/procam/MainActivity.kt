package com.levan.procam

import android.Manifest
import android.annotation.SuppressLint
import android.content.ContentValues
import android.content.pm.PackageManager
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.CaptureResult
import android.hardware.camera2.TotalCaptureResult
import android.os.Bundle
import android.provider.MediaStore
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.OrientationEventListener
import android.view.ScaleGestureDetector
import android.view.Surface
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.OptIn
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.camera2.interop.Camera2CameraInfo
import androidx.camera.camera2.interop.Camera2Interop
import androidx.camera.camera2.interop.ExperimentalCamera2Interop
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.FocusMeteringAction
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.video.FallbackStrategy
import androidx.camera.video.MediaStoreOutputOptions
import androidx.camera.video.Quality
import androidx.camera.video.QualitySelector
import androidx.camera.video.Recorder
import androidx.camera.video.Recording
import androidx.camera.video.VideoCapture
import androidx.camera.video.VideoRecordEvent
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.updatePadding
import com.levan.procam.databinding.ActivityMainBinding
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.TimeUnit
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.log2
import kotlin.math.roundToInt

@OptIn(ExperimentalCamera2Interop::class)
class MainActivity : AppCompatActivity() {

    private lateinit var b: ActivityMainBinding

    private var cameraProvider: ProcessCameraProvider? = null
    private var camera: Camera? = null
    private var imageCapture: ImageCapture? = null
    private var videoCapture: VideoCapture<Recorder>? = null
    private var recording: Recording? = null

    private var videoMode = false
    private var stabilization = true
    private var targetRotation = Surface.ROTATION_0

    /** 35mm-equivalent focal length at 1x, used for the "· 60mm" part of the readout. */
    private var baseFocalMm = 0f
    private var presets: List<Float> = emptyList()
    private val presetViews = mutableListOf<TextView>()
    private val speedViews = mutableListOf<TextView>()

    /** Physical camera id -> label, and which one the HAL is streaming from right now. */
    @Volatile private var lensLabels: Map<String, String> = emptyMap()
    @Volatile private var activePhysicalId: String? = null

    private val zoom = SmoothZoomController(onZoom = { ratio ->
        camera?.cameraControl?.setZoomRatio(ratio)
        showZoom(ratio)
    })

    private val permissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
            if (result[Manifest.permission.CAMERA] == true) {
                startCamera()
            } else {
                Toast.makeText(this, R.string.need_camera, Toast.LENGTH_LONG).show()
                finish()
            }
        }

    private val orientationListener by lazy {
        object : OrientationEventListener(this) {
            override fun onOrientationChanged(orientation: Int) {
                if (orientation == ORIENTATION_UNKNOWN) return
                val rotation = when (orientation) {
                    in 45..134 -> Surface.ROTATION_270
                    in 135..224 -> Surface.ROTATION_180
                    in 225..314 -> Surface.ROTATION_90
                    else -> Surface.ROTATION_0
                }
                if (rotation != targetRotation) {
                    targetRotation = rotation
                    imageCapture?.targetRotation = rotation
                    // Changing it mid-recording would not affect the file, so leave it alone.
                    if (recording == null) videoCapture?.targetRotation = rotation
                }
            }
        }
    }

    private val captureCallback = object : CameraCaptureSession.CaptureCallback() {
        override fun onCaptureCompleted(
            session: CameraCaptureSession,
            request: CaptureRequest,
            result: TotalCaptureResult,
        ) {
            val id = result.get(CaptureResult.LOGICAL_MULTI_CAMERA_ACTIVE_PHYSICAL_ID) ?: return
            if (id != activePhysicalId) {
                activePhysicalId = id
                runOnUiThread { showLens() }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivityMainBinding.inflate(layoutInflater)
        setContentView(b.root)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        val basePadTop = b.topBar.paddingTop
        val basePadBottom = b.bottomPanel.paddingBottom
        ViewCompat.setOnApplyWindowInsetsListener(b.root) { _, insets ->
            val bars = insets.getInsets(
                WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
            )
            b.topBar.updatePadding(top = basePadTop + bars.top + dp(8))
            b.bottomPanel.updatePadding(bottom = basePadBottom + bars.bottom + dp(12))
            insets
        }

        b.preview.implementationMode = PreviewView.ImplementationMode.PERFORMANCE
        // Show the whole frame that is being recorded, not a cropped fill.
        b.preview.scaleType = PreviewView.ScaleType.FIT_CENTER

        val prefs = getPreferences(MODE_PRIVATE)
        zoom.speedStops = prefs.getFloat(PREF_SPEED, 0.5f).toDouble()
        stabilization = prefs.getBoolean(PREF_STAB, true)
        videoMode = intent?.action == MediaStore.INTENT_ACTION_VIDEO_CAMERA ||
            prefs.getBoolean(PREF_VIDEO, false)

        setUpZoomControls()
        setUpSpeedRow()
        setUpModeAndShutter()
        setUpPreviewGestures()

        val needed = mutableListOf(Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO)
        needed.removeAll { granted(it) }
        if (granted(Manifest.permission.CAMERA)) startCamera()
        if (needed.isNotEmpty()) permissionLauncher.launch(needed.toTypedArray())
    }

    override fun onStart() {
        super.onStart()
        orientationListener.enable()
    }

    override fun onStop() {
        super.onStop()
        orientationListener.disable()
        recording?.stop()
        zoom.stop()
    }

    // ---------------------------------------------------------------------------------------
    // Camera

    private fun startCamera() {
        if (cameraProvider != null) return
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            cameraProvider = future.get()
            bindUseCases()
        }, ContextCompat.getMainExecutor(this))
    }

    private fun bindUseCases() {
        val provider = cameraProvider ?: return
        val selector = CameraSelector.DEFAULT_BACK_CAMERA
        val info = try {
            provider.getCameraInfo(selector)
        } catch (e: IllegalArgumentException) {
            Toast.makeText(this, "No back camera found", Toast.LENGTH_LONG).show()
            return
        }

        val previewBuilder = Preview.Builder()
        Camera2Interop.Extender(previewBuilder).setSessionCaptureCallback(captureCallback)

        // Preview stabilisation also stabilises the recording and keeps preview == recording;
        // plain video stabilisation is the fallback where only that is available.
        val previewStab = Preview.getPreviewCapabilities(info).isStabilizationSupported
        val videoStab = Recorder.getVideoCapabilities(info).isStabilizationSupported
        val stabSupported = videoMode && (previewStab || videoStab)
        if (stabSupported && stabilization && previewStab) {
            previewBuilder.setPreviewStabilizationEnabled(true)
        }
        val preview = previewBuilder.build()
        preview.setSurfaceProvider(b.preview.surfaceProvider)

        imageCapture = null
        videoCapture = null
        val second = if (videoMode) {
            val recorder = Recorder.Builder()
                .setQualitySelector(
                    QualitySelector.from(
                        Quality.UHD,
                        FallbackStrategy.lowerQualityOrHigherThan(Quality.UHD),
                    )
                )
                .build()
            val builder = VideoCapture.Builder(recorder).setTargetRotation(targetRotation)
            if (stabSupported && stabilization && !previewStab) {
                builder.setVideoStabilizationEnabled(true)
            }
            builder.build().also { videoCapture = it }
        } else {
            ImageCapture.Builder()
                .setCaptureMode(ImageCapture.CAPTURE_MODE_MAXIMIZE_QUALITY)
                .setTargetRotation(targetRotation)
                .build()
                .also { imageCapture = it }
        }

        provider.unbindAll()
        val cam = try {
            provider.bindToLifecycle(this, selector, preview, second)
        } catch (e: IllegalArgumentException) {
            Toast.makeText(this, "Camera setup failed: ${e.message}", Toast.LENGTH_LONG).show()
            return
        }
        camera = cam

        b.stabToggle.visibility = if (stabSupported) View.VISIBLE else View.INVISIBLE
        b.stabToggle.setText(if (stabilization) R.string.stab_on else R.string.stab_off)

        loadLensInfo(cam)
        cam.cameraInfo.zoomState.removeObservers(this)
        cam.cameraInfo.zoomState.observe(this) { applyZoomRange(it.minZoomRatio, it.maxZoomRatio) }
        // Rebinding resets the camera to 1x; put it back where the user left it.
        zoom.reapply()
    }

    private var rangeMin = 0f
    private var rangeMax = 0f

    private fun applyZoomRange(min: Float, max: Float) {
        if (min == rangeMin && max == rangeMax) return
        rangeMin = min
        rangeMax = max
        zoom.setRange(min, max)

        val candidates = listOf(1f, 2f, 4f, 10f, 20f, 30f, 50f, 100f)
        presets = buildList {
            if (min < 0.99f) add(min)
            addAll(candidates.filter { it >= min && it <= max * 1.001f })
        }
        b.zoomBar.setRange(min, max, presets)
        buildPresetRow()
        showZoom(zoom.zoomRatio)
    }

    /** Works out which physical lenses sit behind the logical camera and labels them. */
    private fun loadLensInfo(cam: Camera) {
        val manager = getSystemService(CameraManager::class.java) ?: return
        try {
            val logicalId = Camera2CameraInfo.from(cam.cameraInfo).cameraId
            val logical = manager.getCameraCharacteristics(logicalId)
            baseFocalMm = equivalentFocal(logical)
            val ids = logical.physicalCameraIds.ifEmpty { setOf(logicalId) }
            lensLabels = ids.associateWith { id ->
                val mm = equivalentFocal(manager.getCameraCharacteristics(id))
                val name = when {
                    mm <= 0f -> "LENS $id"
                    mm < 20f -> "ULTRAWIDE"
                    mm < 40f -> "WIDE"
                    else -> "TELE"
                }
                if (mm > 0f) "$name · ${mm.roundToInt()}mm" else name
            }
        } catch (e: Exception) {
            lensLabels = emptyMap()
        }
        showLens()
    }

    private fun equivalentFocal(c: CameraCharacteristics): Float {
        val focal = c.get(CameraCharacteristics.LENS_INFO_AVAILABLE_FOCAL_LENGTHS)?.firstOrNull()
        val size = c.get(CameraCharacteristics.SENSOR_INFO_PHYSICAL_SIZE)
        if (focal == null || size == null) return 0f
        return focal * FULL_FRAME_DIAGONAL_MM / hypot(size.width, size.height)
    }

    // ---------------------------------------------------------------------------------------
    // Zoom UI

    @SuppressLint("ClickableViewAccessibility")
    private fun setUpZoomControls() {
        b.zoomBar.onDrag = { ratio -> zoom.follow(ratio) }

        fun rocker(view: View, direction: Int) {
            view.setOnTouchListener { v, event ->
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        v.isPressed = true
                        zoom.startRocker(direction)
                    }
                    MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                        v.isPressed = false
                        zoom.releaseRocker()
                        if (event.actionMasked == MotionEvent.ACTION_UP) v.performClick()
                    }
                }
                true
            }
        }
        rocker(b.rockerIn, +1)
        rocker(b.rockerOut, -1)
    }

    private fun buildPresetRow() {
        b.presetRow.removeAllViews()
        presetViews.clear()
        for (p in presets) {
            val chip = chip(ZoomBarView.formatRatio(p) + if (p >= 1f) "×" else "")
            // Tap: glide there at the chosen cinematic speed. Long press: get there quickly.
            chip.setOnClickListener { zoom.glideTo(p) }
            chip.setOnLongClickListener {
                zoom.glideTo(p, stopsPerSecond = QUICK_GLIDE_STOPS)
                true
            }
            b.presetRow.addView(chip)
            presetViews += chip
        }
    }

    private fun setUpSpeedRow() {
        for ((label, speed) in SPEEDS) {
            val chip = chip(label)
            chip.setOnClickListener {
                zoom.speedStops = speed.toDouble()
                getPreferences(MODE_PRIVATE).edit { putFloat(PREF_SPEED, speed) }
                showSpeed()
            }
            chip.tag = speed
            b.speedRow.addView(chip)
            speedViews += chip
        }
        showSpeed()
    }

    private fun showSpeed() {
        val speed = zoom.speedStops.toFloat()
        speedViews.forEach { it.isSelected = it.tag == speed }
        // 1x -> 4x is the wide-to-tele move on a Pixel 6 Pro: exactly two stops.
        val seconds = log2(4f) / speed
        b.speedHint.text = String.format(Locale.US, "1× → 4× in %.1f s", seconds)
    }

    private fun showZoom(ratio: Float) {
        val focal = if (baseFocalMm > 0f) "  ·  ${(baseFocalMm * ratio).roundToInt()}mm" else ""
        b.zoomReadout.text = String.format(Locale.US, "%.2f×%s", ratio, focal)
        b.zoomBar.setRatio(ratio)
        for ((i, view) in presetViews.withIndex()) {
            view.isSelected = abs(ratio / presets[i] - 1f) < 0.02f
        }
    }

    private fun showLens() {
        val label = activePhysicalId?.let { lensLabels[it] }
        b.lensLabel.visibility = if (label != null) View.VISIBLE else View.INVISIBLE
        b.lensLabel.text = label
    }

    // ---------------------------------------------------------------------------------------
    // Capture

    private fun setUpModeAndShutter() {
        b.modePhoto.setOnClickListener { setVideoMode(false) }
        b.modeVideo.setOnClickListener { setVideoMode(true) }
        b.shutter.setOnClickListener { if (videoMode) toggleRecording() else takePhoto() }
        b.stabToggle.setOnClickListener {
            if (recording != null) return@setOnClickListener
            stabilization = !stabilization
            getPreferences(MODE_PRIVATE).edit { putBoolean(PREF_STAB, stabilization) }
            bindUseCases()
        }
        showMode()
    }

    private fun setVideoMode(video: Boolean) {
        if (video == videoMode || recording != null) return
        videoMode = video
        getPreferences(MODE_PRIVATE).edit { putBoolean(PREF_VIDEO, video) }
        showMode()
        bindUseCases()
    }

    private fun showMode() {
        b.modePhoto.isSelected = !videoMode
        b.modeVideo.isSelected = videoMode
        b.shutter.setBackgroundResource(
            when {
                recording != null -> R.drawable.shutter_recording
                videoMode -> R.drawable.shutter_video
                else -> R.drawable.shutter_photo
            }
        )
        if (!videoMode) b.stabToggle.visibility = View.INVISIBLE
    }

    private fun takePhoto() {
        val capture = imageCapture ?: return
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, fileName())
            put(MediaStore.MediaColumns.MIME_TYPE, "image/jpeg")
            put(MediaStore.MediaColumns.RELATIVE_PATH, "Pictures/ProCam")
        }
        val options = ImageCapture.OutputFileOptions.Builder(
            contentResolver, MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values
        ).build()
        b.flash.animate().cancel()
        b.flash.alpha = 0.8f
        b.flash.animate().alpha(0f).setDuration(250).start()
        capture.takePicture(
            options,
            ContextCompat.getMainExecutor(this),
            object : ImageCapture.OnImageSavedCallback {
                override fun onImageSaved(output: ImageCapture.OutputFileResults) {
                    Toast.makeText(this@MainActivity, "Saved to Pictures/ProCam", Toast.LENGTH_SHORT).show()
                }

                override fun onError(e: ImageCaptureException) {
                    Toast.makeText(this@MainActivity, "Photo failed: ${e.message}", Toast.LENGTH_LONG).show()
                }
            },
        )
    }

    @SuppressLint("MissingPermission")
    private fun toggleRecording() {
        recording?.let {
            it.stop()
            return
        }
        val capture = videoCapture ?: return
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, fileName())
            put(MediaStore.MediaColumns.MIME_TYPE, "video/mp4")
            put(MediaStore.MediaColumns.RELATIVE_PATH, "Movies/ProCam")
        }
        val options = MediaStoreOutputOptions.Builder(
            contentResolver, MediaStore.Video.Media.EXTERNAL_CONTENT_URI
        ).setContentValues(values).build()

        var pending = capture.output.prepareRecording(this, options)
        if (granted(Manifest.permission.RECORD_AUDIO)) pending = pending.withAudioEnabled()
        recording = pending.start(ContextCompat.getMainExecutor(this)) { event ->
            when (event) {
                is VideoRecordEvent.Start -> {
                    b.recTimer.visibility = View.VISIBLE
                    b.recTimer.text = "● 00:00"
                    showMode()
                }
                is VideoRecordEvent.Status -> {
                    val s = TimeUnit.NANOSECONDS.toSeconds(event.recordingStats.recordedDurationNanos)
                    b.recTimer.text = String.format(Locale.US, "● %02d:%02d", s / 60, s % 60)
                }
                is VideoRecordEvent.Finalize -> {
                    recording = null
                    b.recTimer.visibility = View.GONE
                    showMode()
                    val msg = if (event.hasError()) {
                        "Recording failed (error ${event.error})"
                    } else {
                        "Saved to Movies/ProCam"
                    }
                    Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()
                    videoCapture?.targetRotation = targetRotation
                }
                else -> Unit
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // Gestures and keys

    @SuppressLint("ClickableViewAccessibility")
    private fun setUpPreviewGestures() {
        val scale = ScaleGestureDetector(this, object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            override fun onScale(detector: ScaleGestureDetector): Boolean {
                zoom.followBy(detector.scaleFactor)
                return true
            }
        })
        b.preview.setOnTouchListener { v, event ->
            scale.onTouchEvent(event)
            if (event.actionMasked == MotionEvent.ACTION_UP && !scale.isInProgress &&
                event.eventTime - event.downTime < 250
            ) {
                focusAt(event.x, event.y)
                v.performClick()
            }
            true
        }
    }

    private fun focusAt(x: Float, y: Float) {
        val cam = camera ?: return
        val point = b.preview.meteringPointFactory.createPoint(x, y)
        cam.cameraControl.startFocusAndMetering(
            FocusMeteringAction.Builder(point)
                .setAutoCancelDuration(5, TimeUnit.SECONDS)
                .build()
        )
        val ring = b.focusRing
        ring.x = x - ring.width / 2f
        ring.y = y - ring.height / 2f
        ring.animate().cancel()
        ring.alpha = 1f
        ring.scaleX = 1.4f
        ring.scaleY = 1.4f
        ring.animate().scaleX(1f).scaleY(1f).setDuration(200).withEndAction {
            ring.animate().alpha(0f).setStartDelay(900).setDuration(300).start()
        }.start()
    }

    /** Volume keys are a hardware zoom rocker: hold up to zoom in, down to zoom out. */
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        when (keyCode) {
            KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.KEYCODE_ZOOM_IN -> {
                if (event.repeatCount == 0) zoom.startRocker(+1)
                return true
            }
            KeyEvent.KEYCODE_VOLUME_DOWN, KeyEvent.KEYCODE_ZOOM_OUT -> {
                if (event.repeatCount == 0) zoom.startRocker(-1)
                return true
            }
            KeyEvent.KEYCODE_CAMERA -> {
                if (event.repeatCount == 0) b.shutter.performClick()
                return true
            }
        }
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        when (keyCode) {
            KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.KEYCODE_VOLUME_DOWN,
            KeyEvent.KEYCODE_ZOOM_IN, KeyEvent.KEYCODE_ZOOM_OUT -> {
                zoom.releaseRocker()
                return true
            }
        }
        return super.onKeyUp(keyCode, event)
    }

    // ---------------------------------------------------------------------------------------
    // Helpers

    private fun chip(text: String): TextView =
        (layoutInflater.inflate(R.layout.chip, b.root, false) as TextView).also {
            it.text = text
            it.layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, dp(34)
            )
        }

    private fun granted(permission: String) =
        ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED

    private fun fileName() =
        "PROCAM_" + SimpleDateFormat("yyyyMMdd_HHmmss_SSS", Locale.US).format(Date())

    private fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()

    private companion object {
        const val PREF_SPEED = "zoom_speed"
        const val PREF_STAB = "stabilization"
        const val PREF_VIDEO = "video_mode"
        const val FULL_FRAME_DIAGONAL_MM = 43.27f
        const val QUICK_GLIDE_STOPS = 4.0

        /** Rocker / preset cruise speeds in stops per second. */
        val SPEEDS = listOf("CRAWL" to 0.1f, "SLOW" to 0.25f, "MED" to 0.5f, "FAST" to 1f)
    }
}
