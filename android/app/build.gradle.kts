plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "app.rhcs.pulse"
    compileSdk = 35

    defaultConfig {
        applicationId = "app.rhcs.pulse"
        minSdk = 24
        // 34 on purpose: newer targets force edge-to-edge layouts and local-network permission prompts
        // that this simple app does not need yet.
        targetSdk = 34
        versionCode = 2
        versionName = "0.2.0"
        // The built-in engine (rhcsd) is an aarch64 binary shipped as librhcsd.so.
        ndk { abiFilters += "arm64-v8a" }
    }

    packaging {
        // The engine is executed from the app's native library folder, so it must be extracted to disk.
        jniLibs { useLegacyPackaging = true }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
}
