// SPDX-License-Identifier: AGPL-3.0-only
plugins {
    id("com.android.application")
}

android {
    namespace = "live.noflex.tv"
    compileSdk = 36

    defaultConfig {
        applicationId = "live.noflex.tv"
        // Android TV's floor. The app itself runs in whatever system WebView the box has.
        minSdk = 23
        targetSdk = 36
        versionCode = 1
        versionName = "0.0.1"
    }

    // `just tv build android` writes this.
    sourceSets["main"].assets.srcDir("../../../dist/android")

    buildTypes {
        release {
            isMinifyEnabled = false
            // Signed by the debug key unless told otherwise, as flutter/android is.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
}
