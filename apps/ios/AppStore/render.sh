#!/usr/bin/env bash
# Draws the App Store assets from the app's own screens, at the sizes App
# Store Connect asks for. The captures come from the simulators, in dark
# mode with the status bar at 9:41; the outputs are not committed.
#
#   raw/       iPhone 17 Pro Max   -> en/       1320 × 2868 (6.9")
#                                  -> en-6.5/   1284 × 2778 (6.5")
#                                  -> en-6.3/   1206 × 2622 (6.3", "iPhone with Dynamic Island")
#   raw-ipad/  iPad Pro 13-inch    -> en-ipad/  2064 × 2752 (13")
#   both                           -> hero/header.png  5244 × 2950 ("Header")
#                                     hero/search.png  3840 × 2560 ("Search results")
#
#   apps/ios/AppStore/render.sh
#
# Needs Google Chrome, which draws frame.html and hero.html headless.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

# shot|title|subtitle, in the order of the listing.
captions=(
  "winder|Your parts in 3D,<br><em>in your pocket</em>|Open a .c2f or .scad design and spin it with your fingertips."
  "gears|Computed<br><em>on your iPhone</em>|The same engine as on the Mac, with no network and no server."
  "nightlight|Sizes to<br><em>the millimetre</em>|Size, volume and triangles, before you start the print."
  "files|From Files<br><em>and iCloud Drive</em>|Your designs follow you from the Mac to iPhone and iPad."
  "chest|Free<br><em>and open source</em>|No account, no ads, no tracking. Your files stay yours."
)

urlencode() { python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$1"; }

# Draws a page of this folder at a size into a PNG. App Store Connect
# refuses an alpha channel: Chrome's screenshots have none, and this checks.
draw() {
  local page="$1" size="$2" out="$3"
  "$chrome" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --allow-file-access-from-files --window-size="$size" \
    --screenshot="$out" "file://$here/$page" >/dev/null 2>&1
  if sips -g hasAlpha "$out" | grep -q 'hasAlpha: yes'; then
    echo "$out has an alpha channel" >&2
    exit 1
  fi
  echo "$out"
}

# The screenshots: each capture under its caption, in a device.
screenshots() {
  local device="$1" raw="$2" size="$3" dir="$4"
  mkdir -p "$here/$dir"
  local i=1 entry shot title sub
  for entry in "${captions[@]}"; do
    IFS='|' read -r shot title sub <<<"$entry"
    # The captions name the device they show.
    [[ "$device" == ipad ]] && title="${title//iPhone/iPad}"
    draw "frame.html?device=$device&shot=$(urlencode "$raw/$shot.png")&title=$(urlencode "$title")&sub=$(urlencode "$sub")" \
      "$size" "$here/$dir/$(printf '%02d' "$i")-$shot.png"
    i=$((i + 1))
  done
}

# The smaller iPhones, from the 6.9" set: scaled so both sides cover the
# target, then the few extra pixels cropped from the middle.
smaller() {
  local dir="$1" w="$2" h="$3" src out
  mkdir -p "$here/$dir"
  for src in "$here"/en/*.png; do
    out="$here/$dir/$(basename "$src")"
    cp "$src" "$out"
    if (( 1320 * h > 2868 * w )); then sips --resampleHeight "$h" "$out" >/dev/null
    else sips --resampleWidth "$w" "$out" >/dev/null; fi
    sips --cropToHeightWidth "$h" "$w" "$out" >/dev/null
    echo "$out"
  done
}

# The wide assets of "Header and search results".
hero() {
  local kind="$1" size="$2" text="$3"
  mkdir -p "$here/hero"
  draw "hero.html?kind=$kind&text=$(urlencode "$text")" "$size" "$here/hero/$kind.png"
}

screenshots iphone raw 1320,2868 en
screenshots ipad raw-ipad 2064,2752 en-ipad
smaller en-6.5 1284 2778
smaller en-6.3 1206 2622
hero header 5244,2950 '{"title":"Your 3D parts, <em>in your pocket</em>","sub":"Open your Code2Form and .scad designs on iPhone and iPad, computed right on the device."}'
hero search 3840,2560 '{"title":"Your parts in 3D, <em>to the millimetre</em>","sub":"Spin them, zoom in, check their size before you print."}'
