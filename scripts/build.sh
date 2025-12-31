#!/usr/bin/env bash
# ============================================================================
# ETerm 统一编译脚本
#
# 编译所有 Rust 组件并部署到 Swift 插件目录
#
# 使用方式:
#   ./scripts/build.sh           # 编译所有
#   ./scripts/build.sh ffi       # 只编译 claude-session-db FFI
#   ./scripts/build.sh memex     # 只编译 memex
#   ./scripts/build.sh plugins   # 只构建 Swift 插件
# ============================================================================
set -e

# 从脚本位置推断项目根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETERM_ROOT="$(dirname "$SCRIPT_DIR")"

# 目录定义
CLAUDE_SESSION_DB="$ETERM_ROOT/claude-session-db"
MEMEX_RS="$ETERM_ROOT/memex/memex-rs"
ENGLISH_DIR="$ETERM_ROOT/english"
VLAUDE_KIT="$ENGLISH_DIR/Plugins/VlaudeKit"
MEMEX_KIT="$ENGLISH_DIR/Plugins/MemexKit"

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

    cd "$CLAUDE_SESSION_DB"
    cargo build --release --features ffi,fts,coordination

    local DYLIB="target/release/libclaude_session_db.dylib"
    local HEADER="include/claude_session_db.h"

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
# 编译 memex binary
# ============================================================================
build_memex() {
    log_info "Building memex..."

    cd "$MEMEX_RS"
    cargo build --release --features cli

    local BINARY="target/release/memex"

    if [ ! -f "$BINARY" ]; then
        log_error "Memex binary not found: $BINARY"
        exit 1
    fi

    # 复制到 MemexKit
    log_info "Copying to MemexKit..."
    mkdir -p "$MEMEX_KIT/Lib"
    cp "$BINARY" "$MEMEX_KIT/Lib/"
    chmod +x "$MEMEX_KIT/Lib/memex"

    log_success "Memex built and deployed"
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

    log_success "Plugins built"
}

# ============================================================================
# 主逻辑
# ============================================================================
main() {
    local TARGET="${1:-all}"

    log_info "ETerm Build System"
    log_info "Root: $ETERM_ROOT"
    echo ""

    case "$TARGET" in
        ffi)
            build_ffi
            ;;
        memex)
            build_memex
            ;;
        plugins)
            build_plugins
            ;;
        all)
            build_ffi
            build_memex
            build_plugins
            ;;
        *)
            log_error "Unknown target: $TARGET"
            echo "Usage: $0 [ffi|memex|plugins|all]"
            exit 1
            ;;
    esac

    echo ""
    log_success "Build completed!"
}

main "$@"
