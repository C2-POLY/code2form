#!/usr/bin/env bash
# Lists the libraries the iPhone and iPad app carries, with their licences,
# for the app's Acknowledgements screen: every crate the engine
# (crates/mobile) builds with for iOS, from cargo metadata.
#
#   scripts/ios-acknowledgements.sh
#
# Run it again when the engine's dependencies change. The fonts' licence
# is shown from the bundle itself (fonts/*/LICENSE).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/apps/ios/App/Acknowledgements.txt"

cargo metadata --format-version 1 --filter-platform aarch64-apple-ios --manifest-path "$root/Cargo.toml" | jq -r '
  (.packages | map({key: .id, value: .}) | from_entries) as $packages
  | (.resolve.nodes | map({key: .id, value: .}) | from_entries) as $nodes
  | (.packages[] | select(.name == "c2f-mobile") | .id) as $root
  # The crates the engine links: normal dependencies, followed from it.
  | def linked:
      . as $seen
      | ([.[] | $nodes[.].deps[] | select(any(.dep_kinds[]; .kind == null)) | .pkg] | unique - $seen) as $new
      | if ($new | length) == 0 then $seen else ($seen + $new | linked) end;
    [$root] | linked
  | map($packages[.]) | map(select(.source != null)) | sort_by(.name | ascii_downcase)
  | "Code2Form is free software under the MIT or Apache 2.0 licence, at its author'"'"'s choice. Its source: https://github.com/C2-POLY/code2form\n\nIt is built with these libraries, each under its own licence:\n",
    (.[] | "\(.name) \(.version): \(.license // "see its repository")" + (if .repository then "\n  \(.repository)" else "" end))
' > "$out"
echo "wrote $out ($(grep -c '^[a-z0-9_-]* [0-9]' "$out") libraries)"
