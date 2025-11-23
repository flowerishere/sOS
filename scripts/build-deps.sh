#!/usr/bin/env bash
set -euo pipefail

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"

mkdir -p "$base/build/bin"
rm -f "$base/build/bin"/*

pushd "$base/build" &>/dev/null || exit 1

if [ ! -f "aarch64-linux-musl-cross.tgz" ]
then
    wget https://musl.cc/aarch64-linux-musl-cross.tgz
    tar -xzf aarch64-linux-musl-cross.tgz
fi

popd &>/dev/null || exit 1

build=${build:-$(ls $base/scripts/deps)}

export PATH="$base/build/aarch64-linux-musl-cross/bin:$PATH"

for script in "$base/scripts/deps/"*
do
    [ -e "$script" ] || continue # skip if no file exists
    [ -x "$script" ] || continue # skip if not executable

    filename=$(basename "$script")

    if [[ "$filename" == _* ]]
    then
        echo "Skipping: $filename"
        continue
    fi

    # skip if not in build list
    if ! grep -qw "$filename" <<< "$build";
    then
        echo "Skipping: $filename"
        continue
    fi

    echo "Preparing: $filename"

    # make sure each script is run in the base directory
    pushd "$base" &>/dev/null || exit 1
    bash "$script"
    popd &>/dev/null || exit 1
done
