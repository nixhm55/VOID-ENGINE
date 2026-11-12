#!/usr/bin/env bash
set -euo pipefail

apk_path="${1:-target/release/apk/void.apk}"
sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
build_tools="${ANDROID_BUILD_TOOLS:-34.0.0}"

if [[ -z "${sdk_root}" ]]; then
  echo "ANDROID_HOME or ANDROID_SDK_ROOT must point to the Android SDK" >&2
  exit 1
fi

android_jar="${sdk_root}/platforms/android-33/android.jar"
tools_dir="${sdk_root}/build-tools/${build_tools}"
java_source="src/android/java/com/mdnm/voidengine/VoidActivity.java"
bluetooth_source="src/android/java/com/mdnm/voidengine/VoidBluetoothBridge.java"
engine_profile="${ENGINE_PROFILE:-${APP_PROFILE:-lite}}"
keystore="${CARGO_APK_RELEASE_KEYSTORE:-${HOME}/.android/debug.keystore}"
keystore_password="${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:-android}"

for required in "${apk_path}" "${android_jar}" "${tools_dir}/d8" \
    "${tools_dir}/zipalign" "${tools_dir}/apksigner" "${java_source}" "${keystore}"; do
  if [[ ! -e "${required}" ]]; then
    echo "Required Android packaging file not found: ${required}" >&2
    exit 1
  fi
done
if [[ "$engine_profile" != "lite" && "$engine_profile" != "max" ]]; then
  echo "ENGINE_PROFILE or APP_PROFILE must be lite or max" >&2
  exit 1
fi
if [[ "$engine_profile" == "max" && ! -f "$bluetooth_source" ]]; then
  echo "The Max profile requires the Android Bluetooth bridge source: $bluetooth_source" >&2
  exit 1
fi

temp_dir="$(mktemp -d)"
trap 'rm -rf "${temp_dir}"' EXIT
mkdir -p "${temp_dir}/classes" "${temp_dir}/dex"

java_sources=("${java_source}")
if [[ "$engine_profile" == "max" ]]; then
  java_sources+=("${bluetooth_source}")
fi
javac -source 8 -target 8 -bootclasspath "${android_jar}" \
  -d "${temp_dir}/classes" "${java_sources[@]}"
jar cf "${temp_dir}/bridge.jar" -C "${temp_dir}/classes" .
"${tools_dir}/d8" --lib "${android_jar}" --min-api 24 \
  --output "${temp_dir}/dex" "${temp_dir}/bridge.jar"

cp "${apk_path}" "${temp_dir}/with-bridge.apk"
(cd "${temp_dir}/dex" && zip -q "${temp_dir}/with-bridge.apk" classes.dex)
"${tools_dir}/zipalign" -f 4 "${temp_dir}/with-bridge.apk" "${temp_dir}/aligned.apk"
"${tools_dir}/apksigner" sign \
  --ks "${keystore}" \
  --ks-pass "pass:${keystore_password}" \
  --key-pass "pass:${keystore_password}" \
  --out "${temp_dir}/signed.apk" \
  "${temp_dir}/aligned.apk"
"${tools_dir}/apksigner" verify "${temp_dir}/signed.apk"
mv "${temp_dir}/signed.apk" "${apk_path}"
