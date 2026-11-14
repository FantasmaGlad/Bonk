import groovy.json.JsonSlurper
import java.awt.RenderingHints
import java.awt.image.BufferedImage
import javax.imageio.ImageIO

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

val jniOutput = layout.buildDirectory.dir("native/jniLibs")
val tokensOutput = layout.buildDirectory.dir("generated/tokens")
val iconsOutput = layout.buildDirectory.dir("generated/icons")
// Only the designated key gets the write test: without this value the button does not exist.
val benchSerial = providers.gradleProperty("benchSerial").getOrElse("")

// The version comes from the workspace: a single one to bump for a release.
val workspaceVersion =
    Regex("""(?m)^version = "(\d+)\.(\d+)\.(\d+)"""")
        .find(rootProject.file("../Cargo.toml").readText())!!
        .groupValues
        .drop(1)
        .map(String::toInt)
val keystore = providers.environmentVariable("BONK_KEYSTORE_FILE").orNull
val keystorePassword = providers.environmentVariable("BONK_KEYSTORE_PASSWORD").orNull

android {
    lint {
        warningsAsErrors = true
        abortOnError = true
        // Shown, never failing: an AndroidX library's own lint rules, targetSdk held until an Android 17
        // device exists, and newer Gradle or library versions, which Dependabot proposes.
        informational +=
            setOf("ObsoleteLintCustomCheck", "OldTargetApi", "AndroidGradlePluginVersion", "NewerVersionAvailable")
    }
    namespace = "app.bonkos.bonk"
    compileSdk = 37

    defaultConfig {
        applicationId = "app.bonkos.bonk"
        minSdk = 26
        targetSdk = 36
        versionCode = workspaceVersion[0] * 10000 + workspaceVersion[1] * 100 + workspaceVersion[2]
        versionName = workspaceVersion.joinToString(".")
        buildConfigField("String", "BENCH_SERIAL", "\"$benchSerial\"")
    }

    signingConfigs {
        if (keystore != null) {
            create("official") {
                storeFile = file(keystore)
                storePassword = keystorePassword
                keyAlias = "bonk"
                keyPassword = keystorePassword
            }
        }
    }

    buildTypes {
        // Compose is very slow in debug. Without the official key, the debug key is enough for tests.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs[if (keystore != null) "official" else "debug"]
        }
    }

    // Without this list AndroidX would announce armv7 and x86, which have no Bonk library.
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "x86_64")
            isUniversalApk = false
        }
    }

    // The language is chosen in Bonk: every install keeps French and English.
    bundle {
        language {
            enableSplit = false
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    sourceSets["main"].jniLibs.directories.add(jniOutput.get().asFile.path)
    sourceSets["main"].kotlin.directories.add(tokensOutput.get().asFile.path)
    sourceSets["main"].res.directories.add(iconsOutput.get().asFile.path)
}

val generateTokens by tasks.registering {
    val input = rootProject.file("../design/tokens.json")
    val output = tokensOutput.get().file("app/bonkos/bonk/Tokens.kt").asFile
    inputs.file(input)
    outputs.file(output)
    doLast {
        @Suppress("UNCHECKED_CAST")
        val tokens = JsonSlurper().parse(input) as Map<String, Any>
        val light = tokens["light"] as Map<String, String>
        val dark = tokens["dark"] as Map<String, String>
        val text = tokens["text"] as Map<String, Int>
        val spacing = tokens["spacing"] as Map<String, Int>
        fun property(name: String) = name.split("_").let { parts ->
            parts.first() + parts.drop(1).joinToString("") { it.replaceFirstChar(Char::uppercase) }
        }
        fun palette(colors: Map<String, String>) =
            colors.entries.joinToString(", ") { (name, hex) -> "${property(name)} = 0xFF${hex.removePrefix("#")}" }
        output.parentFile.mkdirs()
        output.writeText(
            buildString {
                appendLine("package app.bonkos.bonk")
                appendLine("data class Palette(${light.keys.joinToString(", ") { "val ${property(it)}: Long" }})")
                appendLine("object Tokens {")
                appendLine("    val LIGHT = Palette(${palette(light)})")
                appendLine("    val DARK = Palette(${palette(dark)})")
                appendLine("    const val RADIUS = ${tokens["radius"]}")
                text.forEach { (name, size) -> appendLine("    const val TEXT_${name.uppercase()} = $size") }
                spacing.forEach { (name, size) -> appendLine("    const val SPACE_${name.uppercase()} = $size") }
                appendLine("}")
            },
        )
    }
}

val generateIcons by tasks.registering {
    val input = rootProject.file("../crates/bonk/assets/icons/512x512/bonk.png")
    val densities = mapOf("mdpi" to 48, "hdpi" to 72, "xhdpi" to 96, "xxhdpi" to 144, "xxxhdpi" to 192)
    val output = iconsOutput.get().asFile
    inputs.file(input)
    outputs.dir(output)
    doLast {
        val source = ImageIO.read(input)
        densities.forEach { (density, size) ->
            // Halving step by step: a single 512 to 48 step jags the outline.
            var image: BufferedImage = source
            while (image.width / 2 >= size) image = scaled(image, image.width / 2)
            image = scaled(image, size)
            val file = File(output, "mipmap-$density/ic_launcher.png")
            file.parentFile.mkdirs()
            ImageIO.write(image, "png", file)
        }
    }
}

fun scaled(image: BufferedImage, size: Int): BufferedImage {
    val result = BufferedImage(size, size, BufferedImage.TYPE_INT_ARGB)
    val graphics = result.createGraphics()
    graphics.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC)
    graphics.setRenderingHint(RenderingHints.KEY_RENDERING, RenderingHints.VALUE_RENDER_QUALITY)
    graphics.drawImage(image, 0, 0, size, size, null)
    graphics.dispose()
    return result
}

val buildNative by tasks.registering(Exec::class) {
    commandLine(rootProject.file("build-native.sh").path, jniOutput.get().asFile.path)
    environment("BONK_BENCH_SERIAL", benchSerial)
}

tasks.named("preBuild") {
    dependsOn(generateTokens, generateIcons, buildNative)
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.material3:material3")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.13.0")
    testImplementation("org.jetbrains.kotlin:kotlin-test-junit:2.4.21")
}
