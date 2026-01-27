# vimo-agent 部署架构实施计划

> 创建时间: 2026-01-25
> 状态: 待实施

## 1. 背景

vimo-agent 是 vimo 生态的基建组件，是所有 DB 写入的唯一入口。

### 依赖产品

| 产品 | 类型 | 运行方式 |
|------|------|----------|
| memex-rs | 独立 CLI | 直接运行，使用 AgentClient |
| vlaude-daemon | 独立守护进程 | 直接运行，使用 AgentClient |
| MemexKit | ETerm 插件 | 启动 memex 进程 + 自己也用 AgentClient |
| VlaudeKit | ETerm 插件 | 启动 vlaude-daemon + 自己也用 AgentClient |

### 共享底层

- MemexKit 和 memex-rs 共享同一个 AgentClient 实现
- VlaudeKit 和 vlaude-daemon 共享同一个 AgentClient 实现
- 所有产品共享同一个 `~/.vimo/bin/vimo-agent`

## 2. 当前问题

### AgentClient 查找逻辑 (connect.rs)

```
1. ~/.vimo/bin/vimo-agent（默认路径）
2. current_exe().parent()/vimo-agent（源路径）→ 自动部署到 ~/.vimo/bin/
```

### 问题场景

| 场景 | current_exe | 查找路径 | 结果 |
|------|-------------|----------|------|
| memex-rs 独立运行 | ~/.../memex | ~/.../vimo-agent | ✅ |
| vlaude-daemon 独立运行 | ~/.../vlaude-daemon | ~/.../vimo-agent | ✅ |
| MemexKit 插件运行 | ETerm.app/.../ETerm | ETerm.app/.../vimo-agent | ❌ |
| VlaudeKit 插件运行 | ETerm.app/.../ETerm | ETerm.app/.../vimo-agent | ❌ |

**问题**: 插件作为 ETerm 的动态库加载，`current_exe()` 返回 ETerm 路径，找不到 bundle 内的 vimo-agent。

## 3. 解决方案

### 3.1 CI 层面

#### ai-cli-session-db release.yml

**修改**: 添加 vimo-agent 构建和发布

```yaml
- name: Build vimo-agent
  run: |
    cargo build --release --bin vimo-agent --features agent

- name: Create Release
  files: |
    target/release/libai_cli_session_db.dylib
    target/release/vimo-agent              # 新增
    include/ai_cli_session_db.h
```

#### memex release.yml

**修改**: 下载 vimo-agent，与 memex 同目录打包

```yaml
- name: Download vimo-agent
  run: |
    gh release download v$AGENT_VERSION \
      --repo vimo-ai/ai-cli-session-db \
      --pattern "vimo-agent-darwin-arm64" \
      -D .
    chmod +x vimo-agent-darwin-arm64
    mv vimo-agent-darwin-arm64 vimo-agent

- name: Create archive
  run: |
    tar -czvf memex-v$VERSION.tar.gz memex vimo-agent
```

#### vlaude release

**修改**: 同 memex，打包 vlaude-daemon + vimo-agent

#### plugin-deps.json

**修改**: 添加 vimo-agent 依赖

```json
{
  "memexkit": {
    "deps": {
      "libai_cli_session_db": { ... },
      "vimo-agent": {
        "repo": "vimo-ai/ai-cli-session-db",
        "tag": "v0.0.1-beta.x",
        "asset": "vimo-agent-darwin-arm64",
        "dest": "Lib/"
      }
    }
  },
  "vlaudekit": {
    "deps": {
      "libai_cli_session_db": { ... },
      "libsocket_client_ffi": { ... },
      "libvlaude_ffi": { ... },
      "vimo-agent": {
        "repo": "vimo-ai/ai-cli-session-db",
        "tag": "v0.0.1-beta.x",
        "asset": "vimo-agent-darwin-arm64",
        "dest": "Lib/"
      }
    }
  }
}
```

#### release-plugins.yml

**修改**: 处理 vimo-agent 下载和打包

```yaml
# 已有逻辑会从 plugin-deps.json 下载依赖到 dest 目录
# 需要在 "Build plugin" 步骤中复制 vimo-agent 到 bundle

- name: Build plugin
  run: |
    # ... 现有逻辑 ...

    # Copy vimo-agent if exists
    if [ -f "Lib/vimo-agent" ]; then
      cp "Lib/vimo-agent" "$BUNDLE_DIR/Contents/Lib/"
      chmod +x "$BUNDLE_DIR/Contents/Lib/vimo-agent"
    fi
```

### 3.2 Swift 层面

#### MemexKit/AgentDeployer.swift (新建)

```swift
import Foundation

enum AgentDeployError: Error, LocalizedError {
    case sourceNotFound
    case versionMismatch(installed: String, bundled: String)
    case copyFailed(Error)
    case permissionDenied

    var errorDescription: String? {
        switch self {
        case .sourceNotFound:
            return "vimo-agent not found in bundle"
        case .versionMismatch(let installed, let bundled):
            return """
                vimo-agent 版本冲突:
                - 已安装: \(installed)
                - Bundle: \(bundled)
                请手动删除 ~/.vimo/bin/vimo-agent 后重试
                """
        case .copyFailed(let error):
            return "Failed to deploy vimo-agent: \(error.localizedDescription)"
        case .permissionDenied:
            return "Permission denied writing to ~/.vimo/bin/"
        }
    }
}

struct AgentDeployer {

    static let targetDir = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(".vimo/bin")
    static let targetPath = targetDir.appendingPathComponent("vimo-agent")

    /// 部署 vimo-agent（如果需要）
    /// - Throws: AgentDeployError
    static func deployIfNeeded(from bundle: Bundle) throws {
        guard let sourcePath = bundle.url(forResource: "vimo-agent", withExtension: nil, subdirectory: "Lib") else {
            throw AgentDeployError.sourceNotFound
        }

        let fm = FileManager.default

        // 目标不存在 → 直接复制
        if !fm.fileExists(atPath: targetPath.path) {
            try deploy(from: sourcePath)
            return
        }

        // 版本比较
        let installedVersion = getVersion(at: targetPath)
        let bundledVersion = getVersion(at: sourcePath)

        if installedVersion == bundledVersion {
            // 版本一致，无需操作
            return
        }

        if compareVersions(bundledVersion, installedVersion) > 0 {
            // 升级
            try deploy(from: sourcePath)
        } else {
            // 源版本更低 → 报错
            throw AgentDeployError.versionMismatch(
                installed: installedVersion,
                bundled: bundledVersion
            )
        }
    }

    /// 原子部署
    private static func deploy(from source: URL) throws {
        let fm = FileManager.default

        // 创建目录
        try fm.createDirectory(at: targetDir, withIntermediateDirectories: true)

        // 原子复制：先写临时文件，再 rename
        let tempPath = targetDir.appendingPathComponent("vimo-agent.tmp.\(UUID().uuidString)")

        do {
            try fm.copyItem(at: source, to: tempPath)

            // 设置可执行权限
            try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tempPath.path)

            // 如果目标存在，先删除
            if fm.fileExists(atPath: targetPath.path) {
                try fm.removeItem(at: targetPath)
            }

            // 原子 rename
            try fm.moveItem(at: tempPath, to: targetPath)

        } catch {
            // 清理临时文件
            try? fm.removeItem(at: tempPath)
            throw AgentDeployError.copyFailed(error)
        }
    }

    /// 获取版本号
    private static func getVersion(at path: URL) -> String {
        let process = Process()
        process.executableURL = path
        process.arguments = ["--version"]

        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice

        do {
            try process.run()
            process.waitUntilExit()

            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            let output = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
            return output ?? "unknown"
        } catch {
            return "unknown"
        }
    }

    /// 比较版本号
    /// - Returns: >0 if v1 > v2, <0 if v1 < v2, 0 if equal
    private static func compareVersions(_ v1: String, _ v2: String) -> Int {
        // 简单的 semver 比较
        let parts1 = v1.replacingOccurrences(of: "-", with: ".").split(separator: ".")
        let parts2 = v2.replacingOccurrences(of: "-", with: ".").split(separator: ".")

        for i in 0..<max(parts1.count, parts2.count) {
            let p1 = i < parts1.count ? Int(parts1[i]) ?? 0 : 0
            let p2 = i < parts2.count ? Int(parts2[i]) ?? 0 : 0
            if p1 != p2 {
                return p1 - p2
            }
        }
        return 0
    }
}
```

#### MemexService.swift 修改

```swift
private init() {
    // 先部署 vimo-agent
    do {
        try AgentDeployer.deployIfNeeded(from: Bundle(for: Self.self))
    } catch {
        logError("[MemexKit] Agent deploy failed: \(error)")
        // 不继续初始化，让错误暴露
    }

    // 原有初始化逻辑...
    DispatchQueue.global(qos: .utility).async { [weak self] in
        self?.initSharedDb()
    }
}
```

#### VlaudeKit 同理

VlaudePlugin 或 VlaudeClient 初始化时调用 `AgentDeployer.deployIfNeeded()`

## 4. 错误处理原则

| 情况 | 行为 |
|------|------|
| 版本一致 | ✅ 跳过 |
| 源版本更高 | ✅ 升级 |
| 源版本更低 | ❌ 报错，提示手动删除后重试 |
| 复制失败 | ❌ 报错，提示手动处理 |
| 权限不足 | ❌ 报错，提示手动处理 |
| 源不存在 | ❌ 报错，提示 bundle 损坏 |

**原则**: 不做回退、不自动降级、不 cover 边界情况。出问题就报错让用户处理。

## 5. 发布产物结构

### memex release

```
memex-v1.0.0.tar.gz
├── memex
└── vimo-agent
```

### vlaude release

```
vlaude-v1.0.0.tar.gz
├── vlaude-daemon
└── vimo-agent
```

### MemexKit.bundle

```
MemexKit.bundle/
├── Contents/
│   ├── MacOS/
│   │   └── libMemexKit.dylib
│   ├── Lib/
│   │   ├── memex
│   │   └── vimo-agent    # 新增
│   ├── Libs/
│   │   └── libai_cli_session_db.dylib
│   └── Resources/
│       └── ...
```

### VlaudeKit.bundle

```
VlaudeKit.bundle/
├── Contents/
│   ├── MacOS/
│   │   └── libVlaudeKit.dylib
│   ├── Lib/
│   │   └── vimo-agent    # 新增
│   ├── Libs/
│   │   ├── libai_cli_session_db.dylib
│   │   ├── libsocket_client_ffi.dylib
│   │   └── libvlaude_ffi.dylib
│   └── Resources/
│       └── ...
```

## 6. 实施步骤

### Phase 1: CI 基础设施

1. [ ] 修改 `ai-cli-session-db/.github/workflows/release.yml` - 发布 vimo-agent
2. [ ] 修改 `ETerm/.github/plugin-deps.json` - 添加 vimo-agent 依赖
3. [ ] 修改 `ETerm/.github/workflows/release-plugins.yml` - 处理 vimo-agent

### Phase 2: 独立产品

4. [ ] 修改 `memex/.github/workflows/release.yml` - 打包 vimo-agent
5. [ ] 修改 vlaude release（如果有）- 打包 vimo-agent

### Phase 3: Swift 层

6. [ ] 创建 `AgentDeployer.swift` 共享模块
7. [ ] 修改 MemexKit - 启动时调用 AgentDeployer
8. [ ] 修改 VlaudeKit - 启动时调用 AgentDeployer

### Phase 4: 测试验证

9. [ ] 测试 memex-rs 独立运行
10. [ ] 测试 MemexKit 插件运行
11. [ ] 测试版本升级场景
12. [ ] 测试版本冲突报错

## 7. 回滚方案

如果出现问题，用户可以：

```bash
# 手动删除已部署的 agent
rm ~/.vimo/bin/vimo-agent

# 重新启动应用，会自动重新部署
```
