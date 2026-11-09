#!/usr/bin/env bash
# New compile-time headers, with the distro's loader/runtime and glibc unchanged.
set -euo pipefail
prefix=${1:?SDK installation prefix required}
source_dir=$(mktemp -d)
trap 'rm -rf "$source_dir"' EXIT
curl -fL --retry 3 https://github.com/KhronosGroup/Vulkan-Headers/archive/refs/tags/v1.4.304.tar.gz | tar -xz --strip-components=1 -C "$source_dir"
cmake -S "$source_dir" -B "$source_dir/build" -DCMAKE_INSTALL_PREFIX="$prefix" -DVULKAN_HEADERS_ENABLE_TESTS=OFF
cmake --install "$source_dir/build"
