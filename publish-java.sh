#!/usr/bin/env bash
# Build and deploy Lance Java to Baidu Maven.
# Usage:
#   ./publish-java.sh             # deploy with both archs (default)
#   ./publish-java.sh x86_64      # deploy with x86_64 only
#   ./publish-java.sh aarch64     # deploy with aarch64 only
#   ./publish-java.sh all         # deploy with both archs (same as default)
#
# TEMPORARY: the Rust JNI build is skipped here; liblance_jni.so is taken from
# an already published jar (see PREBUILT_JAR_URL). To restore a full build, drop
# the download/extract block below and bring back the `cargo zigbuild` loop plus
# the install_rust/install_protoc/install_zig/install_cargo_zigbuild calls.
set -euxo pipefail

source "$(cd "$(dirname "$0")" && pwd)/cmc.sh"

PREBUILT_JAR_URL="http://10.11.56.212:8765/databuilder/lance/11.0.0.1-baidu/lance-core-11.0.0.1-baidu-SNAPSHOT.jar"

resolve_targets "${1:-all}"

echo ">>> Deploying Lance Java to Baidu Maven..."

prebuilt_jar="${PROJECT_ROOT}/java/target/prebuilt-lance-core.jar"
classes_dir="${PROJECT_ROOT}/java/target/classes"
mkdir -p "${PROJECT_ROOT}/java/target" "${classes_dir}"

echo ">>> Downloading prebuilt jar: ${PREBUILT_JAR_URL}"
retry 3 curl --noproxy '*' -sSL -o "${prebuilt_jar}" "${PREBUILT_JAR_URL}"

for arch in "${TARGETS[@]}"; do
    local_nativelib_dir="$(arch_to_nativelib_dir "$arch")"
    so_path="nativelib/${local_nativelib_dir}/liblance_jni.so"

    echo ">>> Extracting ${so_path} for ${arch}..."
    unzip -oq "${prebuilt_jar}" "${so_path}" -d "${classes_dir}"

    if [ ! -s "${classes_dir}/${so_path}" ]; then
        echo "ERROR: ${so_path} not found in ${PREBUILT_JAR_URL}"
        exit 1
    fi
    echo ">>> Placed: ${classes_dir}/${so_path}"
done

rm -f "${prebuilt_jar}"

cd "${PROJECT_ROOT}/java"
mvn --no-transfer-progress -U deploy -DskipTests -Dskip.build.jni=true -Drust.release.build=true -P deploy-to-baidu

echo ">>> Deploy complete."
