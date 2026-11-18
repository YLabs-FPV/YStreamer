#!/bin/bash -e

# pi-gen refreshes apt's package lists while exporting, after our stage has
# run. The updater fetches them again if a release ever needs a new package
on_chroot <<- EOF
	apt-get clean
EOF
find "${ROOTFS_DIR}/var/lib/apt/lists/" -type f -delete
test -z "$(find "${ROOTFS_DIR}/var/lib/apt/lists/" -type f)"
