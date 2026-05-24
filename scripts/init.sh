#!/usr/bin/env bash
# ============================================================================
# ETerm 开发环境初始化
#
# 新机器跑一次，检查并安装编译所需的系统依赖。
#
# 使用方式:
#   ./scripts/init.sh          # 检查 + 自动安装（不需要 sudo 的部分）
#   ./scripts/init.sh --check  # 只检查，不安装
# ============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETERM_ROOT="$(dirname "$SCRIPT_DIR")"

CHECK_ONLY=false
for arg in "$@"; do
    case $arg in
        --check) CHECK_ONLY=true ;;
    esac
done

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_info()    { echo -e "${BLUE}[init]${NC} $*"; }
log_success() { echo -e "${GREEN}[init]${NC} ✅ $*"; }
log_warn()    { echo -e "${YELLOW}[init]${NC} ⚠️  $*"; }
log_error()   { echo -e "${RED}[init]${NC} ❌ $*"; }

MISSING=()
NEED_SUDO=()

# ============================================================================
# 检查函数
# ============================================================================

check_xcode_cli() {
    log_info "Checking Xcode Command Line Tools..."
    if xcode-select -p &>/dev/null; then
        log_success "Xcode CLI Tools"
    else
        NEED_SUDO+=("xcode-select --install")
        log_error "Xcode CLI Tools not installed"
    fi
}

check_xcode_first_launch() {
    log_info "Checking Xcode first launch setup..."
    # CoreSimulator 存在说明 runFirstLaunch 跑过
    if [ -d "/Library/Developer/PrivateFrameworks/CoreSimulator.framework" ]; then
        log_success "Xcode first launch completed"
    else
        NEED_SUDO+=("sudo xcodebuild -runFirstLaunch")
        log_warn "Xcode first launch not done"
    fi
}

check_metal_toolchain() {
    log_info "Checking Metal Toolchain..."
    if xcrun --find metal &>/dev/null; then
        log_success "Metal Toolchain"
    else
        log_warn "Metal Toolchain not installed"
        if [ "$CHECK_ONLY" = false ]; then
            log_info "Installing Metal Toolchain..."
            xcodebuild -downloadComponent MetalToolchain 2>&1
            if xcrun --find metal &>/dev/null; then
                log_success "Metal Toolchain installed"
            else
                log_error "Metal Toolchain install failed"
                MISSING+=("Metal Toolchain")
            fi
        else
            MISSING+=("Metal Toolchain: xcodebuild -downloadComponent MetalToolchain")
        fi
    fi
}

check_brew() {
    log_info "Checking Homebrew..."
    if command -v brew &>/dev/null; then
        log_success "Homebrew $(brew --version | head -1)"
    elif [ -x /opt/homebrew/bin/brew ]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
        log_success "Homebrew (found at /opt/homebrew, added to PATH)"
    else
        NEED_SUDO+=('/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"')
        log_error "Homebrew not installed"
    fi
}

check_rust() {
    log_info "Checking Rust toolchain..."
    if command -v rustc &>/dev/null && command -v cargo &>/dev/null; then
        log_success "Rust $(rustc --version | awk '{print $2}')"
    else
        log_warn "Rust not installed"
        if [ "$CHECK_ONLY" = false ]; then
            log_info "Installing Rust via rustup..."
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            source "$HOME/.cargo/env"
            if command -v rustc &>/dev/null; then
                log_success "Rust $(rustc --version | awk '{print $2}') installed"
            else
                log_error "Rust install failed"
                MISSING+=("Rust")
            fi
        else
            MISSING+=("Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh")
        fi
    fi
}

check_protoc() {
    log_info "Checking protoc..."
    if command -v protoc &>/dev/null; then
        log_success "protoc $(protoc --version | awk '{print $2}')"
    else
        if command -v brew &>/dev/null; then
            if [ "$CHECK_ONLY" = false ]; then
                log_info "Installing protobuf via brew..."
                brew install protobuf
                log_success "protoc installed"
            else
                MISSING+=("protoc: brew install protobuf")
            fi
        else
            MISSING+=("protoc (install brew first, then: brew install protobuf)")
        fi
    fi
}

check_node() {
    log_info "Checking Node.js..."

    # 先尝试激活 fnm
    if command -v fnm &>/dev/null; then
        eval "$(fnm env)" 2>/dev/null || true
    elif [ -d "$HOME/.local/share/fnm" ]; then
        export PATH="$HOME/.local/share/fnm:$PATH"
        eval "$(fnm env)" 2>/dev/null || true
    fi

    if command -v node &>/dev/null; then
        log_success "Node.js $(node --version)"
    else
        if command -v brew &>/dev/null; then
            if [ "$CHECK_ONLY" = false ]; then
                log_info "Installing fnm + Node.js..."
                brew install fnm 2>/dev/null || true
                eval "$(fnm env)"
                fnm install --lts
                log_success "Node.js $(node --version) installed via fnm"
            else
                MISSING+=("Node.js: brew install fnm && fnm install --lts")
            fi
        else
            MISSING+=("Node.js (install brew first)")
        fi
    fi
}

check_pnpm() {
    log_info "Checking pnpm..."
    if command -v pnpm &>/dev/null; then
        log_success "pnpm $(pnpm --version)"
    else
        if command -v brew &>/dev/null; then
            if [ "$CHECK_ONLY" = false ]; then
                log_info "Installing pnpm via brew..."
                brew install pnpm
                log_success "pnpm installed"
            else
                MISSING+=("pnpm: brew install pnpm")
            fi
        else
            MISSING+=("pnpm (install brew first)")
        fi
    fi
}

# ============================================================================
# 主逻辑
# ============================================================================
main() {
    echo ""
    log_info "ETerm Development Environment Init"
    log_info "Root: $ETERM_ROOT"
    if [ "$CHECK_ONLY" = true ]; then
        log_info "Mode: check only (--check)"
    fi
    echo ""

    check_xcode_cli
    check_xcode_first_launch
    check_metal_toolchain
    check_brew
    check_rust
    check_protoc
    check_node
    check_pnpm

    echo ""

    # 报告结果
    local HAS_ISSUES=false

    if [ ${#NEED_SUDO[@]} -gt 0 ]; then
        HAS_ISSUES=true
        log_warn "The following require manual setup (needs sudo/interactive):"
        echo ""
        for cmd in "${NEED_SUDO[@]}"; do
            echo "    $cmd"
        done
        echo ""
    fi

    if [ ${#MISSING[@]} -gt 0 ]; then
        HAS_ISSUES=true
        log_warn "Missing dependencies:"
        echo ""
        for item in "${MISSING[@]}"; do
            echo "    $item"
        done
        echo ""
    fi

    if [ "$HAS_ISSUES" = false ]; then
        log_success "All dependencies satisfied!"
        echo ""
        log_info "Next: ./scripts/build.sh all"
    else
        echo ""
        log_info "Fix the above issues, then re-run: ./scripts/init.sh"
    fi
}

main "$@"
