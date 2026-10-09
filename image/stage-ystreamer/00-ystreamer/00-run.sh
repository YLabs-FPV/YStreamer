#!/bin/bash -e

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

# An image without these would boot into nothing useful
grep -q '^dtoverlay=dwc2,dr_mode=peripheral' "${ROOTFS_DIR}/boot/firmware/config.txt"
test -L "${ROOTFS_DIR}/etc/systemd/system/multi-user.target.wants/ystreamer.service"
test -x "${ROOTFS_DIR}/usr/bin/wg"
test -x "${ROOTFS_DIR}/usr/bin/tailscale"
test -L "${ROOTFS_DIR}/etc/systemd/system/multi-user.target.wants/tailscaled.service"
