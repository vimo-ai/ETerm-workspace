#!/bin/bash
#
# Vlaude 事件一致性检查脚本
#
# 检查三端事件名称是否一致：
# - Server (TypeScript): vlaude/packages/vlaude-server/src/shared/events/constants.ts
# - Swift (VlaudeKit): ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/Events/EventConstants.swift
# - Rust (socket-client): vlaude/packages/vlaude-core/socket-client/src/events.rs
#
# 用法: ./scripts/check-vlaude-events.sh [--verbose]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# 文件路径
SERVER_FILE="$ROOT_DIR/vlaude/packages/vlaude-server/src/shared/events/constants.ts"
SWIFT_FILE="$ROOT_DIR/ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/Events/EventConstants.swift"
RUST_FILE="$ROOT_DIR/vlaude/packages/vlaude-core/socket-client/src/events.rs"

# 颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

VERBOSE=false
if [[ "${1:-}" == "--verbose" ]]; then
    VERBOSE=true
fi

log_info() { echo -e "${BLUE}[CHECK]${NC} $*"; }
log_success() { echo -e "${GREEN}[CHECK]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[CHECK]${NC} $*"; }
log_error() { echo -e "${RED}[CHECK]${NC} $*"; }

# 检查文件是否存在
check_files_exist() {
    local missing=false

    if [[ ! -f "$SERVER_FILE" ]]; then
        log_error "Server 事件文件不存在: $SERVER_FILE"
        missing=true
    fi

    if [[ ! -f "$SWIFT_FILE" ]]; then
        log_error "Swift 事件文件不存在: $SWIFT_FILE"
        missing=true
    fi

    if [[ ! -f "$RUST_FILE" ]]; then
        log_error "Rust 事件文件不存在: $RUST_FILE"
        missing=true
    fi

    if [[ "$missing" == "true" ]]; then
        log_error "请先创建事件常量文件"
        exit 1
    fi
}

# 从文件中提取事件名（在 @event-registry-start 和 @event-registry-end 之间）
extract_events() {
    local file="$1"
    local events=""

    # 提取标记区域内的内容，然后用正则匹配事件名
    # 事件名格式: "daemon:xxx" 或 'daemon:xxx' 或 "server:xxx" 或 "server-shutdown"
    # 支持单引号和双引号
    events=$(sed -n '/@event-registry-start/,/@event-registry-end/p' "$file" \
        | grep -oE "['\"]((daemon|server):[a-zA-Z]+|server-shutdown)['\"]" \
        | tr -d "\"'" \
        | sort -u)

    echo "$events"
}

# 比较两个事件列表
compare_events() {
    local name1="$1"
    local events1="$2"
    local name2="$3"
    local events2="$4"

    local only_in_1=""
    local only_in_2=""

    # 找出只在 1 中有的
    while IFS= read -r event; do
        [[ -z "$event" ]] && continue
        if ! echo "$events2" | grep -qx "$event"; then
            only_in_1="$only_in_1$event"$'\n'
        fi
    done <<< "$events1"

    # 找出只在 2 中有的
    while IFS= read -r event; do
        [[ -z "$event" ]] && continue
        if ! echo "$events1" | grep -qx "$event"; then
            only_in_2="$only_in_2$event"$'\n'
        fi
    done <<< "$events2"

    local has_diff=false

    if [[ -n "$only_in_1" ]]; then
        log_warn "只在 $name1 中有:"
        echo "$only_in_1" | while IFS= read -r e; do
            [[ -n "$e" ]] && echo "    - $e"
        done
        has_diff=true
    fi

    if [[ -n "$only_in_2" ]]; then
        log_warn "只在 $name2 中有:"
        echo "$only_in_2" | while IFS= read -r e; do
            [[ -n "$e" ]] && echo "    - $e"
        done
        has_diff=true
    fi

    if [[ "$has_diff" == "true" ]]; then
        return 1
    fi
    return 0
}

# 扫描代码中的硬编码事件字符串
# 排除常量定义文件，扫描实际使用的代码
scan_hardcoded_events() {
    local has_hardcoded=false

    # 要扫描的文件目录
    local TS_DIR="$ROOT_DIR/vlaude/packages/vlaude-server/src"
    local SWIFT_DIR="$ROOT_DIR/ETerm/Plugins/VlaudeKit/Sources"
    local RUST_DIR="$ROOT_DIR/vlaude/packages/vlaude-core/socket-client/src"

    # 常量定义文件（排除）
    local CONST_FILES=(
        "$SERVER_FILE"
        "$SWIFT_FILE"
        "$RUST_FILE"
    )

    log_info "扫描硬编码事件字符串..."

    # 扫描 TypeScript 文件
    if [[ -d "$TS_DIR" ]]; then
        local ts_hardcoded=""
        # 查找 .ts 文件中的硬编码事件，排除常量文件、测试文件和 @event-registry 区域
        ts_hardcoded=$(find "$TS_DIR" -name "*.ts" -type f \
            ! -path "$SERVER_FILE" \
            ! -path "*/shared/events/*" \
            ! -name "*.spec.ts" \
            ! -name "*.test.ts" \
            ! -path "*/test/*" \
            -exec grep -l "['\"]\\(daemon:\\|server:\\)" {} \; 2>/dev/null || true)

        if [[ -n "$ts_hardcoded" ]]; then
            # 进一步过滤，排除注释和导入语句
            for file in $ts_hardcoded; do
                local matches=$(grep -n "['\"]\\(daemon:\\|server:\\)" "$file" \
                    | grep -v "^[[:space:]]*//" \
                    | grep -v "^[[:space:]]*\*" \
                    | grep -v "import " \
                    | grep -v "@event-registry" || true)
                if [[ -n "$matches" ]]; then
                    log_warn "TypeScript 硬编码发现: $file"
                    echo "$matches" | head -5 | sed 's/^/    /'
                    has_hardcoded=true
                fi
            done
        fi
    fi

    # 扫描 Swift 文件
    if [[ -d "$SWIFT_DIR" ]]; then
        local swift_hardcoded=""
        swift_hardcoded=$(find "$SWIFT_DIR" -name "*.swift" -type f \
            ! -path "$SWIFT_FILE" \
            ! -path "*/Events/*" \
            -exec grep -l "\"\\(daemon:\\|server:\\)" {} \; 2>/dev/null || true)

        if [[ -n "$swift_hardcoded" ]]; then
            for file in $swift_hardcoded; do
                local matches=$(grep -n "\"\\(daemon:\\|server:\\)" "$file" \
                    | grep -v "^[[:space:]]*//" \
                    | grep -v "@event-registry" || true)
                if [[ -n "$matches" ]]; then
                    log_warn "Swift 硬编码发现: $file"
                    echo "$matches" | head -5 | sed 's/^/    /'
                    has_hardcoded=true
                fi
            done
        fi
    fi

    # 扫描 Rust 文件
    if [[ -d "$RUST_DIR" ]]; then
        local rust_hardcoded=""
        rust_hardcoded=$(find "$RUST_DIR" -name "*.rs" -type f \
            ! -path "$RUST_FILE" \
            -exec grep -l "\"\\(daemon:\\|server:\\)" {} \; 2>/dev/null || true)

        if [[ -n "$rust_hardcoded" ]]; then
            for file in $rust_hardcoded; do
                local matches=$(grep -n "\"\\(daemon:\\|server:\\)" "$file" \
                    | grep -v "^[[:space:]]*//" \
                    | grep -v "@event-registry" || true)
                if [[ -n "$matches" ]]; then
                    log_warn "Rust 硬编码发现: $file"
                    echo "$matches" | head -5 | sed 's/^/    /'
                    has_hardcoded=true
                fi
            done
        fi
    fi

    if [[ "$has_hardcoded" == "true" ]]; then
        return 1
    fi
    return 0
}

main() {
    log_info "检查 Vlaude 事件一致性..."
    echo ""

    # 检查文件存在
    check_files_exist

    # 提取事件
    log_info "提取 Server 端事件..."
    SERVER_EVENTS=$(extract_events "$SERVER_FILE")
    SERVER_COUNT=$(echo "$SERVER_EVENTS" | grep -c . || echo "0")

    log_info "提取 Swift 端事件..."
    SWIFT_EVENTS=$(extract_events "$SWIFT_FILE")
    SWIFT_COUNT=$(echo "$SWIFT_EVENTS" | grep -c . || echo "0")

    log_info "提取 Rust 端事件..."
    RUST_EVENTS=$(extract_events "$RUST_FILE")
    RUST_COUNT=$(echo "$RUST_EVENTS" | grep -c . || echo "0")

    echo ""
    log_info "事件数量统计:"
    echo "    Server (TypeScript): $SERVER_COUNT"
    echo "    Swift (VlaudeKit):   $SWIFT_COUNT"
    echo "    Rust (socket-client): $RUST_COUNT"
    echo ""

    if [[ "$VERBOSE" == "true" ]]; then
        log_info "Server 事件列表:"
        echo "$SERVER_EVENTS" | sed 's/^/    /'
        echo ""
        log_info "Swift 事件列表:"
        echo "$SWIFT_EVENTS" | sed 's/^/    /'
        echo ""
        log_info "Rust 事件列表:"
        echo "$RUST_EVENTS" | sed 's/^/    /'
        echo ""
    fi

    # 比较
    local has_error=false

    log_info "比较 Server ↔ Swift..."
    if ! compare_events "Server" "$SERVER_EVENTS" "Swift" "$SWIFT_EVENTS"; then
        has_error=true
    fi

    log_info "比较 Server ↔ Rust..."
    if ! compare_events "Server" "$SERVER_EVENTS" "Rust" "$RUST_EVENTS"; then
        has_error=true
    fi

    log_info "比较 Swift ↔ Rust..."
    if ! compare_events "Swift" "$SWIFT_EVENTS" "Rust" "$RUST_EVENTS"; then
        has_error=true
    fi

    echo ""

    # 扫描硬编码事件
    local has_hardcoded=false
    if ! scan_hardcoded_events; then
        has_hardcoded=true
        has_error=true
    fi

    echo ""

    if [[ "$has_error" == "true" ]]; then
        if [[ "$has_hardcoded" == "true" ]]; then
            log_error "❌ 发现硬编码事件字符串！请使用常量替代。"
            echo ""
            echo "常量文件位置:"
            echo "  Server: $SERVER_FILE"
            echo "  Swift:  $SWIFT_FILE"
            echo "  Rust:   $RUST_FILE"
        else
            log_error "❌ 事件一致性检查失败！请同步三端事件定义。"
            echo ""
            echo "文件位置:"
            echo "  Server: $SERVER_FILE"
            echo "  Swift:  $SWIFT_FILE"
            echo "  Rust:   $RUST_FILE"
        fi
        exit 1
    else
        log_success "✅ 事件一致性检查通过！三端事件定义一致。"
        log_success "✅ 无硬编码事件字符串。"
    fi
}

main "$@"
