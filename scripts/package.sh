#!/usr/bin/env bash
# ============================================================================
# ETerm 打包脚本
#
# 编译、签名、打包 DMG
#
# 使用方式:
#   ./scripts/package.sh                    # 构建 Release DMG（无签名）
#   ./scripts/package.sh --sign             # 构建并签名（需要 Developer ID）
#   ./scripts/package.sh --notarize         # 构建、签名并公证
# ============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ETERM_ROOT="$(dirname "$SCRIPT_DIR")"
ETERM_XCODE_PROJECT="$ETERM_ROOT/ETerm/ETerm"

# 配置
APP_NAME="ETerm"
BUNDLE_ID="com.vimo.eterm"
VERSION="${VERSION:-$(date +%Y.%m.%d)}"
BUILD_DIR="$ETERM_ROOT/build/package"
OUTPUT_DIR="$ETERM_ROOT/dist"

# 签名配置（从环境变量或默认值）
DEVELOPER_ID="${DEVELOPER_ID:-}"
APPLE_ID="${APPLE_ID:-}"
TEAM_ID="${TEAM_ID:-}"
APP_SPECIFIC_PASSWORD="${APP_SPECIFIC_PASSWORD:-}"

# 解析参数
DO_SIGN=false
DO_NOTARIZE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --sign)
            DO_SIGN=true
            shift
            ;;
        --notarize)
            DO_SIGN=true
            DO_NOTARIZE=true
            shift
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[Package]${NC} $*"; }
log_success() { echo -e "${GREEN}[Package]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[Package]${NC} $*"; }
log_error() { echo -e "${RED}[Package]${NC} $*"; }

# ============================================================================
# 检查依赖
# ============================================================================
check_requirements() {
    log_info "Checking requirements..."

    if ! command -v xcodebuild &> /dev/null; then
        log_error "xcodebuild not found. Please install Xcode."
        exit 1
    fi

    if $DO_SIGN && [ -z "$DEVELOPER_ID" ]; then
        log_error "DEVELOPER_ID environment variable required for signing."
        log_info "Example: DEVELOPER_ID='Developer ID Application: Your Name (TEAMID)' ./scripts/package.sh --sign"
        exit 1
    fi

    if $DO_NOTARIZE; then
        if [ -z "$APPLE_ID" ] || [ -z "$TEAM_ID" ] || [ -z "$APP_SPECIFIC_PASSWORD" ]; then
            log_error "Notarization requires APPLE_ID, TEAM_ID, and APP_SPECIFIC_PASSWORD environment variables."
            exit 1
        fi
    fi

    log_success "Requirements check passed"
}

# ============================================================================
# 编译 Release 版本
# ============================================================================
build_app() {
    log_info "Building $APP_NAME Release..."

    # 清理之前的构建
    rm -rf "$BUILD_DIR"
    mkdir -p "$BUILD_DIR"

    cd "$ETERM_XCODE_PROJECT"

    # 使用 xcodebuild 构建 Release
    xcodebuild \
        -project ETerm.xcodeproj \
        -scheme ETerm \
        -configuration Release \
        -derivedDataPath "$BUILD_DIR/DerivedData" \
        -destination "platform=macOS" \
        ONLY_ACTIVE_ARCH=NO \
        clean build

    # 复制 app 到 package 目录
    cp -R "$BUILD_DIR/DerivedData/Build/Products/Release/$APP_NAME.app" "$BUILD_DIR/"

    log_success "App built: $BUILD_DIR/$APP_NAME.app"
}

# ============================================================================
# 编译并嵌入纯 Swift 插件
# ============================================================================
build_and_embed_plugins() {
    log_info "Building and embedding plugins..."

    local plugins_output="$BUILD_DIR/$APP_NAME.app/Contents/PlugIns"
    mkdir -p "$plugins_output"

    # 使用 build-plugins.sh 编译 Release 版本
    "$SCRIPT_DIR/build-plugins.sh" Release "$plugins_output"

    log_success "Plugins embedded"
}

# ============================================================================
# 签名
# ============================================================================
sign_app() {
    if ! $DO_SIGN; then
        log_warn "Skipping code signing (use --sign to enable)"
        return
    fi

    log_info "Signing app..."

    local app_path="$BUILD_DIR/$APP_NAME.app"

    # 1. 签名所有内嵌的 dylib 和 framework
    find "$app_path/Contents/Frameworks" -type f \( -name "*.dylib" -o -perm +111 \) 2>/dev/null | while read -r file; do
        codesign --force --sign "$DEVELOPER_ID" "$file"
    done

    # 签名 ETermKit.framework
    if [ -d "$app_path/Contents/Frameworks/ETermKit.framework" ]; then
        codesign --force --sign "$DEVELOPER_ID" "$app_path/Contents/Frameworks/ETermKit.framework"
    fi

    # 2. 签名所有插件
    find "$app_path/Contents/PlugIns" -name "*.bundle" 2>/dev/null | while read -r bundle; do
        codesign --force --sign "$DEVELOPER_ID" "$bundle"
    done

    # 3. 签名 app
    codesign --force --sign "$DEVELOPER_ID" \
        --options runtime \
        --entitlements "$ETERM_XCODE_PROJECT/ETerm/ETerm.entitlements" \
        "$app_path"

    # 4. 验证签名
    codesign --verify --deep --strict "$app_path"

    log_success "App signed and verified"
}

# ============================================================================
# 公证
# ============================================================================
notarize_app() {
    if ! $DO_NOTARIZE; then
        log_warn "Skipping notarization (use --notarize to enable)"
        return
    fi

    log_info "Notarizing app..."

    local app_path="$BUILD_DIR/$APP_NAME.app"

    # 打包成 zip 用于公证
    ditto -c -k --keepParent "$app_path" "$BUILD_DIR/$APP_NAME.zip"

    # 提交公证
    xcrun notarytool submit "$BUILD_DIR/$APP_NAME.zip" \
        --apple-id "$APPLE_ID" \
        --team-id "$TEAM_ID" \
        --password "$APP_SPECIFIC_PASSWORD" \
        --wait

    # Staple
    xcrun stapler staple "$app_path"

    # 清理 zip
    rm "$BUILD_DIR/$APP_NAME.zip"

    log_success "App notarized and stapled"
}

# ============================================================================
# 创建 DMG
# ============================================================================
create_dmg() {
    log_info "Creating DMG..."

    mkdir -p "$OUTPUT_DIR"

    local dmg_name="${APP_NAME}-${VERSION}.dmg"
    local dmg_path="$OUTPUT_DIR/$dmg_name"

    # 删除旧的 DMG
    rm -f "$dmg_path"

    # 创建临时目录
    local dmg_staging="$BUILD_DIR/dmg-staging"
    rm -rf "$dmg_staging"
    mkdir -p "$dmg_staging"

    # 复制 app
    cp -R "$BUILD_DIR/$APP_NAME.app" "$dmg_staging/"

    # 创建 Applications 链接
    ln -s /Applications "$dmg_staging/Applications"

    # 创建 DMG
    hdiutil create \
        -volname "$APP_NAME" \
        -srcfolder "$dmg_staging" \
        -ov \
        -format UDZO \
        "$dmg_path"

    # 清理
    rm -rf "$dmg_staging"

    log_success "DMG created: $dmg_path"
    echo ""
    echo "================================================"
    echo "  Output: $dmg_path"
    echo "  Size: $(du -h "$dmg_path" | cut -f1)"
    echo "================================================"
}

# ============================================================================
# 主流程
# ============================================================================
main() {
    log_info "ETerm Package Script"
    log_info "Version: $VERSION"
    log_info "Sign: $DO_SIGN | Notarize: $DO_NOTARIZE"
    echo ""

    check_requirements
    build_app
    build_and_embed_plugins
    sign_app
    notarize_app
    create_dmg

    echo ""
    log_success "Packaging completed!"
}

main "$@"
