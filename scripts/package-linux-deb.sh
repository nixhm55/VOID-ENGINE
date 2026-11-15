#!/usr/bin/env bash
set -euo pipefail

binary="${1:?usage: package-linux-deb.sh MASTER_BINARY OUTPUT.deb}"
output="${2:?usage: package-linux-deb.sh MASTER_BINARY OUTPUT.deb}"
app_name="${APP_NAME:-VOID}"
app_slug="${APP_SLUG:-void}"
deb_package="${app_slug//_/-}"
version="${APP_VERSION:-1.0.0}"
[[ -f "$binary" && -f void.json && -f branding/icon.png ]] || { echo "A master binary, void.json, and icon are required" >&2; exit 1; }
[[ "$app_slug" =~ ^[a-z0-9][a-z0-9_+-]*$ ]] || { echo "Invalid app slug: $app_slug" >&2; exit 1; }
[[ "$deb_package" =~ ^[a-z0-9][a-z0-9+.-]*$ ]] || { echo "Invalid Debian package name: $deb_package" >&2; exit 1; }
[[ "$version" =~ ^[0-9A-Za-z.+:~-]+$ ]] || { echo "Invalid Debian package version: $version" >&2; exit 1; }
command -v dpkg-deb >/dev/null || { echo "dpkg-deb is required" >&2; exit 1; }

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/applications" \
  "$stage/usr/share/pixmaps" "$stage/usr/share/metainfo" \
  "$stage/usr/share/icons/hicolor/128x128/apps" \
  "$stage/usr/share/icons/hicolor/256x256/apps" \
  "$stage/usr/share/icons/hicolor/512x512/apps" "$(dirname "$output")"
install -m 0755 "$binary" "$stage/usr/bin/$app_slug"
install -m 0644 void.json "$stage/usr/bin/$app_slug.json"
install -m 0644 branding/icon.png "$stage/usr/share/pixmaps/$app_slug.png"
install -m 0644 branding/icon.png "$stage/usr/share/icons/hicolor/256x256/apps/$app_slug.png"
install -m 0644 branding/icon.png "$stage/usr/share/icons/hicolor/512x512/apps/$app_slug.png"
if command -v convert >/dev/null 2>&1; then
  convert branding/icon.png -resize 128x128 "$stage/usr/share/icons/hicolor/128x128/apps/$app_slug.png"
  convert branding/icon.png -resize 256x256 "$stage/usr/share/icons/hicolor/256x256/apps/$app_slug.png"
else
  install -m 0644 branding/icon.png "$stage/usr/share/icons/hicolor/128x128/apps/$app_slug.png"
fi

safe_name="${app_name//$'\n'/ }"
cat > "$stage/usr/share/applications/$app_slug.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$safe_name
Comment=$safe_name desktop application powered by VOID Engine
Exec=$app_slug
Icon=$app_slug
StartupWMClass=$app_slug
Categories=Utility;Network;
Terminal=false
EOF

cat > "$stage/usr/share/metainfo/${app_slug}.metainfo.xml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>com.void.${app_slug}</id>
  <name>${safe_name}</name>
  <summary>${safe_name} desktop application</summary>
  <metadata_license>MIT</metadata_license>
  <project_license>MIT</project_license>
  <description>
    <p>${safe_name} packaged as a lightweight native desktop application with VOID Engine.</p>
  </description>
  <launchable type="desktop-id">${app_slug}.desktop</launchable>
  <releases>
    <release version="${version}" date="2026-11-15"/>
  </releases>
</component>
EOF

installed_kb=$(du -sk "$stage/usr" | awk '{print $1}')
cat > "$stage/DEBIAN/control" <<EOF
Package: $deb_package
Version: $version
Section: web
Priority: optional
Architecture: amd64
Installed-Size: $installed_kb
Maintainer: VOID Engine <support@void.com>
Depends: libwebkit2gtk-4.1-0, geoclue-2.0
Description: $safe_name desktop application
 Packaged with VOID Engine for native Linux desktop integration.
EOF
dpkg-deb --build --root-owner-group "$stage" "$output"
echo "Packaged $app_name as $output"
