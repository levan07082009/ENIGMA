# ProCam

A camera app for the **Pixel 6 Pro** (works on any Android 10+ phone), built around one thing
the stock camera does badly: **smooth zoom**, especially while recording video.

The Pixel 6 Pro's three rear cameras (about 16mm ultrawide, 25mm wide and 104mm 4× tele) sit behind
one *logical* camera. ProCam drives that logical camera's zoom ratio continuously, so a zoom
goes from 0.7× to 20× in one move and the phone hands over between lenses on its own.

## Smooth zoom

Every zoom control goes through `SmoothZoomController`, which updates the zoom once per
display frame (120 Hz on the 6 Pro):

- **Log-scale motion.** Speed is measured in *stops per second* (one stop = zoom doubles).
  1×→2× takes as long as 5×→10×, so a zoom looks steady across the whole range instead of
  crawling wide and racing tele like a linear zoom does.
- **Eased starts and stops.** Every move ramps up and down (0.45 s) and never jumps in speed,
  including when you reverse direction mid-zoom or let go of the rocker.
- **Exact landing.** Glides brake along a curve that stops precisely on the target, with no
  overshoot and bounce.

### Controls

| Control | What it does |
| --- | --- |
| **W / T buttons** (hold) | Zoom out / in at the chosen speed, like a camcorder rocker |
| **Volume keys** (hold) | Same as W / T, so you can zoom without covering the screen |
| **Speed**: CRAWL · SLOW · MED · FAST | 0.1 · 0.25 · 0.5 · 1 stop/s (1×→4× in 20 · 8 · 4 · 2 s) |
| **Preset chips** (tap) | Glide to .7× / 1× / 2× / 4× / 10× / 20× at the chosen speed |
| **Preset chips** (long press) | Quick glide (4 stops/s) |
| **Zoom bar** (drag) | Log-scale bar; the zoom follows your finger on a critically damped spring |
| **Pinch** | Same spring smoothing as the zoom bar |
| **Tap preview** | Focus and meter at that point |

The top-left pill shows which physical lens is live (`ULTRAWIDE · 16mm`, `WIDE · 25mm`,
`TELE · 104mm`), when the phone reports it through `LOGICAL_MULTI_CAMERA_ACTIVE_PHYSICAL_ID`. The readout
shows the zoom ratio and the 35mm-equivalent focal length.

### Capture

- **PHOTO**: max-quality JPEG to `Pictures/ProCam`.
- **VIDEO**: 4K (falls back to the best available quality) with audio to `Movies/ProCam`.
  **STAB** toggles stabilisation. When the phone supports preview stabilisation, the preview is
  stabilised too, so what you see is what is recorded.

## Build and install

Download `procam-apk` from the **ProCam APK** GitHub Actions run, or build it yourself
(JDK 17+ and the Android SDK):

```sh
cd procam
./gradlew assembleRelease        # -> app/build/outputs/apk/release/app-release.apk (~2.3 MB)
adb install -r app/build/outputs/apk/release/app-release.apk
./gradlew testDebugUnitTest      # smooth-zoom motion tests
```

The release build is shrunk with R8, contains only arm64 native code, and is signed with the
debug key so it installs without any signing setup.

## Known limits

- The Pixel picks the moment to switch lenses (around 1× and 4×, and it may stay on the main
  sensor at "4×" in low light or at close range). Apps can't control that. It can cause a small
  shift in colour or framing at the switch point.
- Past 4× the image is a digital crop of the tele sensor. The zoom range and the preset chips
  come from whatever range the camera reports to apps.
- Portrait-locked UI. Photos and videos are still rotated correctly when the phone is held
  sideways.

## Code

- `SmoothZoomController.kt`: the zoom motion engine (no camera dependencies, unit-tested)
- `ZoomBarView.kt`: log-scale zoom bar
- `MainActivity.kt`: CameraX setup, controls, lens detection, photo/video capture
