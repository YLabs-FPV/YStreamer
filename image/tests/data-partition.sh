#!/bin/sh
# Tests the image's data-partition script against file-backed disks: a first
# start, restarts, an interrupted setup, a card flashed again over an earlier
# install, a card that's too small, damage.
# Needs Docker and the pi-gen image that image/build.sh leaves behind, and
# runs privileged because it partitions loop devices (only ones it creates)
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cp "$HERE/../stage-ystreamer/01-data-partition/files/ystreamer-data" "$HERE/data-partition-cases.sh" "$WORK/"
docker run --rm --privileged -v /dev:/dev -v "$WORK":/work pi-gen bash /work/data-partition-cases.sh
