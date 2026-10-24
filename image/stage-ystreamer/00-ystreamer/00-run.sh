#!/bin/bash -e

# Only what YStreamer needs: no Recommends, no docs, man pages or other
# languages' messages, for this package and everything after it
install -m 644 /dev/stdin "${ROOTFS_DIR}/etc/apt/apt.conf.d/90-ystreamer" <<- EOF
	APT::Install-Recommends "false";
	APT::Install-Suggests "false";
	APT::AutoRemove::RecommendsImportant "false";
	APT::AutoRemove::SuggestsImportant "false";
EOF
install -m 644 /dev/stdin "${ROOTFS_DIR}/etc/dpkg/dpkg.cfg.d/90-ystreamer" <<- EOF
	path-exclude=/usr/share/doc/*
	path-include=/usr/share/doc/*/copyright
	path-exclude=/usr/share/man/*
	path-exclude=/usr/share/info/*
	path-exclude=/usr/share/lintian/*
	path-exclude=/usr/share/locale/*
	path-include=/usr/share/locale/en*
	path-include=/usr/share/locale/locale.alias
EOF

# Where the updater keeps the running version's package, to go back to if a
# later one doesn't start. Not /tmp: the chroot mounts its own over it
install -d "${ROOTFS_DIR}/var/lib/ystreamer/update"
install -m 644 files/ystreamer.deb "${ROOTFS_DIR}/var/lib/ystreamer/update/current.deb"
on_chroot <<- EOF
	apt-get install -y /var/lib/ystreamer/update/current.deb
EOF

# Tells YStreamer the whole device is its own
install -d "${ROOTFS_DIR}/etc/ystreamer"
touch "${ROOTFS_DIR}/etc/ystreamer/image"

# Tailscale isn't in Debian. so it comes installed, from
# its own repository
on_chroot <<- EOF
	curl -fsSL https://pkgs.tailscale.com/stable/debian/${RELEASE}.noarmor.gpg \
		-o /usr/share/keyrings/tailscale-archive-keyring.gpg
	curl -fsSL https://pkgs.tailscale.com/stable/debian/${RELEASE}.tailscale-keyring.list \
		-o /etc/apt/sources.list.d/tailscale.list
	apt-get update
	apt-get install -y tailscale
EOF

# Raspberry Pi OS Lite brings a compiler, a debugger, kernel headers, the
# Pi 5 kernel and firmware for Wi-Fi chips a Pi 4 doesn't have. None of it
# runs on a YStreamer, and it is most of the download
on_chroot <<- 'EOF'
	set -e
	purge=$(dpkg-query -W -f='${db:Status-Abbrev} ${Package}\n' \
		build-essential gcc g++ cpp gcc-14 g++-14 cpp-14 \
		gcc-aarch64-linux-gnu g++-aarch64-linux-gnu gcc-14-aarch64-linux-gnu \
		g++-14-aarch64-linux-gnu cpp-14-aarch64-linux-gnu libstdc++-14-dev libgcc-14-dev \
		binutils binutils-common gdb manpages-dev pkg-config python3-venv \
		'linux-headers-*' 'linux-image-*rpi-2712' \
		cloud-init rpi-cloud-init-mods rpi-connect-lite mkvtoolnix \
		firmware-atheros firmware-mediatek firmware-libertas 2>/dev/null |
		awk '/^ii/ { print $2 }')
	apt-get purge -y $purge
	apt-get autoremove --purge -y
	apt-get clean
EOF
rm -f "${ROOTFS_DIR}/boot/firmware/kernel_2712.img" "${ROOTFS_DIR}/boot/firmware/initramfs_2712"
# What earlier stages installed before the dpkg excludes were in place
find "${ROOTFS_DIR}/usr/share/doc" -mindepth 1 ! -type d ! -name copyright -delete
find "${ROOTFS_DIR}/usr/share/doc" -mindepth 1 -type d -empty -delete
rm -rf "${ROOTFS_DIR}"/usr/share/man/* "${ROOTFS_DIR}"/usr/share/info/* "${ROOTFS_DIR}"/usr/share/lintian/*
find "${ROOTFS_DIR}/usr/share/locale" -mindepth 1 -maxdepth 1 ! -name 'en*' ! -name locale.alias -exec rm -rf {} +

# An image without these would boot into nothing useful
grep -q '^dtoverlay=dwc2,dr_mode=peripheral' "${ROOTFS_DIR}/boot/firmware/config.txt"
test -L "${ROOTFS_DIR}/etc/systemd/system/multi-user.target.wants/ystreamer.service"
test -x "${ROOTFS_DIR}/usr/bin/wg"
test -x "${ROOTFS_DIR}/usr/bin/tailscale"
test -L "${ROOTFS_DIR}/etc/systemd/system/multi-user.target.wants/tailscaled.service"
for pkg in ystreamer network-manager openssh-server gstreamer1.0-plugins-bad gstreamer1.0-libav linux-image-rpi-v8 firmware-brcm80211; do
	dpkg-query --admindir="${ROOTFS_DIR}/var/lib/dpkg" -W -f='${db:Status-Abbrev}' "$pkg" | grep -q '^ii'
done
test ! -e "${ROOTFS_DIR}/usr/bin/gcc"
test ! -e "${ROOTFS_DIR}/boot/firmware/kernel_2712.img"
