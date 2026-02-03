import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject var runner: DevRunner
    @State private var isBuilding = false
    @State private var errorMessage: String?
    @State private var terminalController: MultiTerminalController?
    @StateObject private var tabManager = TerminalTabManager()
    @State private var isDraggingOver = false

    var body: some View {
        HSplitView {
            SidebarView(
                isDraggingOver: $isDraggingOver,
                onAddWorkspace: addWorkspace,
                onDrop: handleDrop
            )
            .frame(minWidth: 160, idealWidth: 220, maxWidth: 400)

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
        guard let project = runner.selectedProject,
              runner.selectedTarget != nil else {
            print("[ContentView] build: no project/target selected")
            return
        }

        let taskKey = TaskKey(
            projectPath: project.path,
            action: .build,
            deviceId: runner.selectedDevice?.deviceId,
            deviceName: runner.selectedDevice?.name
        )

        executeTask(taskKey: taskKey) {
            let options = BuildOptions(
                config: "Debug",
                deviceId: runner.selectedDevice?.deviceId
            )
            return try runner.buildCommand(options: options)
        }
    }

    private func run() {
        guard let project = runner.selectedProject,
              runner.selectedTarget != nil else {
            return
        }

        let taskKey = TaskKey(
            projectPath: project.path,
            action: .run,
            deviceId: runner.selectedDevice?.deviceId,
            deviceName: runner.selectedDevice?.name
        )

        executeTask(taskKey: taskKey) {
            let options = RunOptions(deviceId: runner.selectedDevice?.deviceId)
            return try runner.runCommand(options: options)
        }
    }

    /// 执行任务（查找/创建 Tab，处理运行中状态）
    private func executeTask(taskKey: TaskKey, commandBuilder: @escaping () throws -> CommandInfo) {
        guard let project = runner.selectedProject else { return }

        isBuilding = true

        Task {
            defer { isBuilding = false }

            // 查找或创建对应的 Tab
            guard let result = tabManager.findOrCreateTaskTab(
                cwd: project.path,
                taskKey: taskKey
            ) else {
                errorMessage = "Failed to create terminal tab"
                return
            }

            let (tab, _, wasRunning) = result

            // 如果正在运行，先停止
            if wasRunning {
                print("[ContentView] executeTask: stopping existing process in tab \(tab.title)")
                terminalController?.sendInterrupt(to: tab.terminalId)
                // 等待进程停止
                try? await Task.sleep(nanoseconds: 500_000_000)  // 0.5s
            }

            // 生成并发送命令
            do {
                let cmd = try commandBuilder()

                var fullCommand = cmd.program
                if !cmd.args.isEmpty {
                    fullCommand += " " + cmd.args.joined(separator: " ")
                }

                print("[ContentView] executeTask: \(taskKey.displayName) -> \(fullCommand)")

                // 发送命令到对应 tab 的终端
                if let cwd = cmd.cwd {
                    terminalController?.sendCommand("cd '\(cwd)' && \(fullCommand)", to: tab.terminalId)
                } else {
                    terminalController?.sendCommand(fullCommand, to: tab.terminalId)
                }
            } catch {
                print("[ContentView] executeTask: error=\(error)")
                errorMessage = error.localizedDescription
            }
        }
    }

    private func stop() {
        // 停止当前选中 tab 的进程
        if let tab = tabManager.selectedTab {
            terminalController?.sendInterrupt(to: tab.terminalId)
        }
    }

    private func clear() {
        terminalController?.clear()
    }
}

#Preview {
    ContentView()
}
