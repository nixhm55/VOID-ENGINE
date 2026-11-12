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
  "$stage/usr/share/icons/hicolor/256x256/apps" "$(dirname "$output")"
install -m 0755 "$binary" "$stage/usr/bin/$app_slug"
install -m 0644 void.json "$stage/usr/bin/$app_slug.json"
install -m 0644 branding/icon.png "$stage/usr/share/icons/hicolor/256x256/apps/$app_slug.png"
cat > "$stage/usr/share/applications/$app_slug.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=${app_name//$'\n'/ }
Exec=$app_slug
Icon=$app_slug
Categories=Utility;Network;
Terminal=false
EOF
cat > "$stage/DEBIAN/control" <<EOF
Package: $deb_package
Version: $version
Section: web
Priority: optional
Architecture: amd64
Maintainer: VOID Engine
Depends: libwebkit2gtk-4.1-0
Description: $app_name web application
EOF
dpkg-deb --build --root-owner-group "$stage" "$output"
echo "Packaged $app_name as $output"
