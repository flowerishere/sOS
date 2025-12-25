#!/usr/bin/env bash
set -e

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"

elf="$1"

qemu-system-riscv64 \
    -machine virt \
    -cpu rv64 \
    -smp 4 \
    -m 128M \
    -drive if=none,format=raw,file="$base/hdd.dsk",id=foo \
    -device virtio-blk-device,scsi=off,drive=foo \
    -nographic \
    -serial mon:stdio \
    -bios default \
    -device virtio-rng-device \
    -device virtio-gpu-device \
    -device virtio-net-device \
    -device virtio-tablet-device \
    -device virtio-keyboard-device \
    -kernel "$elf"\
    -nographic\
