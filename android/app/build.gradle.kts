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

    // One fixed signing key, so every update installs over the previous one. CI supplies it through
    // environment variables (from GitHub secrets); without them the build falls back to the temporary
    // debug key. The key file is never in the repository.
    val keystoreFile = System.getenv("PULSE_KEYSTORE_FILE")
    val keystorePassword = System.getenv("PULSE_KEYSTORE_PASSWORD")
    if (!keystoreFile.isNullOrBlank() && !keystorePassword.isNullOrBlank() && file(keystoreFile).isFile) {
        signingConfigs {
            create("pulse") {
                storeFile = file(keystoreFile)
                storePassword = keystorePassword
                keyAlias = "pulse"
                keyPassword = keystorePassword
            }
        }
        buildTypes.getByName("debug").signingConfig = signingConfigs.getByName("pulse")
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
    // Privileged mode: talks to the Shizuku app (optional, the user installs and starts it).
    implementation("dev.rikka.shizuku:api:13.1.5")
    implementation("dev.rikka.shizuku:provider:13.1.5")
}
