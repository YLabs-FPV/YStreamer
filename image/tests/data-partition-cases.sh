#!/bin/bash
# The cases themselves; data-partition.sh runs this in a privileged container.
# Only ever touches the loop devices it creates.
set -u
SCRIPT=/work/ystreamer-data
GB=$((1024 * 1024 * 1024))
fail=0

check() { # description, command...
    local what=$1
    shift
    if "$@" >/dev/null 2>&1; then echo "  ok    $what"; else
        echo "  FAIL  $what"
        fail=1
    fi
}

setup() { # size in GB -> sets LOOP, T
    IMG=/work/disk-$1.img
    rm -f "$IMG"
    truncate -s "${1}G" "$IMG"
    printf 'label: dos\n8M,512M,c\n,3G,L\n' | sfdisk -q "$IMG"
    LOOP=$(losetup --show --find --partscan "$IMG")
    sleep 1
    mkfs.vfat -n BOOTFS "${LOOP}p1" >/dev/null
    T=/tmp/t$1
    fresh_system
}

# The system partition as the image ships it
fresh_system() {
    mkfs.ext4 -q -F "${LOOP}p2"
    rm -rf "$T"
    mkdir -p "$T/sys" "$T/etc/ystreamer" "$T/var/lib/ystreamer/update"
    mount "${LOOP}p2" "$T/sys"
    touch "$T/etc/ystreamer/image"
    echo '{"version":1}' >"$T/etc/ystreamer/settings.json"
    echo deb >"$T/var/lib/ystreamer/update/current.deb"
}

# What flashing the image over a used card does: the image's partition table
# and system partition, and the rest of the card as it was
reflash() {
    umount "$T/etc/ystreamer" "$T/var/lib/ystreamer" "$T/data" "$T/sys" 2>/dev/null
    sfdisk -q --delete "$LOOP" 3 >/dev/null 2>&1
    echo ", 3G" | sfdisk -q --force --no-reread -N 2 "$LOOP" >/dev/null 2>&1
    partx -u "$LOOP"
    sleep 1
    fresh_system
}

run() { YSTREAMER_ROOT_PART="${LOOP}p2" YSTREAMER_PREFIX="$T" sh "$SCRIPT" 2>&1 | sed 's/^/    | /'; }

teardown() {
    umount "$T/etc/ystreamer" "$T/var/lib/ystreamer" "$T/data" "$T/sys" 2>/dev/null
    losetup -d "$LOOP" 2>/dev/null
    rm -f "$IMG"
}

size_gb() { echo $(($(cat "/sys/class/block/$(basename "$1")/size") * 512 / GB)); }

echo "== 16 GB card, first start"
setup 16
run
check "system partition is 8 GB" test "$(size_gb "${LOOP}p2")" = 8
check "system filesystem grew with it" test "$(df --output=size -BG "$T/sys" | tail -1 | tr -dc 0-9)" -ge 7
check "data partition exists and is labelled" test "$(blkid -o value -s LABEL "${LOOP}p3")" = ystreamer-data
check "data partition takes the rest (about 7 GB)" test "$(size_gb "${LOOP}p3")" -ge 6
check "settings folder is on the data partition" mountpoint -q "$T/etc/ystreamer"
check "state folder is on the data partition" mountpoint -q "$T/var/lib/ystreamer"
check "image marker came along" test -e "$T/etc/ystreamer/image"
check "settings came along" grep -q version "$T/etc/ystreamer/settings.json"
check "kept update package came along" test -e "$T/var/lib/ystreamer/update/current.deb"
check "data partition starts right behind the system one" test "$(cat /sys/class/block/$(basename ${LOOP}p3)/start)" = "$(( $(cat /sys/class/block/$(basename ${LOOP}p2)/start) + $(cat /sys/class/block/$(basename ${LOOP}p2)/size) ))"
check "the setup marker is gone" test ! -e "$T/var/lib/ystreamer-data.formatting"

echo "== same card, started again while mounted"
echo recording >"$T/var/lib/ystreamer/rec.mp4"
echo changed >"$T/etc/ystreamer/settings.json"
run
check "nothing was reformatted" grep -q recording "$T/var/lib/ystreamer/rec.mp4"

echo "== same card after a reboot (everything unmounted)"
umount "$T/etc/ystreamer" "$T/var/lib/ystreamer" "$T/data"
check "system partition shows the shipped settings again" grep -q version "$T/etc/ystreamer/settings.json"
run
check "recording is still there" grep -q recording "$T/var/lib/ystreamer/rec.mp4"
check "changed settings are still there" grep -q changed "$T/etc/ystreamer/settings.json"
check "system partition still 8 GB" test "$(size_gb "${LOOP}p2")" = 8
teardown

echo "== 16 GB card, power cut after the partition was added but before formatting"
setup 16
echo ", $((8 * GB / 512))" | sfdisk -q --force --no-reread -N 2 "$LOOP" >/dev/null 2>&1
partx -u "$LOOP"
mkdir -p "$T/var/lib" && touch "$T/var/lib/ystreamer-data.formatting"
echo "$((16384 + 1048576 + 8 * GB / 512)),,L" | sfdisk -q --force --no-reread --append "$LOOP" >/dev/null 2>&1
partx -u "$LOOP"
sleep 1
check "partition 3 is there, unformatted" test -z "$(blkid -o value -s TYPE "${LOOP}p3")"
run
check "it got formatted and used" mountpoint -q "$T/etc/ystreamer"
check "the setup marker is gone" test ! -e "$T/var/lib/ystreamer-data.formatting"
check "system filesystem caught up with its partition" test "$(df --output=size -BG "$T/sys" | tail -1 | tr -dc 0-9)" -ge 7
teardown

echo "== 16 GB card where partition 3 holds someone else's filesystem"
setup 16
echo ", $((8 * GB / 512))" | sfdisk -q --force --no-reread -N 2 "$LOOP" >/dev/null 2>&1
partx -u "$LOOP"
echo "$((16384 + 1048576 + 8 * GB / 512)),,L" | sfdisk -q --force --no-reread --append "$LOOP" >/dev/null 2>&1
partx -u "$LOOP"
sleep 1
mkfs.vfat -n PHOTOS "${LOOP}p3" >/dev/null
run
check "it was left alone" test "$(blkid -o value -s LABEL "${LOOP}p3")" = PHOTOS
check "settings stay on the system partition" sh -c "! mountpoint -q $T/etc/ystreamer"
teardown

echo "== 16 GB card flashed again over an earlier install"
setup 16
run >/dev/null
echo recording >"$T/var/lib/ystreamer/rec.mp4"
echo '{"password":"old"}' >"$T/etc/ystreamer/settings.json"
reflash
check "partition 3 is gone from the table" sh -c "! sfdisk -d $LOOP | grep -q ${LOOP}p3"
run
check "data partition is back and in use" mountpoint -q "$T/var/lib/ystreamer"
check "the earlier recording is gone" test ! -e "$T/var/lib/ystreamer/rec.mp4"
check "the shipped settings are in use, not the earlier ones" grep -q version "$T/etc/ystreamer/settings.json"
check "the setup marker is gone" test ! -e "$T/var/lib/ystreamer-data.formatting"
umount "$T/etc/ystreamer" "$T/var/lib/ystreamer" "$T/data"
run
check "after a reboot, still the fresh settings" grep -q version "$T/etc/ystreamer/settings.json"
check "after a reboot, still no earlier recording" test ! -e "$T/var/lib/ystreamer/rec.mp4"
teardown

echo "== 8 GB card: too small"
setup 8
run
check "no data partition was made" test ! -e "${LOOP}p3"
check "system partition took the whole card" test "$(size_gb "${LOOP}p2")" -ge 7
check "settings stay on the system partition" sh -c "! mountpoint -q $T/etc/ystreamer"
teardown

echo "== data partition damaged beyond repair"
setup 16
run >/dev/null
umount "$T/etc/ystreamer" "$T/var/lib/ystreamer" "$T/data"
dd if=/dev/urandom of="${LOOP}p3" bs=1M count=64 conv=notrunc 2>/dev/null
run
check "device still starts from the system partition" grep -q version "$T/etc/ystreamer/settings.json"
check "the damaged partition was not formatted over" test -z "$(blkid -o value -s LABEL "${LOOP}p3")"
check "settings stay on the system partition" sh -c "! mountpoint -q $T/etc/ystreamer"
teardown

exit $fail
