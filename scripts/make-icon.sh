#!/usr/bin/env bash
# Draws the app icon and writes it as the bundle's .icns file, macOS only:
#
#   scripts/make-icon.sh                 crates/studio/assets/AppIcon.icns, graphite
#   scripts/make-icon.sh out.icns teal   another file, another tint
#
# The drawing lives in crates/studio/src/app_icon.rs, which also draws the
# icons the app's settings offer; graphite is the default.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
output="${1:-$root/crates/studio/assets/AppIcon.icns}"
tint="${2:-graphite}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cargo run --quiet --release --manifest-path "$root/Cargo.toml" -p code2form --example app-icon -- \
  "$work/AppIcon.iconset" "$tint"
iconutil -c icns "$work/AppIcon.iconset" -o "$output"
echo "wrote $output ($tint)"
