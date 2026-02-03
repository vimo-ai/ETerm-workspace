import SwiftUI
import UniformTypeIdentifiers

/// 项目树节点
enum ProjectTreeNode: Identifiable {
    case folder(name: String, path: String, children: [ProjectTreeNode])
    case project(ProjectInfo)

    var id: String {
        switch self {
        case .folder(_, let path, _): return "folder:\(path)"
        case .project(let p): return "project:\(p.path)"
        }
    }
}

struct ContentView: View {
    @EnvironmentObject var runner: DevRunner
    @State private var isBuilding = false
    @State private var errorMessage: String?
    @State private var terminalController: MultiTerminalController?
    @StateObject private var tabManager = TerminalTabManager()
    @State private var isDraggingOver = false  // 拖拽状态
    @State private var expandedWorkspaces: Set<UUID> = []  // 展开的 workspace
    @State private var collapsedGroups: Set<String> = []  // 折叠的路径分组
    @State private var expandedFolders: Set<String> = []  // 展开的文件夹路径

    var body: some View {
        HStack(spacing: 0) {
            // Sidebar
            sidebar
                .frame(width: 220)

            // Divider
            Rectangle()
                .fill(Theme.border)
                .frame(width: 1)

            // Main content
            if runner.selectedProject != nil {
                mainContent
            } else {
                emptyState
            }
        }
        .background(Theme.bgPrimary)
        .preferredColorScheme(.dark)
        .alert("Error", isPresented: .constant(errorMessage != nil)) {
            Button("OK") { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "")
        }
    }

    // MARK: - Computed Properties

    /// 按路径前缀分组的 workspaces（三层结构：路径组 → Workspace → Project）
    private var groupedWorkspaces: [(id: String, groupName: String?, workspaces: [Workspace])] {
        let homeDir = FileManager.default.homeDirectoryForCurrentUser.path

        // 按 ~/Desktop/xxx 这一级分组
        let grouped = Dictionary(grouping: runner.workspaces) { workspace -> String in
            let path = workspace.path
            guard path.hasPrefix(homeDir) else { return "__root__" }

            let relativePath = String(path.dropFirst(homeDir.count))
                .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let components = relativePath.split(separator: "/").map(String.init)

            // 取前两级作为分组 key（如 Desktop/vimo）
            if components.count >= 3 {
                return "\(homeDir)/\(components[0])/\(components[1])"
            } else {
                return "__standalone__\(path)"
            }
        }

        var result: [(id: String, groupName: String?, workspaces: [Workspace])] = []
        for (groupKey, workspaces) in grouped {
            if groupKey.hasPrefix("__standalone__") || groupKey == "__root__" {
                result.append((id: groupKey, groupName: nil, workspaces: workspaces))
            } else {
                let displayPath = groupKey.replacingOccurrences(of: homeDir, with: "~")
                result.append((id: groupKey, groupName: displayPath, workspaces: workspaces))
            }
        }

        // 排序：有 groupName 的在前
        return result.sorted { lhs, rhs in
            switch (lhs.groupName, rhs.groupName) {
            case (nil, nil): return lhs.id < rhs.id
            case (nil, _): return false
            case (_, nil): return true
            case let (l?, r?): return l < r
            }
        }
    }

    // MARK: - Helper Methods

    /// 切换路径分组展开/折叠
    private func toggleGroupCollapse(_ groupId: String) {
        if collapsedGroups.contains(groupId) {
            collapsedGroups.remove(groupId)
        } else {
            collapsedGroups.insert(groupId)
        }
    }

    /// 切换 workspace 展开/折叠
    private func toggleWorkspaceExpansion(_ id: UUID) {
        if expandedWorkspaces.contains(id) {
            expandedWorkspaces.remove(id)
        } else {
            expandedWorkspaces.insert(id)
        }
    }

    /// 切换文件夹展开/折叠
    private func toggleFolder(_ folderPath: String) {
        if expandedFolders.contains(folderPath) {
            expandedFolders.remove(folderPath)
        } else {
            expandedFolders.insert(folderPath)
        }
    }

    /// 将 projects 按相对路径构建成树
    private func buildProjectTree(workspace: Workspace) -> [ProjectTreeNode] {
        let workspacePath = workspace.path

        // 构建临时树结构 [path: children]
        var tree: [String: [ProjectTreeNode]] = [:]

        for project in workspace.projects {
            // 计算相对路径
            let relativePath = project.path
                .replacingOccurrences(of: workspacePath, with: "")
                .trimmingCharacters(in: CharacterSet(charactersIn: "/"))

            let components = relativePath.split(separator: "/").map(String.init)

            // 如果只有一层（根目录项目），直接加到根层级
            if components.count == 1 {
                tree["", default: []].append(.project(project))
                continue
            }

            // 多层结构：构建中间文件夹
            // 例如 "dev-runner/DevRunner" → folder: dev-runner, project: DevRunner
            let folderComponents = Array(components.dropLast())
            var currentPath = ""

            for (index, component) in folderComponents.enumerated() {
                let parentPath = currentPath
                currentPath = currentPath.isEmpty ? component : "\(currentPath)/\(component)"

                // 如果是最后一个文件夹，添加项目
                if index == folderComponents.count - 1 {
                    tree[currentPath, default: []].append(.project(project))
                }
            }
        }

        // 递归构建树节点
        func buildNodes(at path: String) -> [ProjectTreeNode] {
            guard let children = tree[path] else { return [] }

            var nodes: [ProjectTreeNode] = []

            // 收集所有子文件夹
            let childFolders = tree.keys.filter { key in
                guard !key.isEmpty else { return false }
                let parentPath = (key as NSString).deletingLastPathComponent
                return path.isEmpty ? !parentPath.contains("/") : parentPath == path
            }

            // 添加文件夹节点
            for folderPath in childFolders.sorted() {
                let folderName = (folderPath as NSString).lastPathComponent
                let folderChildren = buildNodes(at: folderPath)
                nodes.append(.folder(name: folderName, path: folderPath, children: folderChildren))
            }

            // 添加当前路径下的项目节点
            nodes.append(contentsOf: children)

            return nodes
        }

        return buildNodes(at: "")
    }

    // MARK: - Sidebar Row Views

    /// Workspace 行
    @ViewBuilder
    private func workspaceRow(workspace: Workspace, indented: Bool) -> some View {
        Button {
            toggleWorkspaceExpansion(workspace.id)
        } label: {
            HStack(spacing: 6) {
                // 折叠箭头
                Image(systemName: expandedWorkspaces.contains(workspace.id) ? "chevron.down" : "chevron.right")
                    .font(.system(size: 9))
                    .foregroundColor(Theme.textMuted)
                    .frame(width: 12)

                // 文件夹图标
                Image(systemName: "folder.fill")
                    .font(.system(size: 12))
                    .foregroundColor(Theme.accent)

                // Workspace 名称
                Text(workspace.name)
                    .font(.system(size: 12, weight: .medium, design: .monospaced))
                    .foregroundColor(Theme.textPrimary)

                Spacer()

                // 项目数量 badge
                if workspace.projects.count > 1 {
                    Text("\(workspace.projects.count)")
                        .font(.system(size: 9, weight: .medium, design: .monospaced))
                        .foregroundColor(Theme.textMuted)
                        .padding(.horizontal, 5)
                        .padding(.vertical, 2)
                        .background(Theme.bgPrimary.opacity(0.5))
                        .clipShape(RoundedRectangle(cornerRadius: 3))
                }
            }
            .padding(.leading, indented ? 20 : 12)
            .padding(.trailing, 12)
            .padding(.vertical, 6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// Project 行
    @ViewBuilder
    private func projectRow(project: ProjectInfo, indented: Bool) -> some View {
        let isSelected = runner.selectedProject?.path == project.path

        Button {
            runner.selectProject(project)
        } label: {
            HStack(spacing: 6) {
                // 类型图标
                Image(systemName: project.adapterType == "xcode" ? "hammer.fill" : "cube.fill")
                    .font(.system(size: 11))
                    .foregroundColor(project.adapterType == "xcode" ? Theme.xcode : Theme.node)

                // 项目名
                Text(project.name)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundColor(isSelected ? Theme.textPrimary : Theme.textSecondary)
                    .lineLimit(1)

                Spacer()

                // 类型 badge
                Text(project.adapterType == "xcode" ? "Xcode" : "Node")
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundColor(project.adapterType == "xcode" ? Theme.xcode : Theme.node)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background((project.adapterType == "xcode" ? Theme.xcode : Theme.node).opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 3))
            }
            .padding(.leading, indented ? 36 : 28)
            .padding(.trailing, 12)
            .padding(.vertical, 5)
            .background(isSelected ? Theme.bgHover : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// Project 行（带深度参数）
    @ViewBuilder
    private func projectRowWithDepth(project: ProjectInfo, depth: Int, groupIndented: Bool) -> some View {
        let isSelected = runner.selectedProject?.path == project.path
        let baseIndent: CGFloat = groupIndented ? 36 : 28
        let depthIndent: CGFloat = CGFloat(depth * 12)

        Button {
            runner.selectProject(project)
        } label: {
            HStack(spacing: 6) {
                // 类型图标
                Image(systemName: project.adapterType == "xcode" ? "hammer.fill" : "cube.fill")
                    .font(.system(size: 11))
                    .foregroundColor(project.adapterType == "xcode" ? Theme.xcode : Theme.node)

                // 项目名
                Text(project.name)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundColor(isSelected ? Theme.textPrimary : Theme.textSecondary)
                    .lineLimit(1)

                Spacer()

                // 类型 badge
                Text(project.adapterType == "xcode" ? "Xcode" : "Node")
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundColor(project.adapterType == "xcode" ? Theme.xcode : Theme.node)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background((project.adapterType == "xcode" ? Theme.xcode : Theme.node).opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 3))
            }
            .padding(.leading, baseIndent + depthIndent)
            .padding(.trailing, 12)
            .padding(.vertical, 5)
            .background(isSelected ? Theme.bgHover : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// 递归渲染项目树（使用 AnyView 解决递归类型问题）
    private func renderProjectTree(nodes: [ProjectTreeNode], depth: Int, groupIndented: Bool) -> AnyView {
        AnyView(
            ForEach(nodes) { node in
                switch node {
                case .folder(let name, let path, let children):
                    VStack(spacing: 0) {
                        // 文件夹行
                        Button {
                            toggleFolder(path)
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: expandedFolders.contains(path) ? "chevron.down" : "chevron.right")
                                    .font(.system(size: 8))
                                    .foregroundColor(Theme.textMuted)
                                Image(systemName: "folder")
                                    .font(.system(size: 10))
                                    .foregroundColor(Theme.textMuted)
                                Text(name)
                                    .font(.system(size: 11, design: .monospaced))
                                    .foregroundColor(Theme.textMuted)
                                Spacer()
                            }
                            .padding(.leading, CGFloat(depth * 12) + (groupIndented ? 36 : 28))
                            .padding(.trailing, 12)
                            .padding(.vertical, 4)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)

                        // 子节点（展开时）
                        if expandedFolders.contains(path) {
                            renderProjectTree(nodes: children, depth: depth + 1, groupIndented: groupIndented)
                        }
                    }

                case .project(let project):
                    // 项目行
                    projectRowWithDepth(project: project, depth: depth, groupIndented: groupIndented)
                }
            }
        )
    }

    // MARK: - Sidebar

    private var sidebar: some View {
        VStack(spacing: 0) {
            // Header
            HStack {
                Text("PROJECTS")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundColor(Theme.textMuted)
                    .tracking(1.5)
                Spacer()
            }
            .padding(.horizontal, 16)
            .padding(.top, 20)
            .padding(.bottom, 12)

            // 路径分组 → Workspace → Project 三层树形列表
            ScrollView {
                VStack(spacing: 2) {
                    ForEach(groupedWorkspaces, id: \.id) { group in
                        // 路径分组头（如果有）
                        if let groupName = group.groupName {
                            Button {
                                toggleGroupCollapse(group.id)
                            } label: {
                                HStack(spacing: 6) {
                                    Image(systemName: collapsedGroups.contains(group.id) ? "chevron.right" : "chevron.down")
                                        .font(.system(size: 9, weight: .semibold))
                                        .foregroundColor(Theme.textMuted)
                                        .frame(width: 12)
                                    Text(groupName)
                                        .font(.system(size: 10, design: .monospaced))
                                        .foregroundColor(Theme.textMuted)
                                    Spacer()
                                }
                                .padding(.horizontal, 12)
                                .padding(.vertical, 6)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }

                        // 分组内的 Workspaces（未折叠时显示）
                        if group.groupName == nil || !collapsedGroups.contains(group.id) {
                            ForEach(group.workspaces) { workspace in
                                // Workspace 行
                                workspaceRow(workspace: workspace, indented: group.groupName != nil)

                                // 展开时显示 Projects（使用树形结构）
                                if expandedWorkspaces.contains(workspace.id) {
                                    let tree = buildProjectTree(workspace: workspace)
                                    renderProjectTree(nodes: tree, depth: 0, groupIndented: group.groupName != nil)
                                }
                            }
                        }
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
            }

            Spacer()

            // Add button
            Button {
                addWorkspace()
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "plus")
                        .font(.system(size: 12, weight: .semibold))
                    Text("Add Workspace")
                        .font(.system(size: 12, weight: .medium, design: .monospaced))
                }
                .foregroundColor(Theme.accent)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Theme.accent.opacity(0.3), lineWidth: 1)
                        .background(Theme.accent.opacity(0.05))
                )
                .clipShape(RoundedRectangle(cornerRadius: 6))
            }
            .buttonStyle(.plain)
            .padding(12)
        }
        .background(Theme.bgSecondary)
        .overlay(
            // 拖拽高亮边框
            RoundedRectangle(cornerRadius: 0)
                .stroke(Theme.accent, lineWidth: isDraggingOver ? 2 : 0)
                .animation(.easeInOut(duration: 0.2), value: isDraggingOver)
        )
        .onDrop(of: [.fileURL], isTargeted: $isDraggingOver) { providers in
            handleDrop(providers: providers)
        }
    }

    // MARK: - Empty State

    private var emptyState: some View {
        VStack(spacing: 20) {
            Image(systemName: runner.workspaces.isEmpty ? "cube.transparent" : "arrow.left")
                .font(.system(size: 48, weight: .thin))
                .foregroundColor(Theme.accent.opacity(0.5))

            VStack(spacing: 8) {
                Text(runner.workspaces.isEmpty ? "NO WORKSPACE" : "NO PROJECT SELECTED")
                    .font(.system(size: 14, weight: .bold, design: .monospaced))
                    .foregroundColor(Theme.textPrimary)
                    .tracking(2)

                Text(runner.workspaces.isEmpty ? "Add a workspace to begin" : "Select a project from the sidebar")
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundColor(Theme.textSecondary)
            }

            if runner.workspaces.isEmpty {
                Button {
                    addWorkspace()
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "plus")
                        Text("Add Workspace")
                    }
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundColor(Theme.bgPrimary)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 10)
                    .background(Theme.accent)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
                .buttonStyle(.plain)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.bgPrimary)
    }

    // MARK: - Main Content

    private var mainContent: some View {
        VStack(spacing: 0) {
            // Header bar
            headerBar

            Rectangle()
                .fill(Theme.border)
                .frame(height: 1)

            // Selectors
            selectorsBar

            Rectangle()
                .fill(Theme.border)
                .frame(height: 1)

            // Terminal output
            terminalArea
        }
        .background(Theme.bgPrimary)
    }

    private var headerBar: some View {
        HStack(spacing: 16) {
            // 当前选中的 Project 信息
            if let project = runner.selectedProject {
                HStack(spacing: 8) {
                    // 项目类型图标
                    Image(systemName: project.adapterType == "xcode" ? "hammer.fill" : "cube.fill")
                        .font(.system(size: 14))
                        .foregroundColor(project.adapterType == "xcode" ? Theme.xcode : Theme.node)

                    // 项目名称
                    Text(project.name)
                        .font(.system(size: 16, weight: .semibold, design: .monospaced))
                        .foregroundColor(Theme.textPrimary)

                    // 类型 badge
                    Text(project.adapterType == "xcode" ? "Xcode" : "Node")
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundColor(project.adapterType == "xcode" ? Theme.xcode : Theme.node)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background((project.adapterType == "xcode" ? Theme.xcode : Theme.node).opacity(0.15))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
            }

            Spacer()

            // Action buttons
            HStack(spacing: 8) {
                SciFiButton(
                    title: "Build",
                    icon: "hammer.fill",
                    isActive: false,
                    isDisabled: runner.selectedTarget == nil || isBuilding,
                    action: build
                )

                SciFiButton(
                    title: "Run",
                    icon: "play.fill",
                    isActive: false,
                    isDisabled: runner.selectedTarget == nil || isBuilding,
                    action: run
                )

                SciFiButton(
                    title: "Stop",
                    icon: "stop.fill",
                    isActive: false,
                    isDisabled: terminalController == nil,
                    action: stop
                )

                SciFiButton(
                    title: "Clear",
                    icon: "trash",
                    isActive: false,
                    isDisabled: terminalController == nil,
                    action: clear
                )
            }

            if isBuilding {
                ProgressView()
                    .scaleEffect(0.6)
                    .tint(Theme.accent)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .background(Theme.bgSecondary)
    }

    private var selectorsBar: some View {
        HStack(spacing: 16) {
            // Target - with type badge
            SciFiPicker(
                "Target",
                icon: "target",
                selection: $runner.selectedTarget,
                options: runner.targets,
                optionLabel: { $0.name },
                optionBadge: { target in
                    let type = target.targetType.lowercased()
                    if type.contains("app") {
                        return ("App", Theme.accent)
                    } else if type.contains("test") {
                        return ("Test", Theme.warning)
                    } else if type.contains("framework") || type.contains("lib") {
                        return ("Lib", Theme.accentAlt)
                    } else {
                        return (target.targetType, Theme.textSecondary)
                    }
                }
            )
            .frame(width: 200)

            // Device
            SciFiPicker(
                "Device",
                icon: "desktopcomputer",
                selection: $runner.selectedDevice,
                options: runner.devices.filter(\.isAvailable),
                optionLabel: { $0.name },
                optionIcon: { device in
                    if device.isMac { return "desktopcomputer" }
                    if device.isSimulator { return "iphone.gen2" }
                    return "iphone"
                }
            )
            .frame(width: 180)

            Spacer()
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .background(Theme.bgSecondary.opacity(0.5))
    }

    private var terminalArea: some View {
        VStack(spacing: 0) {
            // Tab Bar
            TerminalTabBar(tabManager: tabManager) {
                // Add new tab
                terminalController?.createTab()
            }

            Rectangle()
                .fill(Theme.border)
                .frame(height: 1)

            // Terminal View
            MultiTerminalView(
                workingDirectory: runner.selectedProject?.path ?? FileManager.default.currentDirectoryPath,
                tabManager: tabManager
            ) { controller in
                print("[ContentView] terminalArea: onReady called")
                DispatchQueue.main.async {
                    self.terminalController = controller
                    print("[ContentView] terminalArea: controller set")
                }
            }
        }
    }

    // MARK: - Actions

    private func addWorkspace() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.message = "Select a project directory"

        if panel.runModal() == .OK, let url = panel.url {
            do {
                try runner.addWorkspace(path: url.path)
                if let ws = runner.selectedWorkspace {
                    terminalController?.sendCommand("echo '// Workspace: \(ws.name)'")
                    terminalController?.sendCommand("echo '// Found \(ws.projects.count) project(s)'")
                    for project in ws.projects {
                        terminalController?.sendCommand("echo '//   → \(project.name) (\(project.adapterType))'")
                    }
                }
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    // 处理拖拽导入
    private func handleDrop(providers: [NSItemProvider]) -> Bool {
        guard let provider = providers.first else { return false }

        // 加载文件 URL
        provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { (urlData, error) in
            DispatchQueue.main.async {
                if let error = error {
                    self.errorMessage = "Failed to load dropped item: \(error.localizedDescription)"
                    return
                }

                guard let urlData = urlData as? Data,
                      let url = URL(dataRepresentation: urlData, relativeTo: nil) else {
                    self.errorMessage = "Invalid file URL"
                    return
                }

                // 检查是否是目录
                var isDirectory: ObjCBool = false
                guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory),
                      isDirectory.boolValue else {
                    self.errorMessage = "Please drop a folder, not a file"
                    return
                }

                // 导入 workspace
                do {
                    try self.runner.addWorkspace(path: url.path)
                    if let ws = self.runner.selectedWorkspace {
                        self.terminalController?.sendCommand("echo '// Workspace: \(ws.name)'")
                        self.terminalController?.sendCommand("echo '// Found \(ws.projects.count) project(s)'")
                        for project in ws.projects {
                            self.terminalController?.sendCommand("echo '//   → \(project.name) (\(project.adapterType))'")
                        }
                    }
                } catch {
                    self.errorMessage = error.localizedDescription
                }
            }
        }

        return true
    }

    private func build() {
        print("[ContentView] build: called, controller=\(terminalController != nil)")
        guard runner.selectedTarget != nil else {
            print("[ContentView] build: no target selected")
            return
        }
        isBuilding = true

        Task {
            do {
                let options = BuildOptions(
                    config: "Debug",
                    deviceId: runner.selectedDevice?.deviceId
                )
                let cmd = try runner.buildCommand(options: options)

                // 构建完整的命令字符串
                var fullCommand = cmd.program
                if !cmd.args.isEmpty {
                    fullCommand += " " + cmd.args.joined(separator: " ")
                }

                print("[ContentView] build: command=\(fullCommand)")

                // 如果有 cwd，先 cd 过去
                if let cwd = cmd.cwd {
                    terminalController?.sendCommand("cd '\(cwd)' && \(fullCommand)")
                } else {
                    terminalController?.sendCommand(fullCommand)
                }
            } catch {
                print("[ContentView] build: error=\(error)")
                terminalController?.sendCommand("echo 'Error: \(error.localizedDescription)'")
            }
            isBuilding = false
        }
    }

    private func run() {
        guard runner.selectedTarget != nil else { return }

        Task {
            do {
                let options = RunOptions(deviceId: runner.selectedDevice?.deviceId)
                let cmd = try runner.runCommand(options: options)

                // 构建完整的命令字符串
                var fullCommand = cmd.program
                if !cmd.args.isEmpty {
                    fullCommand += " " + cmd.args.joined(separator: " ")
                }

                // 如果有 cwd，先 cd 过去
                if let cwd = cmd.cwd {
                    terminalController?.sendCommand("cd '\(cwd)' && \(fullCommand)")
                } else {
                    terminalController?.sendCommand(fullCommand)
                }
            } catch {
                terminalController?.sendCommand("echo 'Error: \(error.localizedDescription)'")
            }
        }
    }

    private func stop() {
        terminalController?.sendInterrupt()
    }

    private func clear() {
        terminalController?.clear()
    }
}

#Preview {
    ContentView()
}
