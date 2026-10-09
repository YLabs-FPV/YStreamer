#!/bin/sh
# Writes the manifest.json devices look for: manifest.sh <file.deb> <base-url>
# The package and its .sig (from sign.sh) must then be served at <base-url>/
# The notes are the version's section of CHANGELOG.md
set -e
DEB=$1
BASE=${2%/}
CHANGELOG=$(dirname "$0")/../../CHANGELOG.md
VERSION=$(dpkg-deb -f "$DEB" Version)
ARCH=$(dpkg-deb -f "$DEB" Architecture)
OUT=$(dirname "$DEB")/manifest.json
# A failure leaves the last good manifest in place
trap 'rm -f "$OUT.tmp"' EXIT
python3 - "${VERSION%-*}" "$ARCH" "$BASE/$(basename "$DEB")" "$(cat "$DEB.sig")" "$CHANGELOG" >"$OUT.tmp" <<'PY'
import json, re, sys
version, arch, url, signature, changelog = sys.argv[1:]
notes = ""
for section in re.split(r"^## ", open(changelog).read(), flags=re.M)[1:]:
    heading, _, body = section.partition("\n")
    if heading.split()[0].strip("[]") == version:
        notes = body.strip()
if not notes:
    sys.exit(f"CHANGELOG.md has no section for {version}")
json.dump({"version": version, "notes": notes, "packages": {arch: {"url": url, "signature": signature}}}, sys.stdout, indent=2)
PY
mv "$OUT.tmp" "$OUT"
echo "$OUT"
