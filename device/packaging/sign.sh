#!/bin/sh
# Signs a package for the web interface's updater: sign.sh <file.deb>
# writes <file.deb>.sig. The key is the private half of update-key.pub
set -e
KEY=${YSTREAMER_SIGNING_KEY:-$HOME/ystreamer-signing-key.pem}
# Signed apart from the encoding, as a pipe would hide a failure and
# leave an empty .sig
signature=$(openssl pkeyutl -sign -inkey "$KEY" -rawin -in "$1" | base64 -w0)
[ -n "$signature" ] || {
    echo "Couldn't sign $1 with $KEY" >&2
    rm -f "$1.sig"
    exit 1
}
printf '%s' "$signature" >"$1.sig"
echo "$1.sig"
