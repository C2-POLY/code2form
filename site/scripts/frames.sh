#!/bin/sh
# Renders the frames of the homepage's scroll story with the app's own
# renderer (crates/geom/examples/site_frames.rs), then crops and scales
# each one to 880 x 720 and writes it as WebP into site/public/frames.
#
# Needs a Rust toolchain and ffmpeg built with libwebp (Homebrew's, or most
# Linux distributions'). Run from anywhere:
#
#   site/scripts/frames.sh [model.c2f]
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
model=${1:-"$root/examples/winder/watch_winder.c2f"}
out="$root/site/public/frames"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cd "$root"
cargo run --release -p c2f-geom --example site_frames -- "$model" "$tmp"

mkdir -p "$out"
for png in "$tmp"/*.png; do
  name=$(basename "$png" .png)
  case "$name" in
    # The turntable is framed on the assembled model: keep all of it.
    turn-*) filter="crop=1100:900:50:0,scale=880:720:flags=lanczos" ;;
    # The exploded view is framed on the parts apart, with room around.
    *) filter="crop=880:720:160:90" ;;
  esac
  ffmpeg -loglevel error -y -i "$png" -vf "$filter" \
    -c:v libwebp -quality 78 -compression_level 6 "$out/$name.webp"
done
echo "Wrote $(ls "$out" | wc -l | tr -d ' ') frames to $out"
