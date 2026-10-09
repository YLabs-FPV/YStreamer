#!/bin/sh
# Builds the SD card image: build.sh [--reuse] [file.deb]
# Needs Docker. Takes the newest package from `mise run device:deb` unless
# given one, and leaves ystreamer-<version>.img.xz in image/deploy
#
# --reuse keeps Raspberry Pi OS as the last build made it and redoes only the
# YStreamer part: minutes instead of half an hour, but with packages as old
# as that build. Releases are built without it
set -eu

REUSE=
if [ "${1:-}" = --reuse ]; then
    REUSE=1
    shift
fi

# Moving this on is a deliberate change, like any other dependency
PI_GEN_REF=4d8ee447dd3d37e8b0ef8752e460d9082d9d435d

HERE=$(cd "$(dirname "$0")" && pwd)
DEB=${1:-$(ls -t "$HERE"/../device/backend/target/debian/*.deb | head -1)}
VERSION=$(dpkg-deb -f "$DEB" Version)
VERSION=${VERSION%-*}
[ "$(dpkg-deb -f "$DEB" Architecture)" = arm64 ] || {
    echo "$DEB isn't an arm64 package" >&2
    exit 1
}

PI_GEN=$HERE/.pi-gen
if [ ! -d "$PI_GEN/.git" ]; then
    git init -q "$PI_GEN"
    git -C "$PI_GEN" remote add origin https://github.com/RPi-Distro/pi-gen
fi
git -C "$PI_GEN" fetch -q --depth 1 origin "$PI_GEN_REF"
git -C "$PI_GEN" checkout -q --detach FETCH_HEAD
git -C "$PI_GEN" clean -qfdx

cp "$HERE/config" "$PI_GEN/config"
# A stage that runs starts from the one before it, not from its own leftovers
echo CLEAN=1 >>"$PI_GEN/config"
cp -r "$HERE/stage-ystreamer" "$PI_GEN/stage-ystreamer"
mkdir -p "$PI_GEN/stage-ystreamer/00-ystreamer/files"
cp "$DEB" "$PI_GEN/stage-ystreamer/00-ystreamer/files/ystreamer.deb"
# Only our stage's image is wanted, not plain Raspberry Pi OS Lite as well
touch "$PI_GEN/stage2/SKIP_IMAGES"

docker rm -v pigen_work_cont >/dev/null 2>&1 || true
if [ -n "$REUSE" ] && docker container inspect pigen_work >/dev/null 2>&1; then
    touch "$PI_GEN/stage0/SKIP" "$PI_GEN/stage1/SKIP" "$PI_GEN/stage2/SKIP"
    (cd "$PI_GEN" && CONTINUE=1 PRESERVE_CONTAINER=1 ./build-docker.sh)
else
    docker rm -v pigen_work >/dev/null 2>&1 || true
    # Kept afterwards for --reuse; `docker rm -v pigen_work` frees the space
    (cd "$PI_GEN" && PRESERVE_CONTAINER=1 ./build-docker.sh)
fi

mkdir -p "$HERE/deploy"
IMAGE=$(ls -t "$PI_GEN"/deploy/*.img.xz | head -1)
cp "$IMAGE" "$HERE/deploy/ystreamer-$VERSION.img.xz"
ls -lh "$HERE/deploy/ystreamer-$VERSION.img.xz"
