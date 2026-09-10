#!/usr/bin/env bash
# Common build environment setup for Lance CI.
# Usage: source ./cmc.sh
set -euo pipefail

export LC_ALL=en_US.UTF-8
export LANG=en_US.UTF-8
export LANGUAGE=en_US.UTF-8

export PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 使用百度官方代理
export proxy="http://agent.baidu.com:8891"
export http_proxy=$proxy
export https_proxy=$proxy
export no_proxy=127.0.0.1,localhost,local,.local,172.0.0.0/24,10.0.0.0/24,.baidu.com,.baidu-int.com
export HTTP_PROXY=$proxy
export HTTPS_PROXY=$proxy
export NO_PROXY=$no_proxy

# ----------------------------------------------------------
# resolve_targets: 解析目标架构列表
#   参数: $1 = "x86_64" | "aarch64" | "all" (默认 "all")
#   输出: 设置全局数组 TARGETS
# ----------------------------------------------------------
resolve_targets() {
    local input="${1:-all}"
    case "$input" in
        all)    TARGETS=(x86_64 aarch64) ;;
        x86_64) TARGETS=(x86_64) ;;
        aarch64) TARGETS=(aarch64) ;;
        *)
            echo "ERROR: unsupported target: $input (expected x86_64|aarch64|all)"
            exit 1
            ;;
    esac
}

# ----------------------------------------------------------
# arch_to_rust_target: 架构 -> Rust target triple
# ----------------------------------------------------------
arch_to_rust_target() {
    case "$1" in
        x86_64)  echo "x86_64-unknown-linux-gnu" ;;
        aarch64) echo "aarch64-unknown-linux-gnu" ;;
    esac
}

# ----------------------------------------------------------
# arch_to_nativelib_dir: 架构 -> JNI nativelib 目录名
# ----------------------------------------------------------
arch_to_nativelib_dir() {
    case "$1" in
        x86_64)  echo "linux-x86-64" ;;
        aarch64) echo "linux-aarch64" ;;
    esac
}

# ----------------------------------------------------------
# retry: 带重试的命令执行
#   参数: $1 = 最大重试次数, $2.. = 要执行的命令
# ----------------------------------------------------------
retry() {
    local max_attempts="$1"; shift
    local attempt=1
    while true; do
        if "$@"; then
            return 0
        fi
        if [ "$attempt" -ge "$max_attempts" ]; then
            echo "ERROR: command failed after ${max_attempts} attempts: $*"
            return 1
        fi
        echo ">>> Attempt ${attempt}/${max_attempts} failed, retrying in 5s..."
        sleep 5
        ((attempt++))
    done
}

# ----------------------------------------------------------
# install_rust: 安装 Rust 工具链（crates.io / rustup 走百度内网 rsproxy 镜像）
#   镜像配置见 https://rsproxy.now.baidu.com/guide/config.html
#   不指定版本，cargo build 时由 rust-toolchain.toml 自动选择
# ----------------------------------------------------------
install_rust() {
    # toolchain manifest 仍走 rsproxy.cn（原有配置，经公司 HTTP 代理访问）：
    # 内网 rsproxy 的 /dist 只提供 channel-rust-stable.toml，而 rust-toolchain.toml
    # 固定了具体版本，取 channel-rust-<version>.toml 会返回镜像站首页 HTML，
    # rustup 校验和失败并报 "toolchain is not installable"。
    export RUSTUP_DIST_SERVER="https://rsproxy.cn"
    export RUSTUP_UPDATE_ROOT="https://rsproxy.now.baidu-int.com/rustup"

    if ! command -v rustup &>/dev/null; then
        echo ">>> Installing rustup (baidu rsproxy mirror)..."
        retry 3 curl --proto '=https' --tlsv1.2 -sSf -o /tmp/rustup-init.sh https://rsproxy.now.baidu-int.com/rustup-init.sh
        sh /tmp/rustup-init.sh -y --default-toolchain none
        rm -f /tmp/rustup-init.sh
    fi
    source "$HOME/.cargo/env"

    # 添加双架构 target（交叉编译场景需要；native 编译时为 no-op）
    rustup target add aarch64-unknown-linux-gnu 2>/dev/null || true
    rustup target add x86_64-unknown-linux-gnu 2>/dev/null || true

    # 配置 crates.io 内网镜像（百度 rsproxy），避免 cargo build 下载依赖超时
    mkdir -p "$HOME/.cargo"
    cat > "$HOME/.cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = 'baidu-sparse'
[source.baidu]
registry = "https://rsproxy.now.baidu-int.com/crates.io-index"
[source.baidu-sparse]
registry = "sparse+https://rsproxy.now.baidu-int.com/index/"
[registries.baidu]
index = "https://rsproxy.now.baidu-int.com/crates.io-index"
[net]
git-fetch-with-cli = true
EOF
    echo ">>> Cargo registry mirror: rsproxy.now.baidu-int.com"
    echo ">>> Rust toolchain will be auto-selected by rust-toolchain.toml"
}

# ----------------------------------------------------------
# install_protoc: 安装 protobuf 编译器（prost-build 需要）
#   protoc 是 host 工具，始终安装 host 架构版本
# ----------------------------------------------------------
install_protoc() {
    if command -v protoc &>/dev/null; then
        echo ">>> protoc already installed: $(protoc --version)"
        return
    fi
    echo ">>> Installing protoc 3.15.0..."
    local PROTOC_ZIP="protoc-3.15.0-linux-x86_64.zip"
    retry 3 curl --noproxy '*' -sSL -o "${PROTOC_ZIP}" \
        "http://filecenter.matrix.baidu.com/api/v1/file/lance/${PROTOC_ZIP}/20260410083811/download"
    unzip -oq "${PROTOC_ZIP}" -d /usr/local
    rm -f "${PROTOC_ZIP}"
    echo ">>> protoc installed: $(protoc --version)"
}

# ----------------------------------------------------------
# install_maturin: 安装 maturin（Python wheel 构建工具）
# ----------------------------------------------------------
install_maturin() {
    if command -v maturin &>/dev/null; then
        echo ">>> maturin already installed: $(maturin --version)"
        return
    fi
    echo ">>> Installing maturin..."
    retry 3 pip3 install maturin
    echo ">>> maturin installed: $(maturin --version)"
}

# ----------------------------------------------------------
# install_zig: 安装 zig
#   作为 cargo-zigbuild 的后端编译器/链接器，支持交叉编译和兼容老 glibc
# ----------------------------------------------------------
install_zig() {
    local ZIG_VER="0.15.2"
    local ZIG_ARCHIVE="zig-x86_64-linux-${ZIG_VER}.tar.xz"
    local ZIG_DIR="/opt/zig-x86_64-linux-${ZIG_VER}"

    if [ ! -d "${ZIG_DIR}" ]; then
        echo ">>> Installing zig ${ZIG_VER}..."
        retry 3 curl --noproxy '*' -sSL -o "${ZIG_ARCHIVE}" \
            "http://filecenter.matrix.baidu.com/api/v1/file/lance/${ZIG_ARCHIVE}/20260410084246/download"
        tar -xJf "${ZIG_ARCHIVE}" -C /opt
        rm -f "${ZIG_ARCHIVE}"
    fi
    export PATH="${ZIG_DIR}:$PATH"
    echo ">>> zig $(zig version)"
}

# ----------------------------------------------------------
# install_cargo_zigbuild: 安装 cargo-zigbuild（Rust 交叉编译工具）
#   自动管理交叉编译的 CC/CXX/linker，无需手动设置环境变量
# ----------------------------------------------------------
install_cargo_zigbuild() {
    if command -v cargo-zigbuild &>/dev/null; then
        echo ">>> cargo-zigbuild already installed"
        return
    fi
    echo ">>> Installing cargo-zigbuild..."
    retry 3 cargo install cargo-zigbuild --locked
    echo ">>> cargo-zigbuild installed"
}
