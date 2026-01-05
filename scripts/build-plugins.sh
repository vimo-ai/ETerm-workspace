#!/usr/bin/env bash
# ============================================================================
# ETerm 纯 Swift 插件编译脚本
#
# 编译所有纯 Swift 插件并输出到指定目录
#
# 使用方式:
#   ./scripts/build-plugins.sh                    # Debug 编译到用户目录
#   ./scripts/build-plugins.sh release            # Release 编译到用户目录
#   ./scripts/build-plugins.sh release /path/to  # Release 编译到指定目录
# ============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETERM_ROOT="$(dirname "$SCRIPT_DIR")"
PLUGINS_DIR="$ETERM_ROOT/ETerm/Plugins"

# 配置
CONFIGURATION="${1:-Debug}"
OUTPUT_DIR="${2:-$HOME/.vimo/eterm/plugins}"

# 纯 Swift 插件列表（无 native 依赖）
PURE_SWIFT_PLUGINS=(
    "ClaudeKit"
    "ClaudeMonitorKit"
    "DevHelperKit"
    "HistoryKit"
    "MCPRouterKit"
    "OneLineCommandKit"
    "TranslationKit"
    "WorkspaceKit"
    "WritingKit"
)

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[Plugins]${NC} $*"; }
log_success() { echo -e "${GREEN}[Plugins]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[Plugins]${NC} $*"; }
log_error() { echo -e "${RED}[Plugins]${NC} $*"; }

# ============================================================================
# 编译单个插件
# ============================================================================
build_plugin() {
    local plugin_name="$1"
    local plugin_dir="$PLUGINS_DIR/$plugin_name"

    if [ ! -d "$plugin_dir" ]; then
        log_warn "Plugin not found: $plugin_name"
        return 1
    fi

    log_info "Building $plugin_name..."

    cd "$plugin_dir"

    # 使用 swift build
    local config_lower=$(echo "$CONFIGURATION" | tr '[:upper:]' '[:lower:]')
    swift build -c "$config_lower"

    # 创建 bundle 结构
    local bundle_dir="$OUTPUT_DIR/$plugin_name.bundle"
    rm -rf "$bundle_dir"
    mkdir -p "$bundle_dir/Contents/MacOS"
    mkdir -p "$bundle_dir/Contents/Resources"

    # 复制动态库
    local build_dir=".build/$config_lower"
    cp "$build_dir/lib${plugin_name}.dylib" "$bundle_dir/Contents/MacOS/$plugin_name"

    # 修复 ETermKit 依赖路径
    install_name_tool -change @rpath/libETermKit.dylib \
        @executable_path/../Frameworks/ETermKit.framework/ETermKit \
        "$bundle_dir/Contents/MacOS/$plugin_name"

    # 重签名
    codesign -f -s - "$bundle_dir/Contents/MacOS/$plugin_name"

    # 复制 manifest（如果存在）
    if [ -f "Resources/manifest.json" ]; then
        cp "Resources/manifest.json" "$bundle_dir/Contents/Resources/"
    fi

    # 创建 Info.plist
    local bundle_id=$(grep -o '"id"[[:space:]]*:[[:space:]]*"[^"]*"' "Resources/manifest.json" 2>/dev/null | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || echo "com.eterm.$plugin_name")
    local version=$(grep -o '"version"[[:space:]]*:[[:space:]]*"[^"]*"' "Resources/manifest.json" 2>/dev/null | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || echo "1.0.0")

    cat > "$bundle_dir/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$bundle_id</string>
    <key>CFBundleName</key>
    <string>$plugin_name</string>
    <key>CFBundleExecutable</key>
    <string>$plugin_name</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
</dict>
</plist>
EOF

    log_success "$plugin_name built"
}

# ============================================================================
# 主逻辑
# ============================================================================
main() {
    log_info "Building pure Swift plugins..."
    log_info "Configuration: $CONFIGURATION"
    log_info "Output: $OUTPUT_DIR"
    echo ""

    # 确保输出目录存在
    mkdir -p "$OUTPUT_DIR"

    # 编译所有插件
    local success_count=0
    local fail_count=0

    for plugin in "${PURE_SWIFT_PLUGINS[@]}"; do
        if build_plugin "$plugin"; then
            ((success_count++))
        else
            ((fail_count++))
        fi
    done

    echo ""
    log_success "Build completed: $success_count succeeded, $fail_count failed"
}

main "$@"
