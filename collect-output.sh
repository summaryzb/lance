#!/usr/bin/env bash
# Collect build artifacts into output/ for CI release.
# Usage: ./collect-output.sh [java|python]
#   No argument: collect all available artifacts.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT="${SCRIPT_DIR}/output"

COMPONENTS=("$@")
if [ ${#COMPONENTS[@]} -eq 0 ]; then
    COMPONENTS=(java python)
fi

for comp in "${COMPONENTS[@]}"; do
    case "$comp" in
        java)
            rm -rf "${OUTPUT}/java"
            mkdir -p "${OUTPUT}/java"
            cp "${SCRIPT_DIR}"/java/target/lance-core-*.jar "${OUTPUT}/java/"
            ;;
        python)
            rm -rf "${OUTPUT}/python"
            mkdir -p "${OUTPUT}/python"
            wheel_dir="${SCRIPT_DIR}/python/wheels"
            # Extract version from the first .whl filename: pylance-<version>-*.whl
            version=$(ls "${wheel_dir}"/pylance-*.whl | head -1 | sed 's|.*/pylance-\([^-]*\)-.*|\1|')
            tar -czf "${OUTPUT}/python/pylance-${version}.tar.gz" -C "${wheel_dir}" .
            ;;
        *)
            echo "ERROR: unknown component: $comp"
            exit 1
            ;;
    esac
done

echo ">>> Artifacts collected to ${OUTPUT}:"
find "${OUTPUT}" -type f | sort
echo "make the whole output as tar"
tar -zcf "${OUTPUT}.tar.gz" -C "${OUTPUT}" .
