#!/usr/bin/env bash
set -euo pipefail

# Build and deploy a rinch Android app to a connected device/emulator.
#
# Usage:
#   ./build-apk.sh              # build, package, install, launch
#   ./build-apk.sh --build-only # build and package only (no install)
#   ./build-apk.sh --target arm64-v8a  # build for ARM64 (default: x86_64)
#
# Requirements:
#   - ANDROID_NDK_HOME set (or ~/android/android-ndk-r27c)
#   - Android SDK build-tools installed (or ~/android/sdk/build-tools/35.0.0)
#   - cargo-ndk installed (cargo install cargo-ndk)
#   - adb available for install/launch

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Defaults
TARGET="x86_64"
ABI="x86_64"
BUILD_ONLY=false
RELEASE=true

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --target) TARGET="$2"; shift 2 ;;
        --debug) RELEASE=false; shift ;;
        --build-only) BUILD_ONLY=true; shift ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# Map cargo-ndk target to ABI directory name
case "$TARGET" in
    x86_64)       ABI="x86_64" ;;
    arm64-v8a)    ABI="arm64-v8a" ;;
    armeabi-v7a)  ABI="armeabi-v7a" ;;
    x86)          ABI="x86" ;;
    *) echo "Unknown target: $TARGET"; exit 1 ;;
esac

# Find tools
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/android/android-ndk-r27c}"
BUILD_TOOLS="${ANDROID_SDK_BUILD_TOOLS:-$HOME/android/sdk/build-tools/35.0.0}"
PLATFORM="${ANDROID_SDK_PLATFORM:-$HOME/android/sdk/platforms/android-35/android.jar}"
KEYSTORE="${ANDROID_DEBUG_KEYSTORE:-/tmp/debug.keystore}"

if [[ ! -d "$ANDROID_NDK_HOME" ]]; then
    echo "ERROR: ANDROID_NDK_HOME not found at $ANDROID_NDK_HOME"
    exit 1
fi

# Cargo target triple
case "$TARGET" in
    x86_64)       TRIPLE="x86_64-linux-android" ;;
    arm64-v8a)    TRIPLE="aarch64-linux-android" ;;
    armeabi-v7a)  TRIPLE="armv7-linux-androideabi" ;;
    x86)          TRIPLE="i686-linux-android" ;;
esac

PROFILE="release"
PROFILE_FLAG="--release"
if [[ "$RELEASE" == false ]]; then
    PROFILE="debug"
    PROFILE_FLAG=""
fi

# Build
echo "==> Building for $TARGET ($PROFILE)..."
export ANDROID_NDK_HOME
cargo ndk -t "$TARGET" build -p hello-android $PROFILE_FLAG

SO_PATH="$PROJECT_ROOT/target/$TRIPLE/$PROFILE/libhello_android.so"
if [[ ! -f "$SO_PATH" ]]; then
    echo "ERROR: .so not found at $SO_PATH"
    exit 1
fi

# Compile Java companion classes (RinchActivity, RinchInputConnection)
RINCH_ANDROID="$PROJECT_ROOT/crates/rinch-android"
JAVA_SRC="$RINCH_ANDROID/java"

echo "==> Compiling Java..."
APK_DIR="$(mktemp -d)"
trap "rm -rf $APK_DIR" EXIT

mkdir -p "$APK_DIR/classes"
javac -source 8 -target 8 \
    -classpath "$PLATFORM" \
    -d "$APK_DIR/classes" \
    "$JAVA_SRC/com/rinch/RinchActivity.java" \
    "$JAVA_SRC/com/rinch/RinchInputConnection.java" \
    "$JAVA_SRC/com/rinch/RinchInputView.java"

echo "==> Converting to DEX..."
"$BUILD_TOOLS/d8" --output "$APK_DIR/" \
    $(find "$APK_DIR/classes" -name "*.class")

# Package APK
echo "==> Packaging APK..."
mkdir -p "$APK_DIR/lib/$ABI"
cp "$SO_PATH" "$APK_DIR/lib/$ABI/"

"$BUILD_TOOLS/aapt2" link \
    --manifest "$SCRIPT_DIR/android/AndroidManifest.xml" \
    -I "$PLATFORM" \
    --min-sdk-version 28 \
    --target-sdk-version 35 \
    -o "$APK_DIR/base.apk"

(cd "$APK_DIR" && zip -qr base.apk lib/ classes.dex)

"$BUILD_TOOLS/zipalign" -f 4 "$APK_DIR/base.apk" "$APK_DIR/aligned.apk"

# Create debug keystore if needed
if [[ ! -f "$KEYSTORE" ]]; then
    keytool -genkeypair \
        -keystore "$KEYSTORE" -alias debug \
        -keyalg RSA -keysize 2048 -validity 10000 \
        -storepass android -keypass android \
        -dname "CN=Debug, O=Debug" 2>/dev/null
fi

"$BUILD_TOOLS/apksigner" sign \
    --ks "$KEYSTORE" --ks-key-alias debug \
    --ks-pass pass:android --key-pass pass:android \
    --out "$APK_DIR/hello-rinch.apk" \
    "$APK_DIR/aligned.apk" 2>/dev/null

APK_SIZE=$(du -h "$APK_DIR/hello-rinch.apk" | cut -f1)
echo "==> APK ready: $APK_SIZE"

if [[ "$BUILD_ONLY" == true ]]; then
    cp "$APK_DIR/hello-rinch.apk" "$SCRIPT_DIR/hello-rinch.apk"
    echo "==> Saved to $SCRIPT_DIR/hello-rinch.apk"
    exit 0
fi

# Install and launch
echo "==> Installing..."
adb shell am force-stop com.rinch.hello 2>/dev/null || true
adb install -r "$APK_DIR/hello-rinch.apk" 2>&1 | grep -E "Success|Failure"

echo "==> Launching..."
adb shell am start -n com.rinch.hello/com.rinch.RinchActivity

echo "==> Done. Use 'adb logcat -s rinch' for logs."
