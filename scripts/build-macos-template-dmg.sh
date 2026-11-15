#!/usr/bin/env bash
set -euo pipefail

master_binary="${1:?usage: build-macos-template-dmg.sh MASTER_BINARY OUTPUT_TEMPLATE_DMG}"
output_template_dmg="${2:?usage: build-macos-template-dmg.sh MASTER_BINARY OUTPUT_TEMPLATE_DMG}"

APP_NAME="VOID" \
APP_SLUG="void" \
APP_VERSION="1.0.0" \
ENGINE_PROFILE="${ENGINE_PROFILE:-lite}" \
MASTER_BINARY_PATH="$master_binary" \
MASTER_TEMPLATE_DMG="__none__" \
  bash scripts/package-macos-dmg.sh

hdiutil convert output/void.dmg -format UDRW -o "$output_template_dmg" >/dev/null
echo "Built pre-baked macOS template DMG at ${output_template_dmg}"
