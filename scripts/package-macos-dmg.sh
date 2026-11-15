#!/usr/bin/env bash
set -euo pipefail

app_name="${APP_NAME:-VOID}"
app_slug="${APP_SLUG:-void}"
version="${APP_VERSION:-1.0.0}"
engine_profile="${ENGINE_PROFILE:-lite}"
binary="${MASTER_BINARY_PATH:-target/master/void}"
template_dmg="${MASTER_TEMPLATE_DMG:-target/master/void-template.dmg}"
entitlements="macos/void.entitlements"
bg_source="branding/installer-bg.tiff"
[[ -f "$bg_source" ]] || bg_source="branding/installer-bg.png"
output_dir="output"
staging="dmg_staging"
app_bundle_name="${app_slug}.app"
app_path="${staging}/${app_bundle_name}"
contents="${app_path}/Contents"

rm -rf "$staging" "$output_dir"
mkdir -p "${contents}/MacOS" "${contents}/Resources/AppIcon.iconset" "$output_dir"

if [[ ! -f branding/icon.png ]]; then
  node scripts/prepare-branding.mjs
fi

iconset="${contents}/Resources/AppIcon.iconset"
for size in 16 32 128 256 512; do
  sips -s format png -z "$size" "$size" branding/icon.png --out "${iconset}/icon_${size}x${size}.png" >/dev/null
done
sips -s format png -z 32 32 branding/icon.png --out "${iconset}/icon_16x16@2x.png" >/dev/null
sips -s format png -z 64 64 branding/icon.png --out "${iconset}/icon_32x32@2x.png" >/dev/null
sips -s format png -z 256 256 branding/icon.png --out "${iconset}/icon_128x128@2x.png" >/dev/null
sips -s format png -z 512 512 branding/icon.png --out "${iconset}/icon_256x256@2x.png" >/dev/null
sips -s format png -z 1024 1024 branding/icon.png --out "${iconset}/icon_512x512@2x.png" >/dev/null

app_icon_icns="${contents}/Resources/AppIcon.icns"
if ! iconutil -c icns "$iconset" -o "$app_icon_icns" 2>/dev/null; then
  python3 - "$iconset" "$app_icon_icns" <<'PY'
import pathlib, struct, sys

iconset_dir = pathlib.Path(sys.argv[1])
out_icns = pathlib.Path(sys.argv[2])
chunks = []
for ostype, fname in [
    (b"icp4", "icon_16x16.png"),
    (b"ic11", "icon_16x16@2x.png"),
    (b"icp5", "icon_32x32.png"),
    (b"ic12", "icon_32x32@2x.png"),
    (b"ic07", "icon_128x128.png"),
    (b"ic13", "icon_128x128@2x.png"),
    (b"ic08", "icon_256x256.png"),
    (b"ic14", "icon_256x256@2x.png"),
    (b"ic09", "icon_512x512.png"),
    (b"ic10", "icon_512x512@2x.png"),
]:
    p = iconset_dir / fname
    if p.is_file():
        data = p.read_bytes()
        chunks.append(ostype + struct.pack(">I", 8 + len(data)) + data)
body = b"".join(chunks)
out_icns.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)
PY
fi
rm -rf "$iconset"

cp "$binary" "${contents}/MacOS/void"
chmod 0755 "${contents}/MacOS/void"
cp void.json "${contents}/Resources/void.json"
cp void.json "${contents}/MacOS/void.json"

python3 - "${contents}/Info.plist" "$app_name" "$app_slug" "$version" "$engine_profile" <<'PY'
import pathlib, sys
from xml.sax.saxutils import escape

plist_path, app_name, app_slug, version, profile = sys.argv[1:]
extra = ""
if profile == "max":
    extra = (
        "<key>NSLocationUsageDescription</key><string>VOID uses location for websites you choose to use.</string>\n"
        "<key>NSBluetoothAlwaysUsageDescription</key><string>VOID connects to Bluetooth devices when you choose to use a website feature.</string>\n"
    )
info = f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>void</string>
<key>CFBundleIdentifier</key><string>{escape(f"com.void.{app_slug}")}</string>
<key>CFBundleName</key><string>{escape(app_name)}</string>
<key>CFBundleDisplayName</key><string>{escape(app_name)}</string>
<key>CFBundleIconFile</key><string>AppIcon.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{escape(version)}</string>
<key>CFBundleVersion</key><string>{escape(version)}</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSCameraUsageDescription</key><string>VOID needs camera access for websites you choose to use.</string>
<key>NSMicrophoneUsageDescription</key><string>VOID needs microphone access for websites you choose to use.</string>
{extra}</dict></plist>
"""
pathlib.Path(plist_path).write_text(info, encoding="utf-8")
PY
printf 'APPL????' > "${contents}/PkgInfo"
codesign --force --deep --sign - --entitlements "$entitlements" "$app_path"

rw_dmg="${output_dir}/${app_slug}-rw.dmg"
final_dmg="${output_dir}/${app_slug}.dmg"

if [[ -f "$template_dmg" ]]; then
  cp "$template_dmg" "$rw_dmg"
else
  ln -s /Applications "${staging}/Applications"
  cp "$bg_source" "${staging}/.background.tiff"
  cp "$app_icon_icns" "${staging}/.VolumeIcon.icns"
  chflags hidden "${staging}/.background.tiff" "${staging}/.VolumeIcon.icns" 2>/dev/null || true
  hdiutil create -volname "$app_name" -srcfolder "$staging" -ov -format UDRW -fs HFS+ "$rw_dmg" >/dev/null
fi

mount_dir="$(mktemp -d /tmp/void_dmg_mnt.XXXXXX)"
rmdir "$mount_dir"
mounted=0
cleanup() {
  if [[ "$mounted" -eq 1 ]]; then
    hdiutil detach "$mount_dir" -force -quiet 2>/dev/null || true
  fi
  rm -rf "$mount_dir" "$rw_dmg"
}
trap cleanup EXIT

hdiutil attach -readwrite -noverify -noautoopen -nobrowse -mountpoint "$mount_dir" "$rw_dmg" >/dev/null
mounted=1

if [[ -f "$template_dmg" ]]; then
  rm -rf "${mount_dir}/VOID.app" "${mount_dir}/void.app" "${mount_dir}/${app_bundle_name}"
  cp -R "$app_path" "${mount_dir}/${app_bundle_name}"
  ln -sfn /Applications "${mount_dir}/Applications"
  cp "$bg_source" "${mount_dir}/.background.tiff"
  cp "$app_icon_icns" "${mount_dir}/.VolumeIcon.icns"
  diskutil rename "$mount_dir" "$app_name" >/dev/null 2>&1 || true
fi

chflags hidden "${mount_dir}/.background.tiff" "${mount_dir}/.VolumeIcon.icns" 2>/dev/null || true
if command -v SetFile >/dev/null 2>&1; then
  SetFile -a V "${mount_dir}/.background.tiff" 2>/dev/null || true
  SetFile -a V "${mount_dir}/.VolumeIcon.icns" 2>/dev/null || true
  SetFile -a C "$mount_dir" 2>/dev/null || true
elif [[ -x /usr/bin/SetFile ]]; then
  /usr/bin/SetFile -a V "${mount_dir}/.background.tiff" 2>/dev/null || true
  /usr/bin/SetFile -a V "${mount_dir}/.VolumeIcon.icns" 2>/dev/null || true
  /usr/bin/SetFile -a C "$mount_dir" 2>/dev/null || true
fi

python3 - "$mount_dir" "$app_name" "$app_bundle_name" <<'PY'
import os
import plistlib
import struct
import sys

mount_dir, vol_name, app_bundle_name = sys.argv[1:]
bg_file = os.path.join(mount_dir, ".background.tiff")

vol_stat = os.stat(mount_dir)
bg_stat = os.stat(bg_file)
vol_ctime = getattr(vol_stat, "st_birthtime", vol_stat.st_ctime)
bg_ctime = getattr(bg_stat, "st_birthtime", bg_stat.st_ctime)

def make_hfs_alias_v2(v_name: str, v_ctime_unix: float, f_name: str, parent_cnid: int, file_cnid: int, f_ctime_unix: float, posix_rel: str, vol_mnt: str) -> bytes:
    MAC_EPOCH_OFFSET = 2082844800
    v_ct = int(v_ctime_unix) + MAC_EPOCH_OFFSET
    f_ct = int(f_ctime_unix) + MAC_EPOCH_OFFSET

    v_enc = v_name.encode("utf-8")[:27]
    v_pas = bytes([len(v_enc)]) + v_enc + b"\x00" * (27 - len(v_enc))
    f_enc = f_name.encode("utf-8")[:63]
    f_pas = bytes([len(f_enc)]) + f_enc + b"\x00" * (63 - len(f_enc))

    extra = bytearray()
    def add_tag(tag: int, payload: bytes):
        extra.extend(struct.pack(">hh", tag, len(payload)))
        extra.extend(payload)
        if len(payload) % 2 == 1:
            extra.append(0)

    carbon_path = f"{v_name}:{posix_rel.lstrip('/').replace('/', ':')}".encode("utf-8")
    add_tag(2, carbon_path)
    f_u16 = f_name.encode("utf-16-be")
    add_tag(14, struct.pack(">H", len(f_name)) + f_u16)
    v_u16 = v_name.encode("utf-16-be")
    add_tag(15, struct.pack(">H", len(v_name)) + v_u16)
    add_tag(18, posix_rel.encode("utf-8"))
    add_tag(19, vol_mnt.encode("utf-8"))
    extra.extend(struct.pack(">hh", -1, 0))

    total_len = 150 + len(extra)
    header = struct.pack(
        ">4sHHH28sI2sHI64sII4s4shhIH10s",
        b"\x00\x00\x00\x00",
        total_len,
        2,
        0,
        v_pas,
        v_ct,
        b"H+",
        5,
        parent_cnid,
        f_pas,
        file_cnid,
        f_ct,
        b"\x00\x00\x00\x00",
        b"\x00\x00\x00\x00",
        -1,
        -1,
        0x00000D02,
        0,
        b"\x00" * 10
    )
    return header + bytes(extra)

alias_bytes = make_hfs_alias_v2(
    vol_name,
    vol_ctime,
    ".background.tiff",
    2,
    int(bg_stat.st_ino),
    bg_ctime,
    "/.background.tiff",
    f"/Volumes/{vol_name}",
)

bwsp = plistlib.dumps({
    "ContainerShowSidebar": False,
    "ShowPathbar": False,
    "ShowSidebar": False,
    "ShowStatusBar": False,
    "ShowTabView": False,
    "ShowToolbar": False,
    "WindowBounds": "{{140, 140}, {800, 528}}",
    "WindowStyle": "icnv",
}, fmt=plistlib.FMT_BINARY)

icvp = plistlib.dumps({
    "arrangeBy": "none",
    "backgroundColorBlue": 0.0,
    "backgroundColorGreen": 0.0,
    "backgroundColorRed": 0.0,
    "backgroundImageAlias": alias_bytes,
    "backgroundType": 2,
    "gridOffsetX": 0.0,
    "gridOffsetY": 0.0,
    "gridSpacing": 100.0,
    "iconSize": 104.0,
    "labelOnBottom": True,
    "showIconPreview": True,
    "showItemInfo": False,
    "textSize": 13.0,
    "viewOptionsVersion": 1,
}, fmt=plistlib.FMT_BINARY)

def make_iloc(x: int, y: int) -> bytes:
    return struct.pack(">IIII", x, y, 0xFFFFFFFF, 0xFFFF0000)

raw_records = [
    (".", b"bwsp", b"blob", struct.pack(">I", len(bwsp)) + bwsp),
    (".", b"icvp", b"blob", struct.pack(">I", len(icvp)) + icvp),
    (".", b"vSrn", b"long", struct.pack(">I", 1)),
    (app_bundle_name, b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(200, 240)),
    ("Applications", b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(600, 240)),
    (".background.tiff", b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(2000, 2000)),
    (".background", b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(2000, 2000)),
    (".VolumeIcon.icns", b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(2000, 2000)),
    (".DS_Store", b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(2000, 2000)),
    (".fseventsd", b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(2000, 2000)),
    (".Trashes", b"Iloc", b"blob", struct.pack(">I", 16) + make_iloc(2000, 2000)),
]
raw_records.sort(key=lambda r: (r[0].lower().encode("utf-16-be"), r[1]))

leaf = bytearray(struct.pack(">II", 0, len(raw_records)))
for fname, sid, dtype, payload in raw_records:
    u16 = fname.encode("utf-16-be")
    leaf.extend(struct.pack(">I", len(fname)))
    leaf.extend(u16)
    leaf.extend(sid)
    leaf.extend(dtype)
    leaf.extend(payload)
assert len(leaf) <= 4096, "Leaf node overflow"
leaf.extend(b"\x00" * (4096 - len(leaf)))

buf = bytearray(4 + 0x2800)
struct.pack_into(">I", buf, 0, 1)
struct.pack_into(">4sIII16s", buf, 4, b"Bud1", 0x2000, 0x0800, 0x2000, b"\x00" * 16)
struct.pack_into(">IIIII", buf, 4 + 0x0020, 2, 0, len(raw_records), 1, 0x1000)
buf[4 + 0x1000:4 + 0x2000] = leaf

info = bytearray()
info.extend(struct.pack(">II", 3, 0))
addrs = [0x200B, 0x0025, 0x100C] + [0] * 253
info.extend(struct.pack(">256I", *addrs))
info.extend(struct.pack(">I", 1))
info.extend(b"\x04DSDB" + struct.pack(">I", 1))
freelists = {
    6: [0x0040],
    7: [0x0080],
    8: [0x0100],
    9: [0x0200],
    10: [0x0400],
    11: [0x0800, 0x2800],
    12: [0x3000],
    13: [0x4000],
}
for i in range(14, 31):
    freelists[i] = [1 << i]
for i in range(32):
    fl = freelists.get(i, [])
    info.extend(struct.pack(">I", len(fl)))
    for off in fl:
        info.extend(struct.pack(">I", off))
assert len(info) <= 2048
info.extend(b"\x00" * (2048 - len(info)))
buf[4 + 0x2000:4 + 0x2800] = info

ds_path = os.path.join(mount_dir, ".DS_Store")
with open(ds_path, "wb") as f:
    f.write(buf)
PY

chflags hidden "${mount_dir}/.DS_Store" 2>/dev/null || true
sync
hdiutil detach "$mount_dir" -quiet
mounted=0

hdiutil convert "$rw_dmg" -format UDZO -imagekey zlib-level=1 -o "$final_dmg" >/dev/null
rm -f "$rw_dmg"
echo "Packaged ${app_name} as ${final_dmg}"
