#!/usr/bin/env bash
# Build and deploy Lance Java to Baidu Maven.
# Usage:
#   ./publish-java.sh             # build both archs + deploy (default)
#   ./publish-java.sh x86_64      # build x86_64 only + deploy
#   ./publish-java.sh aarch64     # build aarch64 only + deploy
#   ./publish-java.sh all         # build both archs + deploy (same as default)
set -euxo pipefail

source "$(cd "$(dirname "$0")" && pwd)/cmc.sh"

install_rust
install_protoc
install_zig
install_cargo_zigbuild

resolve_targets "${1:-all}"

echo ">>> Deploying Lance Java to Baidu Maven..."

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
mvn --no-transfer-progress -U deploy -DskipTests -Dskip.build.jni=true -Drust.release.build=true -P deploy-to-baidu

echo ">>> Deploy complete."
