#!/usr/bin/env bash
# ============================================================================
# DevRunner 一键构建启动脚本
#
# 使用方式:
#   ./run.sh              # 构建 Rust + Swift，kill 旧进程，启动
#   ./run.sh --swift-only # 仅重新构建 Swift（Rust 没改时）
#   ./run.sh --kill       # 仅停止
# ============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETERM_ROOT="$(dirname "$SCRIPT_DIR")"

DERIVED_DATA="$HOME/.vimo/dev-runner/DerivedData/DevRunner"
APP_PATH="$DERIVED_DATA/Build/Products/Debug/DevRunner.app"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[DevRunner]${NC} $*"; }
log_success() { echo -e "${GREEN}[DevRunner]${NC} $*"; }
log_error() { echo -e "${RED}[DevRunner]${NC} $*"; }

kill_dev_runner() {
    if pgrep -f "DevRunner.app/Contents/MacOS/DevRunner" > /dev/null 2>&1; then
        log_info "Stopping DevRunner..."
        pkill -f "DevRunner.app/Contents/MacOS/DevRunner"
        sleep 1
    else
        log_info "DevRunner not running"
    fi
}

build_rust() {
    log_info "Building Rust core..."
    cd "$SCRIPT_DIR/app"
    cargo build --release

    local DYLIB="$SCRIPT_DIR/app/target/release/libdev_runner_app.dylib"
    if [ ! -f "$DYLIB" ]; then
        log_error "dylib not found: $DYLIB"
        exit 1
    fi

    log_info "Copying dylib..."
    mkdir -p "$SCRIPT_DIR/swift-app/Libs"
    cp "$DYLIB" "$SCRIPT_DIR/swift-app/Libs/"
}

build_swift() {
    log_info "Building DevRunner.app..."
    xcodebuild -project "$SCRIPT_DIR/swift-app/DevRunner.xcodeproj" \
        -scheme DevRunner \
        -configuration Debug \
        -derivedDataPath "$DERIVED_DATA" \
        build 2>&1 | tail -5
}

launch() {
    log_info "Launching DevRunner..."
    open "$APP_PATH"
    log_success "DevRunner started: $APP_PATH"
}

case "${1:---full}" in
    --kill|-k)
        kill_dev_runner
        ;;
    --swift-only|-s)
        build_swift
        kill_dev_runner
        launch
        ;;
    *)
        build_rust
        build_swift
        kill_dev_runner
        launch
        ;;
esac
