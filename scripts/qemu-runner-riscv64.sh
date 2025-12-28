#!/usr/bin/env bash
set -e

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"

elf="$1"

qemu-system-riscv64 \
    -machine virt \
    -cpu rv64 \
    -nographic \
    -smp 4 \
    -m 128M \
    -bios default \
    -kernel target/riscv64gc-unknown-none-elf/debug/sos \
    -drive file=disk.img,if=none,format=raw,id=x0 \
    -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
    -device virtio-net-device,netdev=net0,bus=virtio-mmio-bus.1 \
    -netdev user,id=net0 \
    -d int,cpu_reset,guest_errors \
    -cpu rv64,svpbmt=true
