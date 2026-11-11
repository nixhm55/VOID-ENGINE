#!/usr/bin/env bash
set -euo pipefail

template_apk="${1:?usage: package-android-master.sh TEMPLATE.apk OUTPUT.apk}"
output_apk="${2:?usage: package-android-master.sh TEMPLATE.apk OUTPUT.apk}"
apktool_version="3.0.3"
apktool_sha256="dbf930b076c6b9be08d57c449cacefc3bdd6b71ebd59b3066fc0e1f5b14f9423"
apktool_jar="${APKTOOL_JAR:-.cache/apktool-${apktool_version}.jar}"
sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
build_tools="${ANDROID_BUILD_TOOLS:-34.0.0}"
app_name="${APP_NAME:-VOID}"
app_slug="${APP_SLUG:-void}"
app_package="${ANDROID_PACKAGE:-com.void.void_engine}"
app_version="${APP_VERSION:-1.0.0}"
keystore="${CARGO_APK_RELEASE_KEYSTORE:-${HOME}/.android/debug.keystore}"
keystore_password="${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:-android}"

if [[ -z "$sdk_root" ]]; then
  echo "ANDROID_HOME or ANDROID_SDK_ROOT must point to the Android SDK" >&2
  exit 1
fi
tools_dir="${sdk_root}/build-tools/${build_tools}"
for tool in java curl sha256sum python3; do command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 1; }; done
for tool in zipalign apksigner; do [[ -x "${tools_dir}/${tool}" ]] || { echo "Missing Android tool: ${tools_dir}/${tool}" >&2; exit 1; }; done
android_jar="${sdk_root}/platforms/android-33/android.jar"
[[ -f "$android_jar" ]] || { echo "Missing Android framework resources: $android_jar" >&2; exit 1; }
[[ -f "$template_apk" && -f void.json && -f branding/icon.png ]] || { echo "Template APK, void.json, or branding/icon.png is missing" >&2; exit 1; }

mkdir -p "$(dirname "$apktool_jar")" "$(dirname "$output_apk")" .cache
if [[ ! -f "$apktool_jar" ]] || [[ "$(sha256sum "$apktool_jar" | awk '{print $1}')" != "$apktool_sha256" ]]; then
  curl --fail --location --silent --show-error \
    "https://github.com/iBotPeaches/Apktool/releases/download/v${apktool_version}/apktool_${apktool_version}.jar" \
    --output "${apktool_jar}.download"
  echo "${apktool_sha256}  ${apktool_jar}.download" | sha256sum --check --status || { rm -f "${apktool_jar}.download"; echo "Apktool checksum verification failed" >&2; exit 1; }
  mv "${apktool_jar}.download" "$apktool_jar"
fi

temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT
decoded="${temp_dir}/decoded"
framework_dir="${temp_dir}/framework"
mkdir -p "$framework_dir"
# Use this SDK's platform resources explicitly so Apktool's AAPT2 link always
# receives framework package 1, independent of runner-global Apktool state.
java -jar "$apktool_jar" if "$android_jar" --frame-path "$framework_dir"
java -jar "$apktool_jar" d --force --frame-path "$framework_dir" "$template_apk" --output "$decoded"

python3 - "$decoded/AndroidManifest.xml" "$app_package" "$app_name" "$app_version" <<'PY'
import sys
import xml.etree.ElementTree as ET

manifest_path, package_name, app_name, app_version = sys.argv[1:]
android = "http://schemas.android.com/apk/res/android"
ET.register_namespace("android", android)
tree = ET.parse(manifest_path)
root = tree.getroot()
root.set("package", package_name)
root.set("{%s}versionName" % android, app_version)
application = root.find("application")
if application is None:
    raise SystemExit("Android manifest has no application element")
application.set("{%s}label" % android, app_name)
# Keep the framework theme reference fully qualified. If it is emitted as an
# app-local @style reference, AAPT2 looks for a nonexistent app resource.
application.set("{%s}theme" % android, "@android:style/Theme.Material.NoActionBar")
tree.write(manifest_path, encoding="utf-8", xml_declaration=True)
PY

# Ensure the rebuild links AndroidManifest.xml against framework package 1.
# Edit only this small Apktool metadata block using Python's standard library.
python3 - "$decoded/apktool.yml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
lines = path.read_text(encoding="utf-8").splitlines()
start = next((i for i, line in enumerate(lines) if line == "usesFramework:"), None)
if start is None:
    lines.extend(["", "usesFramework:", "  ids:", "  - 1", "  tag: null"])
else:
    end = start + 1
    while end < len(lines) and (not lines[end] or lines[end][0].isspace()):
        end += 1
    block = lines[start + 1:end]
    ids = []
    for line in block:
        match = re.match(r"^\s*-\s*(\d+)\s*$", line)
        if match and match.group(1) not in ids:
            ids.append(match.group(1))
    if "1" not in ids:
        ids.insert(0, "1")
    tag = next((line.split(":", 1)[1].strip() for line in block
                if re.match(r"^\s*tag\s*:", line)), "null")
    lines[start:end] = ["usesFramework:", "  ids:"] + [f"  - {value}" for value in ids] + [f"  tag: {tag}"]
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY

mkdir -p "$decoded/assets"
cp void.json "$decoded/assets/void.json"
for density in mdpi hdpi xhdpi xxhdpi xxxhdpi; do
  dir="$decoded/res/mipmap-${density}"
  mkdir -p "$dir"
  cp branding/icon.png "$dir/ic_launcher.png"
  cp branding/icon.png "$dir/ic_launcher_round.png"
done
rm -rf "$decoded/res/mipmap-anydpi-v26"

rebuilt="${temp_dir}/rebuilt.apk"
java -jar "$apktool_jar" b --frame-path "$framework_dir" "$decoded" --output "$rebuilt"
if [[ ! -f "$keystore" ]]; then
  mkdir -p "$(dirname "$keystore")"
  keytool -genkeypair -keystore "$keystore" -storepass "$keystore_password" \
    -alias androiddebugkey -keypass "$keystore_password" -keyalg RSA -keysize 2048 \
    -validity 10000 -dname 'CN=Android Debug,O=Android,C=US' -noprompt
fi
aligned="${temp_dir}/aligned.apk"
signed="${temp_dir}/signed.apk"
"${tools_dir}/zipalign" -f 4 "$rebuilt" "$aligned"
"${tools_dir}/apksigner" sign --ks "$keystore" --ks-pass "pass:${keystore_password}" \
  --key-pass "pass:${keystore_password}" --out "$signed" "$aligned"
"${tools_dir}/apksigner" verify "$signed"
mv "$signed" "$output_apk"
echo "Packaged ${app_name} (${app_package}) as ${output_apk}"
