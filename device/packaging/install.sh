#!/bin/sh
# Installs YStreamer on a Raspberry Pi that already runs Raspberry Pi OS:
#
#   curl -fsSL https://ystreamer.yarosfpv.com/install.sh | sudo sh
#
# Options, after `sh -s --` when piped:
#   --check   look the device over and verify the download, install nothing
#   --yes, -y don't ask before installing, or about a device that isn't supported
set -eu

MANIFEST_URL=${YSTREAMER_UPDATE_URL:-https://ystreamer.yarosfpv.com/update/manifest.json}
# The same key as update-key.pub, which the installed updater checks against
PUBLIC_KEY="PGdZ5XTTQ6j+TeKZfSTFty0xEbNqowNBXr4A9zPQvlo="
UPDATES=/var/lib/ystreamer/update
BOOT_CONFIG=/boot/firmware/config.txt

CHECK_ONLY=
ASSUME_YES=
for arg in "$@"; do
    case $arg in
    --check) CHECK_ONLY=1 ;;
    -y | --yes) ASSUME_YES=1 ;;
    *)
        echo "Unknown option $arg" >&2
        exit 2
        ;;
    esac
done

say() { printf '%s\n' "$*"; }
warn() { printf 'Warning: %s\n' "$*" >&2; }
die() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

# Asks on the terminal, since stdin is the script itself when piped from curl
confirm() {
    [ -z "$ASSUME_YES" ] || return 0
    [ -z "$CHECK_ONLY" ] || return 0
    if ! (: </dev/tty) 2>/dev/null; then
        die "${1:+$1 }Run again with --yes (or -y) to go ahead without being asked."
    fi
    printf '%s%s [y/N] ' "${1:+$1 }" "${2:-Continue anyway?}" >/dev/tty
    read -r answer </dev/tty
    case $answer in
    y | Y | yes) ;;
    *) die "Stopped." ;;
    esac
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "This needs $1, which isn't installed."
}

# A desktop that starts at boot holds the screen, so ours never gets it
boots_into_desktop() {
    [ "$(systemctl get-default 2>/dev/null)" = graphical.target ] &&
        systemctl is-enabled --quiet display-manager.service 2>/dev/null
}

desktop_warning() {
    say "WARNING: this system starts a desktop, which keeps the HDMI output for itself."
    say "YStreamer won't be able to show video on the screen. Use Raspberry Pi OS Lite,"
    say "or switch the desktop off with: sudo raspi-config (System Options > Boot)."
}

check_device() {
    arch=$(dpkg --print-architecture 2>/dev/null) || die "This isn't a Debian-based system."
    [ "$arch" = arm64 ] ||
        die "YStreamer needs the 64-bit Raspberry Pi OS; this system is $arch."

    model=$(tr -d '\0' </proc/device-tree/model 2>/dev/null || true)
    case $model in
    "Raspberry Pi 4 Model B"*) ;;
    *) confirm "YStreamer is only tested on the Raspberry Pi 4 Model B; this is ${model:-an unknown device}." ;;
    esac

    codename=$(. /etc/os-release 2>/dev/null && echo "${VERSION_CODENAME:-}")
    [ "$codename" = trixie ] ||
        confirm "YStreamer is only tested on Raspberry Pi OS 13 (trixie); this is ${codename:-an unknown release}."

    [ -f "$BOOT_CONFIG" ] ||
        warn "$BOOT_CONFIG is missing, so the USB-C port can't be set up for the goggles."

    # Applies on the Compute Module 4 only, where it keeps the port a host
    if [ -f "$BOOT_CONFIG" ] && case $model in *"Compute Module 4"*) true ;; *) false ;; esac &&
        grep -q '^otg_mode=1' "$BOOT_CONFIG"; then
        warn "otg_mode=1 in $BOOT_CONFIG keeps the USB port in host mode; remove it for the goggles to connect."
    fi

    # Something else on port 80 would stop the web interface from starting
    if ! systemctl is-active --quiet ystreamer 2>/dev/null &&
        ss -Hltn 'sport = :80' 2>/dev/null | grep -q .; then
        warn "Another program is already listening on port 80, which YStreamer's web interface uses."
    fi
}

# Prints: version, package URL, signature
read_manifest() {
    python3 - "$1" "$2" <<'PY'
import json, sys
manifest = json.load(open(sys.argv[1]))
package = manifest["packages"].get(sys.argv[2])
if not package:
    sys.exit(f"Version {manifest['version']} has no package for {sys.argv[2]} devices")
print(manifest["version"], package["url"], package["signature"])
PY
}

# Ed25519 over the whole file, as packaging/sign.sh makes it
verify() {
    {
        printf '\060\052\060\005\006\003\053\145\160\003\041\000'
        printf '%s' "$PUBLIC_KEY" | base64 -d
    } | openssl pkey -pubin -inform DER -out "$WORK/key.pem"
    printf '%s' "$2" | base64 -d >"$WORK/signature"
    openssl pkeyutl -verify -pubin -inkey "$WORK/key.pem" -rawin \
        -in "$1" -sigfile "$WORK/signature" >/dev/null 2>&1
}

# The boot settings the package would add, going by the list it carries
missing_boot_settings() {
    [ -f "$BOOT_CONFIG" ] || return 0
    dpkg-deb --fsys-tarfile "$1" | tar -xO ./usr/share/ystreamer/config.txt 2>/dev/null |
        while read -r line; do
            case $line in '' | '#'*) continue ;; esac
            grep -qxF "$line" "$BOOT_CONFIG" || printf '      %s\n' "$line"
        done
}

# Says what's about to change on this system, and waits for a yes
approve() {
    say
    case $change in
    install)
        say "YStreamer $version is ready to install. This will:"
        say "  - install the package and what it needs, such as GStreamer"
        say "  - start YStreamer now and at every boot, using port 80 and the HDMI output"
        ;;
    reinstall)
        say "YStreamer $version is already installed. Running this again will:"
        say "  - reinstall the same version"
        ;;
    update)
        say "YStreamer $installed is installed. This will:"
        say "  - update it to $version"
        ;;
    downgrade)
        say "WARNING: YStreamer $installed is installed, which is newer than the latest"
        say "release. This will:"
        say "  - go back to $version"
        ;;
    esac
    if [ "$change" != install ]; then
        say "  - restart YStreamer, which finishes an open recording and pauses the"
        say "    streams and the HDMI output for a few seconds"
        say "  - keep its settings, login and recordings"
    fi
    settings=$(missing_boot_settings "$1")
    if [ -n "$settings" ]; then
        say "  - change $BOOT_CONFIG, keeping a copy of the original, by adding:"
        say "$settings"
        say
        say "WARNING: the USB-C port will be reserved for the goggles until YStreamer is"
        say "uninstalled. It can still power the Pi, but nothing else plugged into it will"
        say "work. This takes effect after a reboot."
    else
        say "  - leave $BOOT_CONFIG as it is: it already has what YStreamer needs"
    fi
    if boots_into_desktop; then
        say
        desktop_warning
    fi
    say
    if [ "$change" = downgrade ]; then
        # --yes covers the usual questions, not quietly going back a version
        (: </dev/tty) 2>/dev/null ||
            die "Going back to an older version has to be confirmed; run this from a terminal."
        ASSUME_YES=
        confirm "" "Go back to $version?"
    else
        confirm "" "Continue?"
    fi
}

for tool in curl python3 openssl base64 dpkg tar; do
    need "$tool"
done
if [ -z "$CHECK_ONLY" ] && [ "$(id -u)" != 0 ]; then
    die "Run this as root, for example with sudo."
fi

check_device

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

say "Looking up the latest version..."
curl -fsSL "$MANIFEST_URL" -o "$WORK/manifest.json" ||
    die "Couldn't reach the update server at $MANIFEST_URL."
release=$(read_manifest "$WORK/manifest.json" "$arch") || die "The update server sent something unexpected."
set -- $release
version=$1
url=$2
signature=$3

say "Downloading YStreamer $version..."
DEB=$WORK/ystreamer.deb
curl -fsSL "$url" -o "$DEB" || die "Couldn't download $url."
verify "$DEB" "$signature" ||
    die "The download's signature doesn't match, so it wasn't installed."
[ "$(dpkg-deb -f "$DEB" Package)" = ystreamer ] || die "The download isn't YStreamer."
say "Signature verified."

# What's on the device already decides between installing, updating,
# reinstalling and going back to an older version
installed=$(dpkg-query -W -f='${db:Status-Status} ${Version}' ystreamer 2>/dev/null || true)
case $installed in
"installed "*)
    installed=${installed#installed }
    if dpkg --compare-versions "$installed" eq "$(dpkg-deb -f "$DEB" Version)"; then
        change=reinstall
    elif dpkg --compare-versions "$installed" lt "$(dpkg-deb -f "$DEB" Version)"; then
        change=update
    else
        change=downgrade
    fi
    installed=${installed%-*}
    ;;
*) change=install ;;
esac

if [ -n "$CHECK_ONLY" ]; then
    if boots_into_desktop; then
        desktop_warning
    fi
    case $change in
    install) say "This device is ready for YStreamer $version. Nothing was installed." ;;
    reinstall) say "YStreamer $version is already installed, the latest version. Nothing was changed." ;;
    update) say "YStreamer $installed is installed; running this would update it to $version. Nothing was changed." ;;
    downgrade) say "YStreamer $installed is installed, newer than the latest release ($version). Nothing was changed." ;;
    esac
    exit 0
fi

approve "$DEB"

before=$(cksum 2>/dev/null <"$BOOT_CONFIG" || true)
# _apt can't read our private folder, and apt would only say so
chmod 755 "$WORK"
chmod 644 "$DEB"
say "Installing..."
apt-get update -q
DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades "$DEB"

# The updater goes back to current.deb if a later version doesn't start, and
# System → Updates can roll back to previous.deb, as after an update from there
mkdir -p "$UPDATES"
if [ "$change" != reinstall ] && [ -f "$UPDATES/current.deb" ]; then
    mv -f "$UPDATES/current.deb" "$UPDATES/previous.deb"
fi
cp "$DEB" "$UPDATES/current.deb"

# Port 80, or 8080 when something else had 80. Asking for the version means
# the one answering is the one just installed
answering() {
    for port in 80 8080; do
        curl -fs --max-time 2 "http://localhost:$port/health" 2>/dev/null |
            grep -qF "\"version\":\"$version\"" && return 0
    done
    return 1
}
tries=0
until answering; do
    tries=$((tries + 1))
    if [ $tries -ge 30 ]; then
        warn "YStreamer was installed but isn't answering. See: journalctl -u ystreamer"
        exit 1
    fi
    sleep 2
done
address=http://$(hostname).local
[ "$port" = 80 ] || address=$address:$port

say
say "YStreamer $version is running: $address"
if [ "$before" != "$(cksum 2>/dev/null <"$BOOT_CONFIG" || true)" ]; then
    say "Reboot before connecting the goggles: the USB-C port was just set up for them."
fi
