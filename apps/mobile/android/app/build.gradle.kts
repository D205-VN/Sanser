import java.util.Properties

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

val releaseKey = Properties().apply {
    val propertiesFile = rootProject.file("key.properties")
    if (propertiesFile.exists()) propertiesFile.inputStream().use { load(it) }
}

android {
    namespace = "com.sanser.mobile"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.sanser.mobile"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = 26
        ndk {
            abiFilters.clear()
            abiFilters += "arm64-v8a"
        }
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        externalNativeBuild { cmake { cppFlags += "-std=c++20" } }
    }

    externalNativeBuild {
        cmake { path = file("../../../../native/mobile/CMakeLists.txt"); version = "3.22.1" }
    }

    signingConfigs {
        if (releaseKey.isNotEmpty()) create("sanserRelease") {
            storeFile = rootProject.file(releaseKey.getProperty("storeFile"))
            storePassword = releaseKey.getProperty("storePassword")
            keyAlias = releaseKey.getProperty("keyAlias")
            keyPassword = releaseKey.getProperty("keyPassword")
        }
    }
    buildTypes {
        release {
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (releaseKey.isNotEmpty()) signingConfig = signingConfigs.getByName("sanserRelease")
            // An absent release key produces an unsigned build, never a public
            // update signed with a disposable debug certificate.
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

val buildMobileP2p by tasks.registering(Exec::class) {
    workingDir(rootProject.file("../../.."))
    commandLine("bash", "scripts/build-mobile-p2p.sh", "aarch64-linux-android")
    val localSdk = Properties().apply {
        val local = rootProject.file("local.properties")
        if (local.exists()) local.inputStream().use { load(it) }
    }.getProperty("sdk.dir")
    if (localSdk != null) environment("ANDROID_HOME", localSdk)
}
tasks.configureEach {
    if (name == "preBuild" || name.startsWith("configureCMake") || name.startsWith("buildCMake")) {
        dependsOn(buildMobileP2p)
    }
}

dependencies {
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("com.google.crypto.tink:tink-android:1.18.0")
    testImplementation("junit:junit:4.13.2")
}
