#!/bin/sh
set -eu

target_arch="$1"
rust_target="$2"
sysroot=/opt/sysroot

# Bootstrap the merged /usr layout before RPMs install files under /lib64.
mkdir -p "$sysroot/etc/pki/rpm-gpg"
for directory in bin sbin lib lib64; do
    mkdir -p "$sysroot/usr/$directory"
    ln -s "usr/$directory" "$sysroot/$directory"
done
cp /etc/pki/rpm-gpg/RPM-GPG-KEY-redhat-release "$sysroot/etc/pki/rpm-gpg/"

# Install target headers and libraries without executing target-architecture scripts.
dnf install -y --installroot="$sysroot" --releasever=10 \
    --forcearch="$target_arch" --setopt=tsflags=noscripts \
    --setopt=install_weak_deps=False \
    gcc glibc-devel libstdc++-devel openssl-devel
dnf clean all --installroot="$sysroot" --releasever=10 --forcearch="$target_arch"

# Absolute symlinks must resolve inside the sysroot, not to the host's libraries.
python3 - "$sysroot" <<'PY'
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
for path in root.rglob("*"):
    if path.is_symlink():
        target = os.readlink(path)
        if os.path.isabs(target):
            destination = os.path.relpath(root / target.lstrip("/"), path.parent)
            path.unlink()
            path.symlink_to(destination)
PY

rustup target add "$rust_target"
printf '#!/bin/sh\nexec clang --target=%s --sysroot=/opt/sysroot -fuse-ld=lld "$@"\n' \
    "$rust_target" > /usr/local/bin/cross-cc
printf '#!/bin/sh\nexec clang++ --target=%s --sysroot=/opt/sysroot -fuse-ld=lld "$@"\n' \
    "$rust_target" > /usr/local/bin/cross-cxx
chmod +x /usr/local/bin/cross-cc /usr/local/bin/cross-cxx
