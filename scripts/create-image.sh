#!/usr/bin/env bash
set -euo pipefail

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"
pushd "$base" &>/dev/null || exit 1

img="$base/moss.img"
mount="$base/build/mount"

mkdir -p "$mount"

dd if=/dev/zero of="$img" bs=1M count=128
mkfs.vfat -F 32 "$img"

if ! mountpoint -q "$mount"; then
    mount -o loop "$img" "$mount"
fi

mkdir -p "$mount/bin"
mkdir -p "$mount/dev"

cp "$base/build/bin"/* "$mount/bin"

if mountpoint -q "$mount"; then
    umount "$mount"
fi
popd &>/dev/null || exit 1