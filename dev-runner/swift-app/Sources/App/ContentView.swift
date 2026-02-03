import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject var runner: DevRunner
    @State private var isBuilding = false
    @State private var errorMessage: String?
    @State private var terminalController: MultiTerminalController?
    @StateObject private var tabManager = TerminalTabManager()
    @State private var isDraggingOver = false  // 拖拽状态
    @State private var collapsedGroups: Set<String> = []  // 折叠的分组

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
            if runner.selectedWorkspace != nil {
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

    /// 按顶层目录分组的 workspaces
    /// 逻辑：取 ~/Desktop/ 或 ~/Documents/ 等下的第一层目录作为分组
    /// 例如：~/Desktop/vimo/ETerm 和 ~/Desktop/vimo/calendar 都归到 ~/Desktop/vimo
    private var groupedWorkspaces: [(id: String, groupName: String?, workspaces: [Workspace])] {
        let homeDir = FileManager.default.homeDirectoryForCurrentUser.path

        // 计算每个 workspace 的分组 key
        // 规则：home 之后取前两级目录作为分组（如 Desktop/vimo）
        // 如果路径只有两级或更少，则不分组
        let grouped = Dictionary(grouping: runner.workspaces) { workspace -> String in
            let path = workspace.path

            // 去掉 home 前缀，得到相对路径
            guard path.hasPrefix(homeDir) else {
                return "__root__"  // 不在 home 下的路径
            }

            let relativePath = String(path.dropFirst(homeDir.count))
                .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let components = relativePath.split(separator: "/").map(String.init)

            // 至少需要 3 级才分组（如 Desktop/vimo/ETerm）
            // 取前两级作为分组 key（如 Desktop/vimo）
            if components.count >= 3 {
                return "\(homeDir)/\(components[0])/\(components[1])"
            } else {
                // 直接在 ~/Desktop/ 下的项目，不分组
                return "__standalone__\(path)"
            }
        }

        // 转换为数组
        var result: [(id: String, groupName: String?, workspaces: [Workspace])] = []

        for (groupKey, workspaces) in grouped {
            if groupKey.hasPrefix("__standalone__") || groupKey == "__root__" {
                // 单独的 workspace：不显示组头
                result.append((id: groupKey, groupName: nil, workspaces: workspaces))
            } else {
                // 有分组：显示分组头，缩短路径（home 替换为 ~）
                let displayPath = groupKey.replacingOccurrences(of: homeDir, with: "~")
                result.append((id: groupKey, groupName: displayPath, workspaces: workspaces))
            }
        }

        // 排序：有 groupName 的在前，nil 的在后；同类按 groupName 排序
        return result.sorted { lhs, rhs in
            switch (lhs.groupName, rhs.groupName) {
            case (nil, nil):
                return lhs.id < rhs.id
            case (nil, _):
                return false
            case (_, nil):
                return true
            case let (l?, r?):
                return l < r
            }
        }
    }

    // MARK: - Sidebar

    private var sidebar: some View {
        VStack(spacing: 0) {
            // Header
            HStack {
                Text("WORKSPACES")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundColor(Theme.textMuted)
                    .tracking(1.5)
                Spacer()
            }
            .padding(.horizontal, 16)
            .padding(.top, 20)
            .padding(.bottom, 12)

            // Workspace list
            ScrollView {
                VStack(spacing: 4) {
                    ForEach(groupedWorkspaces, id: \.id) { group in
                        // 分组头（如果有）
                        if let groupName = group.groupName {
                            Button {
                                // 切换折叠状态
                                if collapsedGroups.contains(group.id) {
                                    collapsedGroups.remove(group.id)
                                } else {
                                    collapsedGroups.insert(group.id)
                                }
                            } label: {
                                HStack(spacing: 6) {
                                    // 折叠小三角
                                    Image(systemName: collapsedGroups.contains(group.id) ? "chevron.right" : "chevron.down")
                                        .font(.system(size: 9, weight: .semibold))
                                        .foregroundColor(Theme.textMuted)
                                        .frame(width: 12)

                                    // 路径文字
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

                        // Workspace 列表（展开时）
                        if group.groupName == nil || !collapsedGroups.contains(group.id) {
                            ForEach(group.workspaces) { workspace in
                                SciFiSidebarItem(
                                    name: workspace.name,
                                    isSelected: runner.selectedWorkspace?.id == workspace.id,
                                    onSelect: { runner.selectWorkspace(workspace) },
                                    onRemove: { runner.removeWorkspace(workspace) }
                                )
                            }
                        }
                    }
                }
                .padding(.horizontal, 8)
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
            Image(systemName: "cube.transparent")
                .font(.system(size: 48, weight: .thin))
                .foregroundColor(Theme.accent.opacity(0.5))

            VStack(spacing: 8) {
                Text("NO WORKSPACE")
                    .font(.system(size: 14, weight: .bold, design: .monospaced))
                    .foregroundColor(Theme.textPrimary)
                    .tracking(2)

                Text("Add a workspace to begin")
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundColor(Theme.textSecondary)
            }

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
            // Workspace name
            if let ws = runner.selectedWorkspace {
                HStack(spacing: 8) {
                    Image(systemName: "cube.fill")
                        .font(.system(size: 14))
                        .foregroundColor(Theme.accent)
                    Text(ws.name)
                        .font(.system(size: 16, weight: .semibold, design: .monospaced))
                        .foregroundColor(Theme.textPrimary)
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
            // Project - with type badge
            SciFiPicker(
                "Project",
                icon: "folder.fill",
                selection: Binding(
                    get: { runner.selectedProject },
                    set: { if let p = $0 { runner.selectProject(p) } }
                ),
                options: runner.selectedWorkspace?.projects ?? [],
                optionLabel: { $0.name },
                optionIcon: { $0.adapterType == "xcode" ? "hammer.fill" : "terminal.fill" },
                optionBadge: { project in
                    if project.adapterType == "xcode" {
                        return ("Xcode", Theme.xcode)
                    } else {
                        return ("Node", Theme.node)
                    }
                }
            )
            .frame(width: 240)

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
                workingDirectory: runner.selectedWorkspace?.path ?? FileManager.default.currentDirectoryPath,
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
