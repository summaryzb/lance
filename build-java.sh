#!/usr/bin/env bash
# Build Lance Java (Rust JNI + Maven) and collect artifacts.
# Usage:
#   ./build-java.sh              # build for both x86_64 and aarch64 (default)
#   ./build-java.sh x86_64       # build for x86_64 only
#   ./build-java.sh aarch64      # build for aarch64 only
#   ./build-java.sh all          # build for both (same as default)
set -euxo pipefail

source "$(cd "$(dirname "$0")" && pwd)/cmc.sh"

install_rust
install_protoc
install_zig
install_cargo_zigbuild

resolve_targets "${1:-all}"

echo ">>> Building Lance Java..."

for arch in "${TARGETS[@]}"; do
    local_rust_target="$(arch_to_rust_target "$arch")"
    local_nativelib_dir="$(arch_to_nativelib_dir "$arch")"

    echo ">>> Building JNI for ${arch}..."

    cd "${PROJECT_ROOT}/java/lance-jni"
    cargo zigbuild --release --target "${local_rust_target}.2.17"

    dest="${PROJECT_ROOT}/java/target/classes/nativelib/${local_nativelib_dir}"
    mkdir -p "$dest"
    cp "${PROJECT_ROOT}/java/lance-jni/target/${local_rust_target}/release/liblance_jni.so" \
       "$dest/liblance_jni.so"
    echo ">>> Placed: ${dest}/liblance_jni.so"
done

cd "${PROJECT_ROOT}/java"
mvn --no-transfer-progress -U install -DskipTests -Dskip.build.jni=true -Drust.release.build=true

"${PROJECT_ROOT}/collect-output.sh" java
echo ">>> Build complete."
