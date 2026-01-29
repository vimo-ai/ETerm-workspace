# FilePreviewKit 设计文档

## 概述

为 ETerm 提供内嵌文件浏览和预览能力，使用户无需切换到 Finder 即可完成文件浏览、预览操作。

## 设计目标

1. **自包含开发环境** - 终端 + 文件浏览 + 预览，不用切出 ETerm
2. **插件化** - 作为独立插件实现，不入侵核心 Tab 系统
3. **复用系统能力** - 最大化利用 macOS 原生 API
4. **与 Workspace 整合** - 工作区标记融入文件浏览器

## 整体架构

```
┌─ FilePreviewKit 插件 ─────────────────────────────────────┐
│                                                           │
│  ┌─ FileBrowserView ─────────────────────────────────┐   │
│  │                                                    │   │
│  │  浏览整个文件系统，从指定 cwd 开始                   │   │
│  │  支持 Workspace 标记显示                           │   │
│  │                                                    │   │
│  └────────────────────────────────────────────────────┘   │
│                                                           │
│  ┌─ FilePreviewView ─────────────────────────────────┐   │
│  │                                                    │   │
│  │  Quick Look (系统) 或 内嵌预览                      │   │
│  │                                                    │   │
│  └────────────────────────────────────────────────────┘   │
│                                                           │
└───────────────────────────────────────────────────────────┘
```

## 文件浏览器设计

### 核心功能

| 功能 | 说明 |
|------|------|
| 目录浏览 | 树形展开，支持整个文件系统 |
| 文件预览 | 点击/空格触发预览 |
| Workspace 标记 | 右键标记/取消，显示标记图标 |
| 快捷跳转 | 跳转到已标记的文件夹 |

### UI 展示

```
┌─ File Browser Tab ──────────────────────┐
│ 📁 ~/Desktop/vimo/                      │
│ ├── 📁⭐ ETerm/          ← 工作区标记   │
│ │   ├── 📁 src/                         │
│ │   ├── 📁 Plugins/                     │
│ │   └── 📄 README.md     ← 点击预览     │
│ ├── 📁⭐ memex/          ← 工作区标记   │
│ └── 📁 waveterm/                        │
└─────────────────────────────────────────┘
```

### 右键菜单

```
右键文件夹：
┌─────────────────────────┐
│ 在 Finder 中显示        │
│ 在终端中打开            │
│ ──────────────────────  │
│ ⭐ 标记为工作区         │  ← Workspace 功能
│ ──────────────────────  │
│ 新建文件                │
│ 新建文件夹              │
└─────────────────────────┘

右键文件：
┌─────────────────────────┐
│ 预览                    │
│ 在默认应用中打开        │
│ ──────────────────────  │
│ 复制路径                │
└─────────────────────────┘
```

## Workspace 整合

### 设计原则

**Workspace 不是独立入口，是文件夹的属性。**

- 不显式呈现"收藏夹"概念
- 右键标记，文件夹上显示小图标
- 复用现有 WorkspaceKit 的数据存储

### 数据复用

```swift
// 复用 WorkspaceKit 的数据库
// ~/.vimo/data/workspace.db
// 表: ZWORKSPACEFOLDER (path, addedAt)
```

### 交互

| 操作 | 效果 |
|------|------|
| 右键 → "标记为工作区" | 添加到 workspace.db，显示 ⭐ 图标 |
| 右键 → "取消标记" | 从 workspace.db 移除，隐藏 ⭐ 图标 |
| 快捷跳转 | 可从某处快速跳到已标记的文件夹 |

## 入口设计

### 两级入口

| 层级 | 入口位置 | 打开的 cwd | 场景 |
|------|---------|-----------|------|
| **Window 层级** | Menu bar 📁 按钮 | `~` | 通用浏览 |
| **Tab 层级** | Terminal Tab 右键菜单 | 该 Tab 的 cwd 快照 | 项目内浏览 |

### Window 层级

```
┌─ Menu Bar ──────────────────────────────┐
│  ETerm    File    Edit    [📁]          │
└───────────────────────────┬─────────────┘
                            │ 点击
                            ▼
        在当前 focus 的 Panel 创建 File Browser Tab
        cwd = ~
```

### Tab 层级

```
右键 Terminal Tab
┌─────────────────────────┐
│ 关闭                    │
│ 重命名                  │
│ ──────────────────────  │
│ 📁 打开文件浏览器        │
└─────────────────────────┘
        ↓
在同 Panel，该 Tab 右侧创建 File Browser Tab
cwd = 该 Terminal 当时的 cwd（快照）
```

### Tab 行为

- **快照模式** - 打开时 cwd 固定，不跟 Terminal 同步
- **可多开** - 可同时打开多个 File Browser Tab，各自独立
- **等同 Cmd+T** - 在 focus Tab 右侧创建

## 文件预览设计

### 预览能力

| 类型 | 实现方案 |
|------|---------|
| **Quick Look** | `QLPreviewPanel` - 系统级，支持几乎所有格式 |
| **Markdown** | 内嵌渲染（可选，Phase 2） |
| **代码** | 语法高亮（可选，Phase 2） |
| **图片** | 内嵌显示（可选，Phase 2） |

### 触发方式

| 操作 | 效果 |
|------|------|
| 单击文件 | 选中 |
| 空格 | Quick Look 预览 |
| 双击 / Enter | 在默认应用中打开（或内嵌预览 Tab） |

### Phase 1 策略

优先使用 **Quick Look**，零成本支持所有格式。内嵌预览作为 Phase 2 增强。

## 技术实现

### 复用系统能力

```swift
// 文件图标
NSWorkspace.shared.icon(forFile: path)

// 文件类型
UTType(filenameExtension: ext)

// Quick Look 预览
QLPreviewPanel.shared()

// 文件变化监听
DispatchSource.makeFileSystemObjectSource(
    fileDescriptor: fd,
    eventMask: .write
)
```

### 文件树实现

```swift
struct FileBrowserView: View {
    @State private var rootPath: String
    @State private var expandedFolders: Set<String>

    var body: some View {
        List {
            OutlineGroup(fileTree, children: \.children) { node in
                FileRowView(node: node)
            }
        }
    }
}

struct FileNode: Identifiable {
    let id: String  // full path
    let name: String
    let isDirectory: Bool
    let isWorkspace: Bool  // 是否被标记
    var children: [FileNode]?
}
```

### Tab 创建

```swift
// 插件通过 HostBridge 创建 Tab
host.createTab(
    in: panelId,
    content: .view(FileBrowserContent(cwd: cwd)),
    position: .afterActive
)
```

## 分阶段规划

### Phase 1: 基础浏览 + Quick Look

- [ ] FileBrowserView 基础实现
- [ ] 目录树遍历与展示
- [ ] 空格触发 Quick Look
- [ ] Menu bar 入口
- [ ] Terminal Tab 右键菜单入口

### Phase 2: Workspace 整合

- [ ] Workspace 标记显示（⭐ 图标）
- [ ] 右键标记/取消标记
- [ ] 快捷跳转到标记文件夹

### Phase 3: 增强预览

- [ ] Markdown 内嵌渲染
- [ ] 代码语法高亮
- [ ] 图片内嵌显示

### Phase 4: 文件操作

- [ ] 新建文件/文件夹
- [ ] 删除（移到废纸篓）
- [ ] 重命名

## 参考

- **Waveterm** - DirectoryPreview + PreviewModel 设计
- **macOS Finder** - 交互参考
- **VS Code Explorer** - 工作区概念

## 附录：与 Waveterm 对比

| 方面 | Waveterm | ETerm FilePreviewKit |
|------|----------|---------------------|
| 技术栈 | Electron + React | SwiftUI + 原生 |
| 预览 | 自己实现（react-markdown 等） | Quick Look + 可选内嵌 |
| 入口 | 目录作为 Block 类型 | Menu bar + Tab 右键 |
| 工作区 | 无 | Workspace 标记融合 |
| 终端路径点击 | 不支持 | 暂不支持（可后续扩展） |
