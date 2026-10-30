#!/usr/bin/env bash
# Draws the app icon for iPhone and iPad and writes it into the iOS app's
# asset catalog, as the one 1024-pixel picture Xcode scales for every
# device:
#
#   scripts/ios-icon.sh            graphite, the default
#   scripts/ios-icon.sh teal       another tint
#
# The drawing lives in crates/studio/src/app_icon.rs, as the Mac app's
# does (scripts/make-icon.sh): square and opaque here, as iOS rounds it.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tint="${1:-graphite}"
icon="$root/apps/ios/App/Assets.xcassets/AppIcon.appiconset/AppIcon.png"

cargo run --quiet --release --manifest-path "$root/Cargo.toml" -p code2form --example app-icon -- "$icon" "$tint"
echo "wrote $icon ($tint)"
