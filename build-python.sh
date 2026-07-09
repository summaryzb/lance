#!/usr/bin/env bash
# Build Lance Python wheel and collect artifacts.
# Usage:
#   ./build-python.sh              # build for both x86_64 and aarch64 (default)
#   ./build-python.sh x86_64       # build for x86_64 only
#   ./build-python.sh aarch64      # build for aarch64 only
#   ./build-python.sh all          # build for both (same as default)
set -euxo pipefail

source "$(cd "$(dirname "$0")" && pwd)/cmc.sh"

install_rust
install_protoc
install_maturin
install_zig

resolve_targets "${1:-all}"

echo ">>> Building Lance Python wheel..."

# 清理旧 wheel，避免 collect-output 打包历史产物
rm -f "${PROJECT_ROOT}/python/wheels"/pylance-*.whl

for arch in "${TARGETS[@]}"; do
    local_rust_target="$(arch_to_rust_target "$arch")"

    echo ">>> Building wheel for ${arch}..."

    cd "${PROJECT_ROOT}/python"
    maturin build --release --strip --zig \
        --target "${local_rust_target}" \
        --compatibility manylinux2014 \
        --out wheels
done

"${PROJECT_ROOT}/collect-output.sh" python
echo ">>> Build complete."
