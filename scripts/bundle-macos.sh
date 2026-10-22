#!/usr/bin/env bash
# Builds "Code2Form.app" in target/release, universal (Apple silicon and
# Intel), and the two zips a release ships next to it:
# Code2Form-<version>-macos-universal.zip (the app) and
# c2f-<version>-macos-universal.zip (the command line).
#
#   scripts/bundle-macos.sh             build the app
#   scripts/bundle-macos.sh --install   also put it in ~/Applications, where
#                                       Launchpad, Spotlight and the Finder see it
#   scripts/bundle-macos.sh --install --default
#                                       and make it the app that opens .c2f, .c2fc and .scad files
#
# The icon comes from crates/studio/assets/AppIcon.icns; redraw it with
# `scripts/make-icon.sh`. The
# fonts come from the repository's fonts folder, and MCAD from the
# reference release's resources when C2F_REFERENCE_RESOURCES points there.
#
# Both slices come from one Rust toolchain that has the targets
# aarch64-apple-darwin and x86_64-apple-darwin: C2F_TOOLCHAIN, else the
# first rustup toolchain that has them, else the cargo on PATH.
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

targets=(aarch64-apple-darwin x86_64-apple-darwin)
# Never let rustup download a toolchain behind the user's back.
export RUSTUP_AUTO_INSTALL=0
# Info.plist declares macOS 11; the C and C++ dependencies follow it too.
export MACOSX_DEPLOYMENT_TARGET=11.0

has_targets() {
  local sysroot target
  sysroot="$("$1" --print sysroot 2>/dev/null)" || return 1
  for target in "${targets[@]}"; do
    [[ -d "$sysroot/lib/rustlib/$target" ]] || return 1
  done
}

cargo=cargo
rustup="$(command -v rustup || true)"
if [[ -z "$rustup" && -x /opt/homebrew/opt/rustup/bin/rustup ]]; then
  rustup=/opt/homebrew/opt/rustup/bin/rustup  # Homebrew installs it keg-only
fi
found=false
if [[ -n "$rustup" ]]; then
  for toolchain in ${C2F_TOOLCHAIN:-$("$rustup" toolchain list | cut -d' ' -f1)}; do
    rustc="$("$rustup" which --toolchain "$toolchain" rustc 2>/dev/null)" || continue
    if has_targets "$rustc"; then
      # Cargo would otherwise run the first rustc on PATH, maybe another one.
      export RUSTC="$rustc"
      cargo="$(dirname "$rustc")/cargo"
      found=true
      echo "building with the rustup toolchain $toolchain"
      break
    fi
  done
fi
if ! $found && ! has_targets rustc; then
  echo "no Rust toolchain has both ${targets[*]}: install rustup, then run" \
    "rustup target add ${targets[*]}" >&2
  exit 1
fi

"$cargo" build --release -p code2form -p c2f-cli --manifest-path "$root/Cargo.toml" \
  --target "${targets[0]}" --target "${targets[1]}"

universal="$root/target/universal-apple-darwin/release"
mkdir -p "$universal"
for bin in code2form c2f; do
  lipo -create -output "$universal/$bin" \
    "$root/target/${targets[0]}/release/$bin" "$root/target/${targets[1]}/release/$bin"
done

app="$root/target/release/Code2Form.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$universal/code2form" "$app/Contents/MacOS/"
cp "$assets/AppIcon.icns" "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" "$assets/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"

# The repository's own library (MIT OR Apache-2.0, as the rest of it),
# with its licences, found by `use <electronics/...>` as MCAD is.
mkdir -p "$app/Contents/Resources/libraries"
ditto "$root/library" "$app/Contents/Resources/libraries"
cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$app/Contents/Resources/libraries/"

# The fonts text() defaults to (Liberation 2.00.1, SIL Open Font License,
# with their licence), from the repository.
mkdir -p "$app/Contents/Resources/fonts"
ditto "$root/fonts" "$app/Contents/Resources/fonts"

# The MCAD library (LGPL 2.1), with its licence, from the reference
# release's resources when C2F_REFERENCE_RESOURCES names them.
resources="${C2F_REFERENCE_RESOURCES:-}"
if [[ -n "$resources" && -d "$resources/libraries/MCAD" ]]; then
  ditto "$resources/libraries/MCAD" "$app/Contents/Resources/libraries/MCAD"
  echo "bundled MCAD from $resources"
else
  echo "note: MCAD not bundled (set C2F_REFERENCE_RESOURCES to the reference" \
    "release's Contents/Resources to include it)" >&2
fi

# An ad-hoc signature: enough for a locally built app on Apple silicon.
codesign --force --sign - "$app"
codesign --force --sign - "$universal/c2f"
echo "built $app ($version, $(lipo -archs "$app/Contents/MacOS/code2form"))"

app_zip="$root/target/release/Code2Form-$version-macos-universal.zip"
cli_zip="$root/target/release/c2f-$version-macos-universal.zip"
rm -f "$app_zip" "$cli_zip"
ditto -c -k --keepParent "$app" "$app_zip"
zip -q -j "$cli_zip" "$universal/c2f" "$root/LICENSE-MIT" "$root/LICENSE-APACHE"
echo "zipped $app_zip and $cli_zip"

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
for ext in ["c2f", "c2fc", "scad"] {
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
