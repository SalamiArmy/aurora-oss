#!/usr/bin/env bash
# Build a release arm64-v8a APK of the game and sign it.
#
#   tools/build-android.sh OUT.apk
#
# Needs: rustup target aarch64-linux-android, `cargo install cargo-apk`, an Android SDK (ANDROID_HOME or
# ANDROID_SDK_ROOT, with build-tools and platforms;android-33), an NDK (ANDROID_NDK_ROOT), and a JDK (keytool).
#
# Signing: if ANDROID_KEYSTORE_B64 (base64 of a .jks/.keystore), ANDROID_KEYSTORE_PASSWORD and ANDROID_KEY_ALIAS are all
# set, the APK is signed with that key (the key password is assumed to equal the keystore password). Otherwise it is
# signed with a freshly generated debug key and says so loudly: such an APK installs, but is not a proper release.
set -euo pipefail
trap 'echo "::error::build-android.sh failed at line $LINENO: $BASH_COMMAND" >&2' ERR

OUT="${1:?usage: tools/build-android.sh OUT.apk}"
cd "$(dirname "$0")/.."
mkdir -p "$(dirname "$OUT")"
OUT="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"

SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[ -n "$SDK" ] || { echo "ANDROID_HOME / ANDROID_SDK_ROOT is not set" >&2; exit 1; }
export ANDROID_HOME="$SDK" ANDROID_SDK_ROOT="$SDK"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --- assets: an APK has no directory layout, so levels/ and assets/ are packed into the APK's assets/ folder
STAGE=crates/au2-android/apk-assets
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -r levels assets "$STAGE/"

# --- the key the final APK gets, and the throwaway one cargo-apk needs in order to produce a signed release APK at all
DEBUG_KS="$TMP/debug.keystore"
keytool -genkeypair -keystore "$DEBUG_KS" -storepass android -keypass android -alias androiddebugkey \
  -dname "CN=Android Debug,O=Android,C=US" -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1
export CARGO_APK_RELEASE_KEYSTORE="$DEBUG_KS"
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android

# --- build
cargo apk build --release -p au2-android
ls -la target/release/apk >&2 || true
BUILT="$(find target/release/apk -maxdepth 1 -name '*.apk' | head -n 1)"
[ -n "$BUILT" ] || { echo "cargo-apk produced no APK under target/release/apk" >&2; exit 1; }

# --- sign
APKSIGNER="$(find "$SDK/build-tools" -name apksigner -type f 2>/dev/null | sort -V | tail -n 1)"
[ -n "$APKSIGNER" ] || { echo "apksigner not found under $SDK/build-tools" >&2; exit 1; }

# Always sign explicitly with apksigner, so the result does not depend on what cargo-apk did with its keystore settings.
if [ -n "${ANDROID_KEYSTORE_B64:-}" ] && [ -n "${ANDROID_KEYSTORE_PASSWORD:-}" ] && [ -n "${ANDROID_KEY_ALIAS:-}" ]; then
  echo "Signing with the release key from the repository secrets (alias: ${ANDROID_KEY_ALIAS})."
  printf '%s' "$ANDROID_KEYSTORE_B64" | base64 -d > "$TMP/release.keystore"
  "$APKSIGNER" sign --ks "$TMP/release.keystore" --ks-key-alias "$ANDROID_KEY_ALIAS"     --ks-pass env:ANDROID_KEYSTORE_PASSWORD --key-pass env:ANDROID_KEYSTORE_PASSWORD --out "$OUT" "$BUILT"
else
  echo "::warning::No release signing secrets (ANDROID_KEYSTORE_B64, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS): signing with the DEBUG key."
  echo "WARNING: this APK is signed with a throwaway debug key, not a release key. It installs, but updates will not"
  echo "         install over an APK signed with a different key."
  "$APKSIGNER" sign --ks "$DEBUG_KS" --ks-key-alias androiddebugkey --ks-pass pass:android --key-pass pass:android     --out "$OUT" "$BUILT"
fi
VERIFY="$("$APKSIGNER" verify --print-certs "$OUT")"
echo "$VERIFY" | head -n 3
echo "wrote $OUT"
