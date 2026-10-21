#!/usr/bin/awk -f
# Keeps the ECHO messages of a reference implementation log. A message runs until the next
# line that starts another message, so echoed strings may span lines.
/^(ECHO|WARNING|DEPRECATED|ERROR|TRACE|INFO|FONT-WARNING|Compiling|Rendering|Parsing):/ ||
/^(QObject|Fontconfig|Please see|Can't|Saved|Current top level)/ {
  keep = ($0 ~ /^ECHO:/)
  if (keep) print
  next
}
keep { print }
