#!/usr/bin/env bash
set -e

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"

elf="$1"
bin="${elf%.elf}.bin"
aarch64-none-elf-objcopy -O binary "$elf" "$bin"

qemu-system-aarch64 \
    -machine virt,gic-version=3 \
    -cpu cortex-a72 \
    -smp 4 \
    -m 2G \
    -nographic \
    -initrd "$base/moss.img" \
    -s \
    -kernel "$bin"