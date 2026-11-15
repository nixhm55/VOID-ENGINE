#!/usr/bin/env bash
set -euo pipefail

template_apk="${1:?usage: package-android-aab.sh TEMPLATE.apk OUTPUT.aab}"
output_aab="${2:?usage: package-android-aab.sh TEMPLATE.apk OUTPUT.aab}"
sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/usr/local/lib/android/sdk}}"
build_tools="${ANDROID_BUILD_TOOLS:-34.0.0}"
app_name="${APP_NAME:-VOID}"
app_slug="${APP_SLUG:-void}"
app_package="${ANDROID_PACKAGE:-com.void.void_engine}"
app_version="${APP_VERSION:-1.0.0}"
keystore="${CARGO_APK_RELEASE_KEYSTORE:-${HOME}/.android/debug.keystore}"
keystore_password="${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:-android}"

tools_dir="${sdk_root}/build-tools/${build_tools}"
if [[ ! -x "${tools_dir}/aapt2" ]]; then
  latest_bt="$(find "${sdk_root}/build-tools" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -n 1 || true)"
  if [[ -n "$latest_bt" && -x "${latest_bt}/aapt2" ]]; then
    tools_dir="$latest_bt"
  else
    echo "Missing Android build-tools (aapt2) under ${sdk_root}/build-tools" >&2
    exit 1
  fi
fi

[[ -f "$template_apk" && -f void.json && -f branding/icon.png ]] || {
  echo "Template APK, void.json, or branding/icon.png is missing" >&2
  exit 1
}

mkdir -p "$(dirname "$output_aab")"
temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT
repacked_apk="${temp_dir}/repacked.apk"
proto_apk="${temp_dir}/proto.apk"
unsigned_aab="${temp_dir}/unsigned.aab"

# Step 1: Patch binary AndroidManifest.xml, void.json, and mipmap icons inside the template APK ZIP
python3 - "$template_apk" "$repacked_apk" "$app_package" "$app_name" "$app_version" <<'PY'
import pathlib
import struct
import sys
import zipfile

template_apk, repacked_apk, package_name, app_name, app_version = sys.argv[1:]
void_json_bytes = pathlib.Path("void.json").read_bytes()
icon_bytes = pathlib.Path("branding/icon.png").read_bytes()

def patch_axml(data: bytes, pkg: str, label: str, ver: str) -> bytes:
    if len(data) < 36:
        return data
    magic, _ = struct.unpack_from("<II", data, 0)
    if magic != 0x00080003:
        return data
    sp_type, sp_hdr_size, sp_size, str_count, style_count, flags, strings_start, _ = struct.unpack_from("<HHIIIIII", data, 8)
    if sp_type != 0x0001 or style_count != 0:
        return data
    is_utf8 = bool(flags & 0x100)
    offsets = [struct.unpack_from("<I", data, 8 + sp_hdr_size + i * 4)[0] for i in range(str_count)]
    pool_data_start = 8 + strings_start
    strings = []
    for off in offsets:
        pos = pool_data_start + off
        if is_utf8:
            c_len = data[pos]; pos += 1
            if c_len & 0x80:
                c_len = ((c_len & 0x7f) << 8) | data[pos]; pos += 1
            b_len = data[pos]; pos += 1
            if b_len & 0x80:
                b_len = ((b_len & 0x7f) << 8) | data[pos]; pos += 1
            strings.append(data[pos:pos + b_len].decode("utf-8", errors="replace"))
        else:
            c_len = struct.unpack_from("<H", data, pos)[0]; pos += 2
            if c_len & 0x8000:
                low = struct.unpack_from("<H", data, pos)[0]; pos += 2
                c_len = ((c_len & 0x7fff) << 16) | low
            strings.append(data[pos:pos + c_len * 2].decode("utf-16le", errors="replace"))

    def add_or_get(s: str) -> int:
        if s in strings:
            return strings.index(s)
        strings.append(s)
        return len(strings) - 1

    pkg_idx = add_or_get(pkg)
    label_idx = add_or_get(label)
    ver_idx = add_or_get(ver)

    tail = bytearray(data[8 + sp_size:])
    pos = 0
    while pos + 8 <= len(tail):
        c_type, c_hdr_size, c_size = struct.unpack_from("<HHI", tail, pos)
        if c_size < 8 or pos + c_size > len(tail):
            break
        if c_type == 0x0102 and c_hdr_size >= 16 and pos + 36 <= len(tail):
            ns_idx, name_idx, attr_start, attr_size, attr_count = struct.unpack_from("<iiHHH", tail, pos + 16)
            elem_name = strings[name_idx] if 0 <= name_idx < len(strings) else ""
            attr_base = pos + 16 + attr_start
            for a in range(attr_count):
                a_off = attr_base + a * attr_size
                if a_off + 20 > pos + c_size:
                    break
                a_ns, a_name, a_raw, tv_size, tv_zero, tv_type, tv_data = struct.unpack_from("<iiiHBBI", tail, a_off)
                attr_name = strings[a_name] if 0 <= a_name < len(strings) else ""
                if elem_name == "manifest" and attr_name == "package":
                    struct.pack_into("<iiiHBBI", tail, a_off, a_ns, a_name, pkg_idx, tv_size, tv_zero, 0x03, pkg_idx)
                elif elem_name == "manifest" and attr_name == "versionName":
                    struct.pack_into("<iiiHBBI", tail, a_off, a_ns, a_name, ver_idx, tv_size, tv_zero, 0x03, ver_idx)
                elif elem_name in ("application", "activity") and attr_name == "label":
                    struct.pack_into("<iiiHBBI", tail, a_off, a_ns, a_name, label_idx, tv_size, tv_zero, 0x03, label_idx)
        pos += c_size

    encoded_blobs = []
    new_offsets = []
    cur_off = 0
    for s in strings:
        new_offsets.append(cur_off)
        if is_utf8:
            u16_len = len(s.encode("utf-16le")) // 2
            b = s.encode("utf-8")
            b_len = len(b)
            hdr = bytearray()
            if u16_len > 0x7f:
                hdr.extend([(u16_len >> 8) | 0x80, u16_len & 0xff])
            else:
                hdr.append(u16_len)
            if b_len > 0x7f:
                hdr.extend([(b_len >> 8) | 0x80, b_len & 0xff])
            else:
                hdr.append(b_len)
            blob = bytes(hdr) + b + b"\x00"
        else:
            b = s.encode("utf-16le")
            u16_len = len(b) // 2
            if u16_len > 0x7fff:
                hdr = struct.pack("<HH", (u16_len >> 16) | 0x8000, u16_len & 0xffff)
            else:
                hdr = struct.pack("<H", u16_len)
            blob = hdr + b + b"\x00\x00"
        encoded_blobs.append(blob)
        cur_off += len(blob)

    pool_bytes = bytearray(b"".join(encoded_blobs))
    while len(pool_bytes) % 4 != 0:
        pool_bytes.append(0)
    new_str_count = len(strings)
    new_strings_start = sp_hdr_size + new_str_count * 4
    new_sp_size = new_strings_start + len(pool_bytes)
    sp_hdr = struct.pack("<HHIIIIII", 0x0001, sp_hdr_size, new_sp_size, new_str_count, 0, flags, new_strings_start, 0)
    off_table = b"".join(struct.pack("<I", o) for o in new_offsets)
    new_total_size = 8 + new_sp_size + len(tail)
    file_hdr = struct.pack("<II", 0x00080003, new_total_size)
    return file_hdr + sp_hdr + off_table + bytes(pool_bytes) + bytes(tail)

mipmap_targets = {
    f"res/mipmap-{d}/{name}.png"
    for d in ("mdpi", "hdpi", "xhdpi", "xxhdpi", "xxxhdpi")
    for name in ("ic_launcher", "ic_launcher_round")
}
written = set()

with zipfile.ZipFile(template_apk, "r") as zin, zipfile.ZipFile(repacked_apk, "w") as zout:
    for info in zin.infolist():
        fn = info.filename
        if fn.startswith("META-INF/") and (fn.endswith((".SF", ".RSA", ".DSA", ".EC")) or fn == "META-INF/MANIFEST.MF"):
            continue
        if fn.startswith("res/mipmap-anydpi"):
            continue
        raw = zin.read(fn)
        out_info = zipfile.ZipInfo(filename=fn, date_time=(2026, 1, 1, 0, 0, 0))
        out_info.external_attr = info.external_attr
        if fn == "resources.arsc":
            out_info.compress_type = zipfile.ZIP_STORED
            zout.writestr(out_info, raw)
        elif fn == "AndroidManifest.xml":
            out_info.compress_type = zipfile.ZIP_DEFLATED
            zout.writestr(out_info, patch_axml(raw, package_name, app_name, app_version))
        elif fn == "assets/void.json":
            out_info.compress_type = zipfile.ZIP_DEFLATED
            zout.writestr(out_info, void_json_bytes)
        elif fn in mipmap_targets or (fn.startswith("res/mipmap") and fn.endswith(".png")):
            out_info.compress_type = zipfile.ZIP_STORED
            zout.writestr(out_info, icon_bytes)
        else:
            out_info.compress_type = info.compress_type
            zout.writestr(out_info, raw)
        written.add(fn)

    if "assets/void.json" not in written:
        zi = zipfile.ZipInfo("assets/void.json", date_time=(2026, 1, 1, 0, 0, 0))
        zi.compress_type = zipfile.ZIP_DEFLATED
        zout.writestr(zi, void_json_bytes)

    for target in sorted(mipmap_targets):
        if target not in written:
            zi = zipfile.ZipInfo(target, date_time=(2026, 1, 1, 0, 0, 0))
            zi.compress_type = zipfile.ZIP_STORED
            zout.writestr(zi, icon_bytes)
PY

# Step 2: Convert binary AXML + resources.arsc to Protobuf format via native C++ aapt2 (~30ms)
"${tools_dir}/aapt2" convert --output-format proto -o "$proto_apk" "$repacked_apk"

# Step 3: Package into standard Android App Bundle (.aab) structure with BundleConfig.pb
python3 - "$proto_apk" "$unsigned_aab" <<'PY'
import sys
import zipfile

proto_apk, unsigned_aab = sys.argv[1:]

# Standard Protobuf BundleConfig message:
# bundletool { version: "1.17.2" } -> field 1 (0x0a), len 8, field 1 (0x0a), len 6, "1.17.2"
bundle_config_pb = b"\x0a\x08\x0a\x061.17.2"

with zipfile.ZipFile(proto_apk, "r") as zin, zipfile.ZipFile(unsigned_aab, "w", compression=zipfile.ZIP_DEFLATED) as zout:
    bc_info = zipfile.ZipInfo("BundleConfig.pb", date_time=(2026, 1, 1, 0, 0, 0))
    bc_info.compress_type = zipfile.ZIP_DEFLATED
    zout.writestr(bc_info, bundle_config_pb)

    for info in zin.infolist():
        fn = info.filename
        if fn.startswith("META-INF/"):
            continue
        raw = zin.read(fn)
        if fn == "AndroidManifest.xml":
            dest = "base/manifest/AndroidManifest.xml"
        elif fn == "resources.pb":
            dest = "base/resources.pb"
        elif fn.startswith("classes") and fn.endswith(".dex"):
            dest = f"base/dex/{fn}"
        elif fn.startswith(("res/", "assets/", "lib/")):
            dest = f"base/{fn}"
        else:
            dest = f"base/root/{fn}"

        out_info = zipfile.ZipInfo(dest, date_time=(2026, 1, 1, 0, 0, 0))
        out_info.compress_type = zipfile.ZIP_DEFLATED
        out_info.external_attr = info.external_attr
        zout.writestr(out_info, raw)
PY

if [[ ! -f "$keystore" ]]; then
  mkdir -p "$(dirname "$keystore")"
  keytool -genkeypair -keystore "$keystore" -storepass "$keystore_password" \
    -alias androiddebugkey -keypass "$keystore_password" -keyalg RSA -keysize 2048 \
    -validity 10000 -dname 'CN=Android Debug,O=Android,C=US' -noprompt
fi

if command -v jarsigner >/dev/null 2>&1; then
  jarsigner -keystore "$keystore" -storepass "$keystore_password" -keypass "$keystore_password" \
    -sigalg SHA256withRSA -digestalg SHA-256 "$unsigned_aab" androiddebugkey >/dev/null
  mv "$unsigned_aab" "$output_aab"
else
  "${tools_dir}/apksigner" sign --ks "$keystore" --ks-pass "pass:${keystore_password}" \
    --key-pass "pass:${keystore_password}" --min-sdk-version 24 --out "$output_aab" "$unsigned_aab"
fi

echo "Packaged ${app_name} (${app_package}) as ${output_aab}"
