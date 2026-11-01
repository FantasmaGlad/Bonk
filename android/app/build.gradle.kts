import groovy.json.JsonSlurper

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

val jniOutput = layout.buildDirectory.dir("native/jniLibs")
val tokensOutput = layout.buildDirectory.dir("generated/tokens")
// Seule la clé désignée reçoit l'essai d'écriture : sans cette valeur, le bouton n'existe pas.
val benchSerial = providers.gradleProperty("benchSerial").getOrElse("")

// La version vient du workspace : une seule à relever pour une release.
val workspaceVersion =
    Regex("""(?m)^version = "(\d+)\.(\d+)\.(\d+)"""")
        .find(rootProject.file("../Cargo.toml").readText())!!
        .groupValues
        .drop(1)
        .map(String::toInt)
val keystore = providers.environmentVariable("BONK_KEYSTORE_FILE").orNull
val keystorePassword = providers.environmentVariable("BONK_KEYSTORE_PASSWORD").orNull

android {
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
        // Compose est très lent en débogage. Sans la clé officielle, la clé de débogage suffit aux essais.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs[if (keystore != null) "official" else "debug"]
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    sourceSets["main"].jniLibs.directories.add(jniOutput.get().asFile.path)
    sourceSets["main"].kotlin.directories.add(tokensOutput.get().asFile.path)
}

// La charte vient du même fichier que celle de la fenêtre de bureau.
val generateTokens by tasks.registering {
    val input = rootProject.file("../design/tokens.json")
    val output = tokensOutput.get().file("app/bonkos/bonk/Tokens.kt").asFile
    inputs.file(input)
    outputs.file(output)
    doLast {
        @Suppress("UNCHECKED_CAST")
        val tokens = JsonSlurper().parse(input) as Map<String, Any>
        val colors = tokens["color"] as Map<String, String>
        val text = tokens["text"] as Map<String, Int>
        val spacing = tokens["spacing"] as Map<String, Int>
        output.parentFile.mkdirs()
        output.writeText(
            buildString {
                appendLine("package app.bonkos.bonk")
                appendLine("object Tokens {")
                colors.forEach { (name, hex) ->
                    appendLine("    const val ${name.uppercase()} = 0xFF${hex.removePrefix("#")}")
                }
                appendLine("    const val RADIUS = ${tokens["radius"]}")
                text.forEach { (name, size) -> appendLine("    const val TEXT_${name.uppercase()} = $size") }
                spacing.forEach { (name, size) -> appendLine("    const val SPACE_${name.uppercase()} = $size") }
                appendLine("}")
            },
        )
    }
}

val buildNative by tasks.registering(Exec::class) {
    commandLine(rootProject.file("build-native.sh").path, jniOutput.get().asFile.path)
}

tasks.named("preBuild") {
    dependsOn(generateTokens, buildNative)
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.material3:material3")
}
