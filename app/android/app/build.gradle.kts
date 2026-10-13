plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.example.inkweave"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.example.inkweave"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
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

// --- Rust host ---------------------------------------------------------------------------------
//
// Cross-compile the Inkweave Rust host and drop the .so files where AGP picks them up, so the APK
// actually contains the kernel instead of only the Flutter shell. The NDK path comes from the value
// AGP already resolved (`ndkVersion` above) rather than an absolute path committed to the repo, and
// `cargo ndk` receives it through ANDROID_NDK_HOME.
// Every ABI the app ships an APK for must be listed here. `--split-per-abi` will happily emit an
// armeabi-v7a APK that contains the Flutter shell and no Rust host at all, and the resulting app
// fails at `RustLib.init()` on a 32-bit device rather than at build time — so the two lists have to
// agree, and this one is the one that decides.
val rustAbis = listOf("arm64-v8a", "armeabi-v7a", "x86_64")

val buildRustHost = tasks.register<Exec>("buildRustHost") {
    group = "build"
    description = "Cross-compiles the Inkweave Rust host for ${rustAbis.joinToString(", ")}"

    // app/android/app -> app/rust
    workingDir = file("${projectDir}/../../rust")
    environment("ANDROID_NDK_HOME", android.ndkDirectory.absolutePath)

    val jniLibs = file("${projectDir}/src/main/jniLibs")
    commandLine(
        listOf("cargo", "ndk") +
            rustAbis.flatMap { listOf("-t", it) } +
            listOf("-o", jniLibs.absolutePath, "build", "-p", "inkweave_bridge", "--release"),
    )
}

// No `outputs` declaration on purpose: the task must run on every build, or a Rust change would
// silently ship the previous .so. Cargo's own incrementality keeps the cost low.
tasks.named("preBuild") { dependsOn(buildRustHost) }
