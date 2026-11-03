#!/usr/bin/env bash
# Construit la bibliothèque Rust pour chaque puce de l'application : ARM64 (téléphones, tablettes) et x86_64
# (Chromebook Intel ou AMD, émulateur). Usage : build-native.sh <dossier jniLibs>
set -euo pipefail
SDK=${ANDROID_HOME:-$HOME/Android/Sdk}
NDK=${ANDROID_NDK_HOME:-$(find "$SDK/ndk" -mindepth 1 -maxdepth 1 | sort -V | tail -1)}
BIN=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
export PATH="$BIN:$PATH"
racine=$(cd "$(dirname "$0")/.." && pwd)
# Cible Rust, dossier de l'ABI Android, préfixe du compilateur du NDK (niveau d'API 26, le minSdk).
for puce in "aarch64-linux-android arm64-v8a aarch64-linux-android26" "x86_64-linux-android x86_64 x86_64-linux-android26"; do
  read -r cible abi compilateur <<< "$puce"
  variable=$(echo "$cible" | tr '[:lower:]-' '[:upper:]_')
  export "CARGO_TARGET_${variable}_LINKER=$compilateur-clang"
  export "CC_${cible//-/_}=$compilateur-clang"
  export "AR_${cible//-/_}=llvm-ar"
  cargo build --manifest-path "$racine/Cargo.toml" -p bonk-android --release --target "$cible"
  mkdir -p "$1/$abi"
  cp "$racine/target/$cible/release/libbonk_android.so" "$1/$abi/"
done
