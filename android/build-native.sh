#!/usr/bin/env bash
# Construit la bibliothèque Rust pour le téléphone. Usage : build-native.sh <dossier jniLibs>
set -euo pipefail
SDK=${ANDROID_HOME:-$HOME/Android/Sdk}
NDK=${ANDROID_NDK_HOME:-$(find "$SDK/ndk" -mindepth 1 -maxdepth 1 | sort -V | tail -1)}
BIN=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
export PATH="$BIN:$PATH"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android26-clang
export CC_aarch64_linux_android=aarch64-linux-android26-clang
export AR_aarch64_linux_android=llvm-ar
racine=$(cd "$(dirname "$0")/.." && pwd)
cargo build --manifest-path "$racine/Cargo.toml" -p bonk-android --release --target aarch64-linux-android
mkdir -p "$1/arm64-v8a"
cp "$racine/target/aarch64-linux-android/release/libbonk_android.so" "$1/arm64-v8a/"
