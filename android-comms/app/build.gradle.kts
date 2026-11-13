plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Release signing material is never committed. It is supplied at build time
// through the environment (see docs/android-release-build.md); when any of the
// four values is missing the release build stays unsigned so local debug work
// and pull-request builds still succeed.
val releaseStoreFile = System.getenv("KEYSTORE_FILE")
val releaseStorePassword = System.getenv("KEYSTORE_PASSWORD")
val releaseKeyAlias = System.getenv("KEY_ALIAS")
val releaseKeyPassword = System.getenv("KEY_PASSWORD")
val hasReleaseSigning =
    !releaseStoreFile.isNullOrBlank() &&
        !releaseStorePassword.isNullOrBlank() &&
        !releaseKeyAlias.isNullOrBlank() &&
        !releaseKeyPassword.isNullOrBlank()

android {
    namespace = "io.kovanica.comms"
    compileSdk = 34

    defaultConfig {
        applicationId = "io.kovanica.comms"
        minSdk = 24
        targetSdk = 34
        versionCode = 2
        versionName = "0.2.0"
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = file(releaseStoreFile!!)
                storePassword = releaseStorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    buildFeatures {
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    sourceSets {
        getByName("main") {
            java.srcDir(projectDir.resolve("../../android-chain/src/main/kotlin"))
        }
        getByName("test") {
            java.srcDir(projectDir.resolve("../../android-chain/src/test/kotlin"))
        }
    }
}

dependencies {
    implementation("net.java.dev.jna:jna:5.14.0@aar")
    testImplementation("junit:junit:4.13.2")
}
