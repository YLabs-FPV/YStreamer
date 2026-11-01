#!/bin/sh
# Builds the .deb for Debian 12 "bookworm", which the Radxa ROCK 3A runs, in
# an arm64 Docker container (natively on an Apple Silicon or arm64 machine,
# emulated elsewhere): build.sh
#
# Needs Docker and the web interface built (mise run frontend:build; the
# device:deb-bookworm task does both). Leaves
# ystreamer_<version>-1~bookworm_arm64.deb in device/backend/target/bookworm/debian,
# which packaging/sign.sh signs and manifest.sh lists as arm64-bookworm
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
IMAGE=ystreamer-build-bookworm

[ -f "$ROOT/device/frontend/dist/index.html" ] || {
    echo "The web interface isn't built; run: mise run frontend:build" >&2
    exit 1
}

docker build --platform linux/arm64 -t "$IMAGE" "$HERE"
# The crate cache lives in a volume, so a second build doesn't download again
docker run --rm --platform linux/arm64 \
    -v "$ROOT:/src" \
    -v ystreamer-cargo-bookworm:/usr/local/cargo/registry \
    -w /src/device/backend \
    -e CARGO_TARGET_DIR=/src/device/backend/target/bookworm \
    "$IMAGE" cargo deb --deb-revision '1~bookworm'

ls -t "$ROOT"/device/backend/target/bookworm/debian/*.deb | head -1
