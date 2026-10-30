#!/usr/bin/env bash
# Builds the engine for the iPad app: crates/mobile in release for iOS
# (aarch64-apple-ios) and its simulator on Apple silicon
# (aarch64-apple-ios-sim), wrapped in
# apps/ios/Code2FormCore/Code2FormCore.xcframework, and its Swift
# bindings, apps/ios/Code2FormCore/Sources/Code2FormCore/Code2FormCore.swift,
# which UniFFI writes. The Swift package there links both; neither is
# committed.
#
#   scripts/ios-xcframework.sh
#
# The slices come from one rustup toolchain that has both targets and is
# at least Rust 1.86, which uniffi-bindgen needs: C2F_TOOLCHAIN, else the
# active one, else the first that fits. Homebrew's Rust, often first on
# PATH, has no iOS targets: install rustup, then run
# `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
package="$root/apps/ios/Code2FormCore"
targets=(aarch64-apple-ios aarch64-apple-ios-sim)
# Never let rustup download a toolchain behind the user's back.
export RUSTUP_AUTO_INSTALL=0
# The app's minimum; the C and C++ dependencies follow it too.
export IPHONEOS_DEPLOYMENT_TARGET=18.0

fits() {
  local sysroot target minor
  sysroot="$("$1" --print sysroot 2>/dev/null)" || return 1
  for target in "${targets[@]}"; do
    [[ -d "$sysroot/lib/rustlib/$target" ]] || return 1
  done
  minor="$("$1" --version | sed -n 's/^rustc 1\.\([0-9]*\).*/\1/p')"
  [[ -n "$minor" && "$minor" -ge 86 ]]
}

rustup="$(command -v rustup || true)"
if [[ -z "$rustup" && -x /opt/homebrew/opt/rustup/bin/rustup ]]; then
  rustup=/opt/homebrew/opt/rustup/bin/rustup  # Homebrew installs it keg-only
fi
cargo=""
if [[ -n "$rustup" ]]; then
  active="$("$rustup" show active-toolchain 2>/dev/null | cut -d' ' -f1 || true)"
  for toolchain in ${C2F_TOOLCHAIN:-$active $("$rustup" toolchain list | cut -d' ' -f1)}; do
    rustc="$("$rustup" which --toolchain "$toolchain" rustc 2>/dev/null)" || continue
    if fits "$rustc"; then
      # Cargo would otherwise run the first rustc on PATH, maybe another one.
      export RUSTC="$rustc"
      cargo="$(dirname "$rustc")/cargo"
      echo "building with the rustup toolchain $toolchain"
      break
    fi
  done
fi
if [[ -z "$cargo" ]]; then
  echo "no rustup toolchain of Rust 1.86 or later has ${targets[*]}: install" \
    "rustup, then run rustup target add ${targets[*]}" >&2
  exit 1
fi

"$cargo" build --release -p c2f-mobile --lib --manifest-path "$root/Cargo.toml" \
  --target "${targets[0]}" --target "${targets[1]}"

# The bindings, read from the library built for this Mac.
"$cargo" build --release -p c2f-mobile --features bindgen --manifest-path "$root/Cargo.toml"
target="${CARGO_TARGET_DIR:-$root/target}"
generated="$target/uniffi-swift"
rm -rf "$generated"
"$target/release/uniffi-bindgen" generate --library "$target/release/libc2f_mobile.dylib" \
  --language swift --out-dir "$generated"

# The C header and its module map in a folder of their own: an app that
# links another XCFramework with a module map at the top of its headers
# would otherwise get two files at the same place.
headers="$generated/headers"
mkdir -p "$headers/Code2FormCoreFFI"
cp "$generated/Code2FormCoreFFI.h" "$headers/Code2FormCoreFFI/"
cp "$generated/Code2FormCoreFFI.modulemap" "$headers/Code2FormCoreFFI/module.modulemap"

xcframework="$package/Code2FormCore.xcframework"
rm -rf "$xcframework"
xcodebuild -create-xcframework \
  -library "$target/${targets[0]}/release/libc2f_mobile.a" -headers "$headers" \
  -library "$target/${targets[1]}/release/libc2f_mobile.a" -headers "$headers" \
  -output "$xcframework"

mkdir -p "$package/Sources/Code2FormCore"
cp "$generated/Code2FormCore.swift" "$package/Sources/Code2FormCore/"
echo "wrote $xcframework and its Swift bindings"
