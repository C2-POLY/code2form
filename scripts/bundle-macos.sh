#!/usr/bin/env bash
# Builds "Code2Form.app" in target/release.
#
#   scripts/bundle-macos.sh             build the app
#   scripts/bundle-macos.sh --install   also put it in ~/Applications, where
#                                       Launchpad, Spotlight and the Finder see it
#   scripts/bundle-macos.sh --install --default
#                                       and make it the app that opens .c2f and .scad files
#
# The icon comes from crates/studio/assets/AppIcon.icns; redraw it with
# `swift scripts/make-icon.swift crates/studio/assets/AppIcon.icns`. The
# fonts and libraries come from the reference app, installed in
# /Applications or wherever C2F_REFERENCE_RESOURCES points.
set -euo pipefail

install=false
default=false
for arg in "$@"; do
  case "$arg" in
    --install) install=true ;;
    --default) default=true ;;
    *) echo "usage: $0 [--install [--default]]" >&2; exit 2 ;;
  esac
done
if $default && ! $install; then
  echo "--default needs --install" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
assets="$root/crates/studio/assets"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")"

cargo build --release -p code2form --manifest-path "$root/Cargo.toml"

app="$root/target/release/Code2Form.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/code2form" "$app/Contents/MacOS/"
cp "$assets/AppIcon.icns" "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" "$assets/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# The fonts text() defaults to (Liberation, SIL Open Font License) and the
# MCAD library (LGPL 2.1), with their licences, copied from the reference
# app so that this one works on its own.
resources="${C2F_REFERENCE_RESOURCES:-}"
for candidate in /Applications/OpenSCAD-2021.01.app /Applications/OpenSCAD.app; do
  if [[ -z "$resources" && -d "$candidate/Contents/Resources/fonts" ]]; then
    resources="$candidate/Contents/Resources"
  fi
done
if [[ -n "$resources" ]]; then
  mkdir -p "$app/Contents/Resources/fonts" "$app/Contents/Resources/libraries"
  for fonts in "$resources"/fonts/Liberation-*; do
    ditto "$fonts" "$app/Contents/Resources/fonts/$(basename "$fonts")"
  done
  ditto "$resources/libraries/MCAD" "$app/Contents/Resources/libraries/MCAD"
  echo "bundled the fonts and libraries of $resources"
else
  echo "warning: the reference app was not found (set C2F_REFERENCE_RESOURCES to its" \
    "Contents/Resources): text() and MCAD will need it installed" >&2
fi

# An ad-hoc signature: enough for a locally built app on Apple silicon.
codesign --force --sign - "$app"
echo "built $app ($version)"

if $install; then
  target="$HOME/Applications/Code2Form.app"
  mkdir -p "$HOME/Applications"
  rm -rf "$target"
  ditto "$app" "$target"
  lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
  "$lsregister" -f "$target"
  echo "installed $target"
fi

if $default; then
  swift - "$target" <<'SWIFT'
import AppKit
import UniformTypeIdentifiers

let app = URL(fileURLWithPath: CommandLine.arguments[1])
for ext in ["c2f", "scad"] {
    guard let type = UTType(filenameExtension: ext) else {
        FileHandle.standardError.write("no file type for .\(ext)\n".data(using: .utf8)!)
        exit(1)
    }
    var finished = false
    var failure: Error?
    NSWorkspace.shared.setDefaultApplication(at: app, toOpen: type) { error in
        failure = error
        finished = true
    }
    // The answer may come on the main thread: keep it running meanwhile.
    while !finished {
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    }
    if let failure {
        FileHandle.standardError.write("cannot set the default app for .\(ext): \(failure)\n".data(using: .utf8)!)
        exit(1)
    }
    print(".\(ext) files (\(type.identifier)) now open with \(app.lastPathComponent)")
}
SWIFT
fi
