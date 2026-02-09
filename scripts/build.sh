#!/usr/bin/env bash
# ============================================================================
# ETerm 统一编译脚本
#
# 编译所有 Rust 组件并部署到 Swift 插件目录
#
# 使用方式:
#   ./scripts/build.sh           # 编译所有
#   ./scripts/build.sh etermkit  # 只编译 ETermKit SDK
#   ./scripts/build.sh ffi       # 只编译 ai-cli-session-db FFI
#   ./scripts/build.sh socket    # 只编译 socket-client-ffi
#   ./scripts/build.sh vlaude-ffi # 只编译 vlaude-ffi (数据查询 API)
#   ./scripts/build.sh memex     # 只编译 memex
#   ./scripts/build.sh sugarloaf # 只编译 sugarloaf-ffi
#   ./scripts/build.sh mcp-router # 只编译 mcp-router-core
#   ./scripts/build.sh agent     # 只编译 vimo-agent
#   ./scripts/build.sh pty-daemon # 只编译 pty-daemon
#   ./scripts/build.sh plugins   # 只构建 Swift 插件
#   ./scripts/build.sh lint      # 运行 clippy 检查所有 Rust 项目
#   ./scripts/build.sh check     # 只运行事件一致性检查
# ============================================================================
set -e

# 从脚本位置推断项目根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETERM_ROOT="$(dirname "$SCRIPT_DIR")"

# 目录定义
ETERMKIT_PKG="$ETERM_ROOT/ETerm/Packages/ETermKit"
ETERMKIT_FRAMEWORK="$ETERM_ROOT/ETerm/Build/ETermKit.framework"
CLAUDE_SESSION_DB="$ETERM_ROOT/ai-cli-session-db"
MEMEX_RS="$ETERM_ROOT/memex/memex-rs"
MEMEX_WEB_SRC="$ETERM_ROOT/memex/web"
MEMEX_WEB_DEST="$HOME/.vimo/memex/web"
VLAUDE_CORE="$ETERM_ROOT/vlaude/packages/vlaude-core"
ETERM_DIR="$ETERM_ROOT/ETerm"
RIO_DIR="$ETERM_DIR/rio"
VLAUDE_KIT="$ETERM_DIR/Plugins/VlaudeKit"
MEMEX_KIT="$ETERM_DIR/Plugins/MemexKit"
LSP_KIT="$ETERM_DIR/Plugins/LspKit"
MCP_ROUTER="$ETERM_ROOT/mcp-router/core"
MCP_ROUTER_KIT="$ETERM_DIR/Plugins/MCPRouterKit"
DEV_RUNNER="$ETERM_ROOT/dev-runner"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[ETerm]${NC} $*"; }
log_success() { echo -e "${GREEN}[ETerm]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[ETerm]${NC} $*"; }
log_error() { echo -e "${RED}[ETerm]${NC} $*"; }

# ============================================================================
# 编译 ETermKit SDK 并打包成 Framework
# ============================================================================
build_etermkit() {
    log_info "Building ETermKit SDK..."

    cd "$ETERMKIT_PKG"

    # 编译 release 版本
    swift build -c release

    local BUILD_DIR="$ETERMKIT_PKG/.build/release"
    local DYLIB="$BUILD_DIR/libETermKit.dylib"

    if [ ! -f "$DYLIB" ]; then
        log_error "ETermKit dylib not found: $DYLIB"
        exit 1
    fi

    # 创建 framework 结构（模仿 Xcode：swiftmodule 在 framework 外面）
    log_info "Packaging into framework..."
    local BUILD_OUTPUT="$ETERM_DIR/Build"
    rm -rf "$ETERMKIT_FRAMEWORK"
    rm -rf "$BUILD_OUTPUT/ETermKit.swiftmodule"

    mkdir -p "$ETERMKIT_FRAMEWORK/Versions/A/Resources"
    mkdir -p "$ETERMKIT_FRAMEWORK/Versions/A/Modules/ETermKit.swiftmodule"
    mkdir -p "$BUILD_OUTPUT/ETermKit.swiftmodule"

    # 复制 dylib
    cp "$DYLIB" "$ETERMKIT_FRAMEWORK/Versions/A/ETermKit"

    # 获取当前架构
    local ARCH=$(uname -m)
    local TRIPLE="${ARCH}-apple-macosx"

    # 复制 swiftmodule 到 framework 内部（Swift PM 需要）
    # 同时提供 macosx 和 macos 两种命名（不同工具链可能使用不同的 triple）
    local TRIPLE_X="${ARCH}-apple-macosx"
    local TRIPLE_NO_X="${ARCH}-apple-macos"

    for triple in "$TRIPLE_X" "$TRIPLE_NO_X"; do
        cp "$BUILD_DIR/Modules/ETermKit.swiftmodule" "$ETERMKIT_FRAMEWORK/Versions/A/Modules/ETermKit.swiftmodule/${triple}.swiftmodule"
        cp "$BUILD_DIR/Modules/ETermKit.swiftdoc" "$ETERMKIT_FRAMEWORK/Versions/A/Modules/ETermKit.swiftmodule/${triple}.swiftdoc"
        cp "$BUILD_DIR/Modules/ETermKit.abi.json" "$ETERMKIT_FRAMEWORK/Versions/A/Modules/ETermKit.swiftmodule/${triple}.abi.json"
        if [ -f "$BUILD_DIR/Modules/ETermKit.swiftsourceinfo" ]; then
            cp "$BUILD_DIR/Modules/ETermKit.swiftsourceinfo" "$ETERMKIT_FRAMEWORK/Versions/A/Modules/ETermKit.swiftmodule/${triple}.swiftsourceinfo"
        fi

        # 同时复制到 framework 外面（兼容旧方式）
        cp "$BUILD_DIR/Modules/ETermKit.swiftmodule" "$BUILD_OUTPUT/ETermKit.swiftmodule/${triple}.swiftmodule"
        cp "$BUILD_DIR/Modules/ETermKit.swiftdoc" "$BUILD_OUTPUT/ETermKit.swiftmodule/${triple}.swiftdoc"
        cp "$BUILD_DIR/Modules/ETermKit.abi.json" "$BUILD_OUTPUT/ETermKit.swiftmodule/${triple}.abi.json"
        if [ -f "$BUILD_DIR/Modules/ETermKit.swiftsourceinfo" ]; then
            cp "$BUILD_DIR/Modules/ETermKit.swiftsourceinfo" "$BUILD_OUTPUT/ETermKit.swiftmodule/${triple}.swiftsourceinfo"
        fi
    done

    # 创建 Info.plist
    cat > "$ETERMKIT_FRAMEWORK/Versions/A/Resources/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.vimo.ETermKit</string>
    <key>CFBundleName</key>
    <string>ETermKit</string>
    <key>CFBundlePackageType</key>
    <string>FMWK</string>
</dict>
</plist>
EOF

    # 修改 install name（在创建符号链接前）
    install_name_tool -id "@rpath/ETermKit.framework/ETermKit" "$ETERMKIT_FRAMEWORK/Versions/A/ETermKit"

    # 创建符号链接
    cd "$ETERMKIT_FRAMEWORK/Versions"
    ln -sfh A Current
    cd "$ETERMKIT_FRAMEWORK"
    ln -sfh Versions/Current/ETermKit ETermKit
    ln -sfh Versions/Current/Resources Resources
    ln -sfh Versions/Current/Modules Modules

    # 签名整个 framework
    codesign -f -s - "$ETERMKIT_FRAMEWORK"

    log_success "ETermKit.framework built at: $ETERMKIT_FRAMEWORK"
}

# ============================================================================
# 编译 ai-cli-session-db FFI
# ============================================================================
build_ffi() {
    log_info "Building ai-cli-session-db FFI..."

    if [ -f "$CLAUDE_SESSION_DB/build.sh" ]; then
        cd "$CLAUDE_SESSION_DB" && ./build.sh
    else
        log_error "ai-cli-session-db/build.sh not found"
        exit 1
    fi

    log_success "FFI built and deployed"
}

# ============================================================================
# 编译 socket-client-ffi
# ============================================================================
build_socket_ffi() {
    log_info "Building socket-client-ffi..."

    # vlaude-core 是独立 workspace，需要单独编译
    cd "$VLAUDE_CORE"
    cargo build --release -p socket-client-ffi

    local DYLIB="$VLAUDE_CORE/target/release/libsocket_client_ffi.dylib"
    local HEADER="$VLAUDE_CORE/socket-client-ffi/socket_client_ffi.h"

    if [ ! -f "$DYLIB" ]; then
        log_error "Socket FFI dylib not found: $DYLIB"
        exit 1
    fi

    # 复制到 VlaudeKit
    log_info "Copying to VlaudeKit..."
    mkdir -p "$VLAUDE_KIT/Libs/SocketClient"
    cp "$DYLIB" "$VLAUDE_KIT/Libs/SocketClient/"
    [ -f "$HEADER" ] && cp "$HEADER" "$VLAUDE_KIT/Libs/SocketClient/"

    # 创建 module.modulemap
    cat > "$VLAUDE_KIT/Libs/SocketClient/module.modulemap" << 'EOF'
module SocketClientFFI {
    header "socket_client_ffi.h"
    link "socket_client_ffi"
    export *
}
EOF

    log_success "Socket FFI built and deployed"
}

# ============================================================================
# 编译 vlaude-ffi (数据查询 API)
# ============================================================================
build_vlaude_ffi() {
    log_info "Building vlaude-ffi..."

    cd "$VLAUDE_CORE"
    cargo build --release -p vlaude-ffi

    local DYLIB="$VLAUDE_CORE/target/release/libvlaude_ffi.dylib"
    local HEADER="$VLAUDE_CORE/vlaude-ffi/vlaude_ffi.h"

    if [ ! -f "$DYLIB" ]; then
        log_error "Vlaude FFI dylib not found: $DYLIB"
        exit 1
    fi

    # 复制到 VlaudeKit
    log_info "Copying to VlaudeKit..."
    mkdir -p "$VLAUDE_KIT/Libs/VlaudeFfi"
    cp "$DYLIB" "$VLAUDE_KIT/Libs/VlaudeFfi/"
    [ -f "$HEADER" ] && cp "$HEADER" "$VLAUDE_KIT/Libs/VlaudeFfi/"

    # 创建 module.modulemap
    cat > "$VLAUDE_KIT/Libs/VlaudeFfi/module.modulemap" << 'EOF'
module VlaudeFFI {
    header "vlaude_ffi.h"
    link "vlaude_ffi"
    export *
}
EOF

    log_success "Vlaude FFI built and deployed"
}

# ============================================================================
# 编译 sugarloaf-ffi
# ============================================================================
build_sugarloaf() {
    log_info "Building sugarloaf-ffi..."

    cd "$RIO_DIR"
    cargo build --release -p sugarloaf-ffi

    local STATIC_LIB="$RIO_DIR/target/release/libsugarloaf_ffi.a"

    if [ ! -f "$STATIC_LIB" ]; then
        log_error "sugarloaf-ffi static lib not found: $STATIC_LIB"
        exit 1
    fi

    # 复制到 ETerm/Libs/Sugarloaf
    log_info "Copying to ETerm/Libs/Sugarloaf..."
    mkdir -p "$ETERM_DIR/ETerm/Libs/Sugarloaf"
    cp "$STATIC_LIB" "$ETERM_DIR/ETerm/Libs/Sugarloaf/"

    # 复制到 dev-runner（如果目录存在）
    if [ -d "$DEV_RUNNER/swift-app/Sources/Terminal/Libs" ]; then
        log_info "Copying to dev-runner..."
        cp "$STATIC_LIB" "$DEV_RUNNER/swift-app/Sources/Terminal/Libs/"
    fi

    log_success "sugarloaf-ffi built and deployed"
}

# ============================================================================
# 编译 memex binary
# ============================================================================
build_memex() {
    log_info "Building memex..."

    if [ -f "$MEMEX_RS/build.sh" ]; then
        cd "$MEMEX_RS" && ./build.sh
    else
        log_error "memex-rs/build.sh not found"
        exit 1
    fi

    log_success "Memex built and deployed"
}

# ============================================================================
# 编译 vimo-agent
# ============================================================================
build_agent() {
    log_info "Building vimo-agent..."

    cd "$CLAUDE_SESSION_DB"
    cargo build --release --bin vimo-agent --features agent

    # 工作空间编译输出到根目录的 target（不是子包的 target）
    local BINARY="$ETERM_ROOT/target/release/vimo-agent"

    if [ ! -f "$BINARY" ]; then
        log_error "vimo-agent binary not found: $BINARY"
        exit 1
    fi

    # 部署到 ~/.vimo/bin/
    local DEPLOY_DIR="$HOME/.vimo/bin"
    mkdir -p "$DEPLOY_DIR"
    cp "$BINARY" "$DEPLOY_DIR/"
    chmod +x "$DEPLOY_DIR/vimo-agent"

    log_success "vimo-agent built and deployed to $DEPLOY_DIR"
}

# ============================================================================
# 编译 pty-daemon
# ============================================================================
build_pty_daemon() {
    log_info "Building pty-daemon..."

    cd "$ETERM_ROOT"
    cargo build --release -p pty-daemon

    local BINARY="$ETERM_ROOT/target/release/pty-daemon"

    if [ ! -f "$BINARY" ]; then
        log_error "pty-daemon binary not found: $BINARY"
        exit 1
    fi

    local DEPLOY_DIR="$HOME/.vimo/bin"
    mkdir -p "$DEPLOY_DIR"
    cp "$BINARY" "$DEPLOY_DIR/"
    chmod +x "$DEPLOY_DIR/pty-daemon"

    log_success "pty-daemon built and deployed to $DEPLOY_DIR"
}

# ============================================================================
# 编译 mcp-router-core
# ============================================================================
build_mcp_router() {
    log_info "Building mcp-router-core..."

    cd "$MCP_ROUTER"
    cargo build --release

    local DYLIB="$MCP_ROUTER/target/release/libmcp_router_core.dylib"
    local HEADER="$MCP_ROUTER/include/mcp_router_core.h"

    if [ ! -f "$DYLIB" ]; then
        log_error "mcp-router-core dylib not found: $DYLIB"
        exit 1
    fi

    # 复制到 MCPRouterKit
    log_info "Copying to MCPRouterKit..."
    mkdir -p "$MCP_ROUTER_KIT/Lib"
    cp "$DYLIB" "$MCP_ROUTER_KIT/Lib/"
    [ -f "$HEADER" ] && cp "$HEADER" "$MCP_ROUTER_KIT/Lib/"

    log_success "mcp-router-core built and deployed"
}

# ============================================================================
# 构建 Swift 插件
# ============================================================================
build_plugins() {
    log_info "Building Swift plugins..."

    if [ -f "$VLAUDE_KIT/build.sh" ]; then
        log_info "Building VlaudeKit..."
        cd "$VLAUDE_KIT" && ./build.sh
    fi

    if [ -f "$MEMEX_KIT/build.sh" ]; then
        log_info "Building MemexKit..."
        cd "$MEMEX_KIT" && ./build.sh
    fi

    if [ -f "$LSP_KIT/build.sh" ]; then
        log_info "Building LspKit..."
        cd "$LSP_KIT" && ./build.sh
    fi

    log_success "Plugins built"
}

# ============================================================================
# Clippy 检查
# ============================================================================
lint_rust() {
    log_info "Running clippy on all Rust projects..."

    # 主 Workspace（ai-cli-session-collector, ai-cli-session-db, memex-rs 等）
    log_info "Checking main workspace..."
    cd "$ETERM_ROOT"
    cargo clippy --workspace -- -D warnings

    # 独立 workspace: vlaude-core
    log_info "Checking vlaude-core..."
    cd "$VLAUDE_CORE"
    cargo clippy --workspace -- -D warnings

    # 独立 workspace: mcp-router
    log_info "Checking mcp-router..."
    cd "$MCP_ROUTER"
    cargo clippy -- -D warnings

    # 独立 workspace: rio (sugarloaf)
    log_info "Checking rio/sugarloaf..."
    cd "$RIO_DIR"
    cargo clippy --workspace -- -D warnings

    log_success "Clippy passed for all projects"
}

# ============================================================================
# Vlaude 事件一致性检查
# ============================================================================
check_vlaude_events() {
    log_info "Checking Vlaude event consistency..."

    local CHECK_SCRIPT="$SCRIPT_DIR/check-vlaude-events.sh"
    if [ -f "$CHECK_SCRIPT" ]; then
        if ! "$CHECK_SCRIPT"; then
            log_error "Event consistency check failed! Fix event definitions before building."
            exit 1
        fi
    else
        log_warn "Event check script not found: $CHECK_SCRIPT"
    fi
}

# ============================================================================
# 主逻辑
# ============================================================================
main() {
    local TARGET="${1:-all}"

    log_info "ETerm Build System"
    log_info "Root: $ETERM_ROOT"
    echo ""

    # 构建前检查事件一致性（仅当涉及 VlaudeKit 或全量构建时）
    case "$TARGET" in
        socket|vlaude-ffi|plugins|all)
            check_vlaude_events
            echo ""
            ;;
    esac

    case "$TARGET" in
        etermkit)
            build_etermkit
            ;;
        ffi)
            build_ffi
            ;;
        socket)
            build_socket_ffi
            ;;
        vlaude-ffi)
            build_vlaude_ffi
            ;;
        sugarloaf)
            build_sugarloaf
            ;;
        memex)
            build_memex
            ;;
        mcp-router)
            build_mcp_router
            ;;
        agent)
            build_agent
            ;;
        pty-daemon)
            build_pty_daemon
            ;;
        plugins)
            build_etermkit  # 插件依赖 ETermKit，先确保它已构建
            build_plugins
            ;;
        lint)
            lint_rust
            ;;
        check)
            check_vlaude_events
            ;;
        all)
            build_etermkit  # 首先编译 SDK
            build_ffi
            build_socket_ffi
            build_vlaude_ffi
            build_sugarloaf
            build_memex
            build_mcp_router
            build_agent
            build_pty_daemon
            build_plugins
            ;;
        *)
            log_error "Unknown target: $TARGET"
            echo "Usage: $0 [etermkit|ffi|socket|vlaude-ffi|sugarloaf|memex|mcp-router|agent|pty-daemon|plugins|lint|check|all]"
            exit 1
            ;;
    esac

    echo ""
    log_success "Build completed!"
}

main "$@"
