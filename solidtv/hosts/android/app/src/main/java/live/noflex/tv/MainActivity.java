// SPDX-License-Identifier: AGPL-3.0-only
//
// One full-screen WebView and nothing else: the app is in assets, drawn by its own
// renderer, and playing through a <video> behind its canvas. This only hands it the
// keys a WebView keeps to itself, and the two things a page can't do for itself.

package live.noflex.tv;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Color;
import android.os.Build;
import android.os.Bundle;
import android.view.KeyEvent;
import android.view.View;
import android.view.WindowManager;
import android.webkit.JavascriptInterface;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.window.OnBackInvokedDispatcher;
import androidx.webkit.WebViewAssetLoader;

public class MainActivity extends Activity {
    private WebView webView;

    @SuppressLint({"SetJavaScriptEnabled", "AddJavascriptInterface"})
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        hideSystemBars();

        // A real origin rather than file://, so module scripts load and storage persists. Plain
        // http, because an https page's <video> is auto-upgraded to https whatever the mixed
        // content mode says, which a home server on plain HTTP answers with nothing.
        WebViewAssetLoader assets = new WebViewAssetLoader.Builder()
                .setHttpAllowed(true)
                .addPathHandler("/assets/", new WebViewAssetLoader.AssetsPathHandler(this))
                .build();

        // chrome://inspect, on the debug APK.
        WebView.setWebContentsDebuggingEnabled((getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0);
        webView = new WebView(this);
        webView.setBackgroundColor(Color.parseColor("#07080b"));
        webView.setFocusable(true);
        webView.setFocusableInTouchMode(true);
        WebSettings settings = webView.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setMediaPlaybackRequiresUserGesture(false);
        webView.addJavascriptInterface(new Host(), "NoFlexHost");
        webView.setWebViewClient(new WebViewClient() {
            @Override
            public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                return assets.shouldInterceptRequest(request.getUrl());
            }

            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                return false;
            }
        });
        // Otherwise a WebView draws its grey play button over any <video> without a poster.
        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public Bitmap getDefaultVideoPoster() {
                return Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888);
            }
        });
        setContentView(webView);
        webView.requestFocus();
        // Predictive back: from targetSdk 36 the key never reaches dispatchKeyEvent, and the
        // system would leave the app instead. Measured on an Android 16 emulator.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, () -> {
                sendKey("keydown", "GoBack");
                sendKey("keyup", "GoBack");
            });
        }
        webView.loadUrl("http://appassets.androidplatform.net/assets/index.html");
    }

    private void hideSystemBars() {
        getWindow().getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                        | View.SYSTEM_UI_FLAG_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                        | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
    }

    // A WebView acts on Back and the media keys itself rather than passing them to the
    // page, so they're delivered as what the page's key map reads.
    private static String pageKey(int keyCode) {
        switch (keyCode) {
            case KeyEvent.KEYCODE_BACK: return "GoBack";
            case KeyEvent.KEYCODE_MEDIA_PLAY: return "MediaPlay";
            case KeyEvent.KEYCODE_MEDIA_PAUSE: return "MediaPause";
            case KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE: return "MediaPlayPause";
            case KeyEvent.KEYCODE_MEDIA_STOP: return "MediaStop";
            case KeyEvent.KEYCODE_MEDIA_FAST_FORWARD: return "MediaFastForward";
            case KeyEvent.KEYCODE_MEDIA_REWIND: return "MediaRewind";
            default: return null;
        }
    }

    private void sendKey(String type, String key) {
        webView.evaluateJavascript("document.dispatchEvent(new KeyboardEvent('" + type + "',{key:'" + key + "',bubbles:true}))", null);
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        String key = pageKey(event.getKeyCode());
        if (key == null) return super.dispatchKeyEvent(event);
        // The callback in onCreate has this one.
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return true;
        sendKey(event.getAction() == KeyEvent.ACTION_DOWN ? "keydown" : "keyup", key);
        return true;
    }

    @Override
    protected void onPause() {
        super.onPause();
        webView.onPause();
    }

    @Override
    protected void onResume() {
        super.onResume();
        webView.onResume();
        hideSystemBars();
    }

    /** What the page reaches as `window.NoFlexHost`; see src/lib/host.ts. */
    private final class Host {
        /** One APK runs on all of these, and a WebView's user agent can't tell them apart. */
        @JavascriptInterface
        public String deviceName() {
            PackageManager packages = getPackageManager();
            String family = "Android TV";
            if (packages.hasSystemFeature("amazon.hardware.fire_tv")) {
                family = "Fire TV";
            } else {
                try {
                    packages.getPackageInfo("com.google.android.apps.tv.launcherx", 0);
                    family = "Google TV";
                } catch (PackageManager.NameNotFoundException notGoogleTv) {
                    // Android TV's own launcher, or a vendor's.
                }
            }
            return family + " (" + Build.MODEL + ")";
        }

        @JavascriptInterface
        public void exit() {
            runOnUiThread(MainActivity.this::finish);
        }

        @JavascriptInterface
        public void keepAwake(boolean on) {
            runOnUiThread(() -> {
                if (on) getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                else getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            });
        }
    }
}
