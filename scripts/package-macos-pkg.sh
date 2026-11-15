#!/usr/bin/env bash
set -euo pipefail

app_name="${APP_NAME:-VOID}"
app_slug="${APP_SLUG:-void}"
version="${APP_VERSION:-1.0.0}"
engine_profile="${ENGINE_PROFILE:-lite}"
binary="${MASTER_BINARY_PATH:-target/master/void}"
entitlements="macos/void.entitlements"
pkg_bg="branding/pkg-bg.png"
output_dir="output"
staging="pkg_staging"
payload_dir="${staging}/payload"
resources_dir="${staging}/resources"
app_bundle_name="${app_slug}.app"
app_path="${payload_dir}/${app_bundle_name}"
contents="${app_path}/Contents"

rm -rf "$staging" "$output_dir"
mkdir -p "${contents}/MacOS" "${contents}/Resources/AppIcon.iconset" "$resources_dir" "$output_dir"

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

# Generate Info.plist, Distribution XML, Welcome HTML, Conclusion HTML, and dark-matte background if needed
python3 - "$staging" "$app_name" "$app_slug" "$version" "$engine_profile" "$pkg_bg" <<'PY'
import pathlib, struct, sys, zlib
from xml.sax.saxutils import escape

staging_dir, app_name, app_slug, version, profile, pkg_bg_path = sys.argv[1:]
staging = pathlib.Path(staging_dir)
contents = staging / "payload" / f"{app_slug}.app" / "Contents"
resources = staging / "resources"

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
(contents / "Info.plist").write_text(info, encoding="utf-8")
(contents / "PkgInfo").write_text("APPL????", encoding="utf-8")

# Copy or synthesize minimalist pure-black matte background for PKG installer
bg_src = pathlib.Path(pkg_bg_path)
bg_dst = resources / "pkg-bg.png"
if bg_src.is_file():
    bg_dst.write_bytes(bg_src.read_bytes())
else:
    W, H = 620, 440
    raw = bytearray()
    for y in range(H):
        raw.append(0)
        for x in range(W):
            # Sleek pure-black to deep-carbon vertical matte gradient with subtle accent line
            c = 6 if x < 180 else 10
            if x == 180 and 40 <= y <= H - 40:
                c = 38
            raw.extend((c, c, c + 2))
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)
    ihdr = struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0)
    bg_dst.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))

welcome_html = f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body {{
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", sans-serif;
    margin: 24px 20px;
    color: #f5f5f7;
  }}
  h1 {{
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.02em;
    margin: 0 0 12px 0;
  }}
  p {{
    font-size: 13.5px;
    line-height: 1.55;
    opacity: 0.86;
    margin: 0 0 12px 0;
  }}
  .meta {{
    margin-top: 20px;
    padding-top: 14px;
    border-top: 1px solid rgba(255, 255, 255, 0.12);
    font-size: 12px;
    opacity: 0.65;
  }}
</style>
</head>
<body>
  <h1>Install {escape(app_name)}</h1>
  <p>This installer will guide you through installing <strong>{escape(app_name)}</strong> ({escape(version)}) into your Applications folder.</p>
  <p>Click <strong>Continue</strong> to proceed with the standard installation.</p>
  <div class="meta">Version {escape(version)} &bull; Powered by VOID Engine</div>
</body>
</html>
"""
(resources / "welcome.html").write_text(welcome_html, encoding="utf-8")

conclusion_html = f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body {{
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", sans-serif;
    margin: 24px 20px;
    color: #f5f5f7;
  }}
  h1 {{
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.02em;
    margin: 0 0 12px 0;
  }}
  p {{
    font-size: 13.5px;
    line-height: 1.55;
    opacity: 0.86;
    margin: 0 0 12px 0;
  }}
</style>
</head>
<body>
  <h1>{escape(app_name)} is Installed</h1>
  <p><strong>{escape(app_name)}</strong> has been installed in your <strong>/Applications</strong> folder and is ready to launch from Launchpad or Spotlight.</p>
</body>
</html>
"""
(resources / "conclusion.html").write_text(conclusion_html, encoding="utf-8")

pkg_id = f"com.void.{app_slug}.pkg"
dist_xml = f"""<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
  <title>{escape(app_name)}</title>
  <organization>com.void</organization>
  <domains enable_localSystem="true" enable_currentUserHome="true" />
  <options customize="never" require-scripts="false" rootVolumeOnly="true" hostArchitectures="x86_64,arm64" />
  <background file="pkg-bg.png" mime-type="image/png" alignment="bottomleft" scaling="toFit" />
  <background-darkAqua file="pkg-bg.png" mime-type="image/png" alignment="bottomleft" scaling="toFit" />
  <welcome file="welcome.html" mime-type="text/html" />
  <conclusion file="conclusion.html" mime-type="text/html" />
  <choices-outline>
    <line choice="default">
      <line choice="{escape(pkg_id)}" />
    </line>
  </choices-outline>
  <choice id="default" />
  <choice id="{escape(pkg_id)}" visible="false">
    <pkg-ref id="{escape(pkg_id)}" />
  </choice>
  <pkg-ref id="{escape(pkg_id)}" version="{escape(version)}" onConclusion="none">component.pkg</pkg-ref>
</installer-gui-script>
"""
(staging / "distribution.xml").write_text(dist_xml, encoding="utf-8")
PY

codesign --force --deep --sign - --entitlements "$entitlements" "$app_path"

component_pkg="${staging}/component.pkg"
final_pkg="${output_dir}/${app_slug}.pkg"

pkgbuild \
  --root "$payload_dir" \
  --install-location "/Applications" \
  --identifier "com.void.${app_slug}.pkg" \
  --version "$version" \
  --min-os-version 11.0 \
  --compression latest \
  "$component_pkg" >/dev/null

productbuild \
  --distribution "${staging}/distribution.xml" \
  --resources "$resources_dir" \
  --package-path "$staging" \
  "$final_pkg" >/dev/null

rm -rf "$staging"
echo "Packaged ${app_name} as ${final_pkg}"
