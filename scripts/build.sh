#!/usr/bin/env bash
# ============================================================================
# ETerm 统一编译脚本
#
# 编译所有 Rust 组件并部署到 Swift 插件目录
#
# 使用方式:
#   ./scripts/build.sh           # 编译所有
#   ./scripts/build.sh ffi       # 只编译 claude-session-db FFI
#   ./scripts/build.sh socket    # 只编译 socket-client-ffi
#   ./scripts/build.sh memex     # 只编译 memex
#   ./scripts/build.sh sugarloaf # 只编译 sugarloaf-ffi
#   ./scripts/build.sh mcp-router # 只编译 mcp-router-core
#   ./scripts/build.sh plugins   # 只构建 Swift 插件
#   ./scripts/build.sh check     # 只运行事件一致性检查
# ============================================================================
set -e

# 从脚本位置推断项目根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETERM_ROOT="$(dirname "$SCRIPT_DIR")"

# 目录定义
CLAUDE_SESSION_DB="$ETERM_ROOT/claude-session-db"
MEMEX_RS="$ETERM_ROOT/memex/memex-rs"
VLAUDE_CORE="$ETERM_ROOT/vlaude/packages/vlaude-core"
ETERM_DIR="$ETERM_ROOT/ETerm"
RIO_DIR="$ETERM_DIR/rio"
VLAUDE_KIT="$ETERM_DIR/Plugins/VlaudeKit"
MEMEX_KIT="$ETERM_DIR/Plugins/MemexKit"
LSP_KIT="$ETERM_DIR/Plugins/LspKit"
MCP_ROUTER="$ETERM_ROOT/mcp-router/core"
MCP_ROUTER_KIT="$ETERM_DIR/Plugins/MCPRouterKit"

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
# 编译 claude-session-db FFI
# ============================================================================
build_ffi() {
    log_info "Building claude-session-db FFI..."

    # 在 workspace 根目录编译，输出到 workspace target
    cd "$ETERM_ROOT"
    cargo build --release -p claude-session-db --features ffi,fts,coordination

    # 使用 workspace target 路径
    local DYLIB="$ETERM_ROOT/target/release/libclaude_session_db.dylib"
    local HEADER="$CLAUDE_SESSION_DB/include/claude_session_db.h"

    if [ ! -f "$DYLIB" ]; then
        log_error "FFI dylib not found: $DYLIB"
        exit 1
    fi

    # 复制到 VlaudeKit
    log_info "Copying to VlaudeKit..."
    mkdir -p "$VLAUDE_KIT/Libs/SharedDB"
    cp "$DYLIB" "$VLAUDE_KIT/Libs/SharedDB/"
    [ -f "$HEADER" ] && cp "$HEADER" "$VLAUDE_KIT/Libs/SharedDB/"

    # 复制到 MemexKit
    log_info "Copying to MemexKit..."
    mkdir -p "$MEMEX_KIT/Libs/SharedDB"
    cp "$DYLIB" "$MEMEX_KIT/Libs/SharedDB/"
    [ -f "$HEADER" ] && cp "$HEADER" "$MEMEX_KIT/Libs/SharedDB/"

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

    log_success "sugarloaf-ffi built and deployed"
}

# ============================================================================
# 编译 memex binary
# ============================================================================
build_memex() {
    log_info "Building memex..."

    # 在 workspace 根目录编译，输出到 workspace target
    cd "$ETERM_ROOT"
    cargo build --release -p memex-rs --features cli

    # 使用 workspace target 路径
    local BINARY="$ETERM_ROOT/target/release/memex"

    if [ ! -f "$BINARY" ]; then
        log_error "Memex binary not found: $BINARY"
        exit 1
    fi

    # 复制到 MemexKit（开发模式）
    log_info "Copying to MemexKit..."
    mkdir -p "$MEMEX_KIT/Lib"
    cp "$BINARY" "$MEMEX_KIT/Lib/"
    chmod +x "$MEMEX_KIT/Lib/memex"

    # 复制到 ~/.vimo/eterm/bin/（用户安装路径，确保新贡献者能找到）
    local ETERM_BIN="$HOME/.vimo/eterm/bin"
    log_info "Installing to $ETERM_BIN..."
    mkdir -p "$ETERM_BIN"
    cp "$BINARY" "$ETERM_BIN/"
    chmod +x "$ETERM_BIN/memex"

    log_success "Memex built and deployed"
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
        socket|plugins|all)
            check_vlaude_events
            echo ""
            ;;
    esac

    case "$TARGET" in
        ffi)
            build_ffi
            ;;
        socket)
            build_socket_ffi
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
        plugins)
            build_plugins
            ;;
        check)
            check_vlaude_events
            ;;
        all)
            build_ffi
            build_socket_ffi
            build_sugarloaf
            build_memex
            build_mcp_router
            build_plugins
            ;;
        *)
            log_error "Unknown target: $TARGET"
            echo "Usage: $0 [ffi|socket|sugarloaf|memex|mcp-router|plugins|check|all]"
            exit 1
            ;;
    esac

    echo ""
    log_success "Build completed!"
}

main "$@"
