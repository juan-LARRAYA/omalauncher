#!/usr/bin/env bash
# Builds mapas-core (Mapas' engine, in Rust) for Android and leaves it in jniLibs, like fetch-proot.sh does with
# proot. Needs rustup or the aarch64-linux-android target, and the same NDK (NDK_VERSION in fetch-proot.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NDK_VERSION=$(sed -n 's/^NDK_VERSION=//p' "$ROOT/scripts/fetch-proot.sh")
bin="$ANDROID_HOME/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"
rustup target add aarch64-linux-android
export CC_aarch64_linux_android="$bin/aarch64-linux-android29-clang"
export AR_aarch64_linux_android="$bin/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
cd "$ROOT/mapas-core"
cargo build --release --locked --lib --target aarch64-linux-android
mkdir -p "$ROOT/app/src/main/jniLibs/arm64-v8a"
cp target/aarch64-linux-android/release/libmapas_core.so "$ROOT/app/src/main/jniLibs/arm64-v8a/"
