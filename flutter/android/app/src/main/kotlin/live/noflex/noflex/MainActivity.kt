// SPDX-License-Identifier: AGPL-3.0-only
package live.noflex.noflex

import android.app.PictureInPictureParams
import android.content.res.Configuration
import android.os.Build
import android.util.Rational
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/** Picture-in-picture, which Flutter has no API for. See services/pip_service.dart. */
class MainActivity : FlutterActivity() {
    private var channel: MethodChannel? = null
    private var playing = false
    private var aspect = Rational(16, 9)

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        channel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "live.noflex/pip").also {
            it.setMethodCallHandler { call, result ->
                when (call.method) {
                    "setPlaying" -> {
                        playing = call.argument<Boolean>("playing") ?: false
                        val width = call.argument<Int>("width") ?: 16
                        val height = call.argument<Int>("height") ?: 9
                        aspect = clampAspect(width, height)
                        applyAutoEnter()
                        result.success(null)
                    }
                    "enter" -> {
                        enter()
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }
        }
    }

    // Android refuses ratios outside 1:2.39 to 2.39:1.
    private fun clampAspect(width: Int, height: Int): Rational {
        if (width <= 0 || height <= 0) return Rational(16, 9)
        val ratio = width.toDouble() / height
        return when {
            ratio > 2.39 -> Rational(239, 100)
            ratio < 1 / 2.39 -> Rational(100, 239)
            else -> Rational(width, height)
        }
    }

    private fun params(): PictureInPictureParams {
        val builder = PictureInPictureParams.Builder().setAspectRatio(aspect)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            builder.setAutoEnterEnabled(playing)
            builder.setSeamlessResizeEnabled(false)
        }
        return builder.build()
    }

    private fun applyAutoEnter() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) setPictureInPictureParams(params())
    }

    private fun enter() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) enterPictureInPictureMode(params())
    }

    // Android 12+ enters by itself from setAutoEnterEnabled.
    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (playing && Build.VERSION.SDK_INT < Build.VERSION_CODES.S) enter()
    }

    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        channel?.invokeMethod("changed", isInPictureInPictureMode)
    }
}
