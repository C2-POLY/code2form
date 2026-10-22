#!/usr/bin/env bash
# Regenerates the oracle goldens from the reference implementation.
#
# tests/oracle/cases (parser): case.ast from `-o x.ast`, or case.err with
# the first parser error when the reference rejects the file.
# tests/oracle/eval (evaluator): case.csg from `-o x.csg`, and case.echo
# with the ECHO messages it printed.
#
#   C2F_REFERENCE=/path/to/its/executable scripts/oracle.sh
set -euo pipefail

reference="${C2F_REFERENCE:-/Applications/OpenSCAD-2021.01.app/Contents/MacOS/OpenSCAD}"
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

"$reference" --version 2>&1 | tail -1 > "$root/tests/oracle/REFERENCE_VERSION"

for scad in "$root"/tests/oracle/cases/*.scad; do
  name="${scad%.scad}"
  rm -f "$name.ast" "$name.err"
  log="$("$reference" -o "$tmp/out.ast" "$scad" 2>&1 || true)"
  if [[ -f "$tmp/out.ast" ]]; then
    mv "$tmp/out.ast" "$name.ast"
  else
    # Keep the message, drop the absolute path the reference puts in it.
    grep -m1 '^ERROR' <<<"$log" | sed -E 's/ in file [^,]*,/,/' > "$name.err"
  fi
done

for scad in "$root"/tests/oracle/eval/*.scad; do
  name="${scad%.scad}"
  (cd "$(dirname "$scad")" && "$reference" -o "$tmp/out.csg" "$(basename "$scad")" 2> "$tmp/log" || true)
  mv "$tmp/out.csg" "$name.csg"
  "$root/scripts/echo-lines.awk" "$tmp/log" > "$name.echo"
done
# tests/oracle/params (customizer): for each parameter set of case.json,
# case.N.csg and case.N.echo from `-p case.json -P <set N>`, N counting
# the sets from 1 in file order.
for scad in "$root"/tests/oracle/params/*.scad; do
  name="${scad%.scad}"
  n=0
  while IFS= read -r set; do
    n=$((n + 1))
    (cd "$(dirname "$scad")" && "$reference" -o "$tmp/out.csg" -p "$(basename "$name").json" -P "$set" \
      "$(basename "$scad")" 2> "$tmp/log" || true)
    mv "$tmp/out.csg" "$name.$n.csg"
    "$root/scripts/echo-lines.awk" "$tmp/log" > "$name.$n.echo"
  done < <(python3 -c 'import json, sys; [print(k) for k in json.load(open(sys.argv[1]))["parameterSets"]]' "$name.json")
done
# tests/oracle/time (animation): case.T.csg and case.T.echo from
# `-D '$t=T'`, at the times below.
for scad in "$root"/tests/oracle/time/*.scad; do
  name="${scad%.scad}"
  for t in 0 0.25 0.7; do
    (cd "$(dirname "$scad")" && "$reference" -D "\$t=$t" -o "$tmp/out.csg" "$(basename "$scad")" 2> "$tmp/log" || true)
    mv "$tmp/out.csg" "$name.$t.csg"
    "$root/scripts/echo-lines.awk" "$tmp/log" > "$name.$t.echo"
  done
done
echo "goldens written under $root/tests/oracle"
