import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject var runner: DevRunner
    @State private var errorMessage: String?
    @State private var terminalController: MultiTerminalController?
    @StateObject private var tabManager = TerminalTabManager()
    @State private var isDraggingOver = false
    @State private var showNewTaskSheet = false

    /// Control API Server (MCP 遥控器后端)
    @State private var apiServer: ControlAPIServer?

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
        .sheet(isPresented: $showNewTaskSheet) {
            NewTaskSheet(isPresented: $showNewTaskSheet) { action, target, device in
                startTask(action: action, target: target, device: device)
            }
            .environmentObject(runner)
        }
        .onReceive(NotificationCenter.default.publisher(for: .stopTask)) { notification in
            if let terminalId = notification.userInfo?["terminalId"] as? Int {
                terminalController?.sendInterrupt(to: terminalId)
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .restartTask)) { notification in
            if let tabIdStr = notification.userInfo?["tabId"] as? String,
               let tabId = UUID(uuidString: tabIdStr),
               let tab = tabManager.tabs.first(where: { $0.id == tabId }),
               let command = tab.commandString {
                restartTask(tab: tab, command: command)
            }
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
        VSplitView {
            // 任务列表（上部）
            TaskListView(tabManager: tabManager, onNewTask: {
                showNewTaskSheet = true
            })
            .frame(minHeight: 120, idealHeight: 180, maxHeight: 300)

            // 终端输出（下部）
            terminalArea
        }
        .background(Theme.bgPrimary)
    }

    private var terminalArea: some View {
        VStack(spacing: 0) {
            // Tab Bar (mini version - just shows selected)
            if let selectedTab = tabManager.selectedTab {
                miniTabBar(selectedTab)

                Rectangle()
                    .fill(Theme.border)
                    .frame(height: 1)
            }

            // Terminal View
            MultiTerminalView(
                workingDirectory: runner.selectedProject?.path ?? FileManager.default.currentDirectoryPath,
                tabManager: tabManager
            ) { controller in
                DispatchQueue.main.async {
                    self.terminalController = controller
                    // 初始化 Control API Server
                    setupAPIServer(controller: controller)
                }
            }
        }
    }

    private func miniTabBar(_ tab: TerminalTab) -> some View {
        HStack(spacing: 8) {
            // Status indicator
            miniStatusIndicator(tab.taskState)

            // Title
            Text(tab.title)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .foregroundColor(Theme.textPrimary)

            // Duration (for active/completed tasks)
            if let durationText = tab.durationText {
                Text(durationText)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(Theme.textMuted)
            }

            Spacer()

            // Quick actions
            if tab.taskState.isActive {
                Button {
                    terminalController?.sendInterrupt(to: tab.terminalId)
                } label: {
                    Image(systemName: "stop.fill")
                        .font(.system(size: 9))
                        .foregroundColor(Theme.error)
                }
                .buttonStyle(.plain)
                .help("Stop (Ctrl+C)")
            }

            Button {
                terminalController?.clear()
            } label: {
                Image(systemName: "trash")
                    .font(.system(size: 9))
                    .foregroundColor(Theme.textSecondary)
            }
            .buttonStyle(.plain)
            .help("Clear terminal")

            // Add new shell tab
            Button {
                if let project = runner.selectedProject {
                    tabManager.createTab(cwd: project.path, title: "zsh", taskKey: nil)
                }
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 9))
                    .foregroundColor(Theme.textSecondary)
            }
            .buttonStyle(.plain)
            .help("New shell tab")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Theme.bgSecondary)
    }

    @ViewBuilder
    private func miniStatusIndicator(_ state: TaskState) -> some View {
        switch state {
        case .idle:
            Circle()
                .fill(Theme.textMuted.opacity(0.5))
                .frame(width: 6, height: 6)
        case .sent:
            Circle()
                .fill(Theme.warning)
                .frame(width: 6, height: 6)
        case .running:
            Circle()
                .fill(Theme.success)
                .frame(width: 6, height: 6)
        case .completed(let exitCode):
            if exitCode == 0 {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 9))
                    .foregroundColor(Theme.success)
            } else {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 9))
                    .foregroundColor(Theme.error)
            }
        }
    }

    // MARK: - API Server

    private func setupAPIServer(controller: MultiTerminalController) {
        guard apiServer == nil, let pool = controller.terminalPool else { return }

        let server = ControlAPIServer(port: 9274)
        server.configure(runner: runner, tabManager: tabManager, terminalPool: pool)
        server.start()
        apiServer = server

        print("[ContentView] Control API Server started on port 9274")
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
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    private func handleDrop(providers: [NSItemProvider]) -> Bool {
        guard let provider = providers.first else { return false }

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

                var isDirectory: ObjCBool = false
                guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory),
                      isDirectory.boolValue else {
                    self.errorMessage = "Please drop a folder, not a file"
                    return
                }

                do {
                    try self.runner.addWorkspace(path: url.path)
                } catch {
                    self.errorMessage = error.localizedDescription
                }
            }
        }

        return true
    }

    /// 启动任务（从 NewTaskSheet 调用）
    private func startTask(action: TaskAction, target: TargetInfo?, device: DeviceInfo?) {
        guard let project = runner.selectedProject else { return }

        // 同步更新 UI state（侧边栏选中等）
        if let target = target {
            runner.selectedTarget = target
        }
        if let device = device {
            runner.selectedDevice = device
        }

        // 捕获值，避免闭包中读取可变的全局状态
        let projectPath = project.path
        let targetName = target?.name ?? runner.selectedTarget?.name ?? ""
        let deviceId = device?.deviceId
        let deviceName = device?.name

        switch action {
        case .shell:
            tabManager.createTab(cwd: projectPath, title: "zsh", taskKey: nil)

        case .build:
            let taskKey = TaskKey(
                projectPath: projectPath,
                action: .build,
                deviceId: deviceId,
                deviceName: deviceName
            )
            executeTask(projectPath: projectPath, taskKey: taskKey) {
                let options = BuildOptions(
                    config: "Debug",
                    deviceId: deviceId
                )
                return try runner.buildCommand(projectPath: projectPath, target: targetName, options: options)
            }

        case .run:
            let taskKey = TaskKey(
                projectPath: projectPath,
                action: .run,
                deviceId: deviceId,
                deviceName: deviceName
            )
            executeTask(projectPath: projectPath, taskKey: taskKey) {
                let options = RunOptions(deviceId: deviceId)
                return try runner.runCommand(projectPath: projectPath, target: targetName, options: options)
            }
        }
    }

    /// 执行任务（查找/创建 Tab，处理运行中状态，标记状态机）
    private func executeTask(projectPath: String, taskKey: TaskKey, commandBuilder: @escaping () throws -> CommandInfo) {
        // 查找或创建对应的 Tab
        guard let result = tabManager.findOrCreateTaskTab(
            cwd: projectPath,
            taskKey: taskKey
        ) else {
            errorMessage = "Failed to create terminal tab"
            return
        }

        let (tab, _, wasRunning) = result

        Task {
            // 如果正在运行，先停止
            if wasRunning {
                terminalController?.sendInterrupt(to: tab.terminalId)
                try? await Task.sleep(nanoseconds: 500_000_000)  // 0.5s
            }

            // 生成并发送命令
            do {
                let cmd = try commandBuilder()

                var fullCommand = cmd.program
                if !cmd.args.isEmpty {
                    fullCommand += " " + cmd.args.joined(separator: " ")
                }

                let baseCommand: String
                if let cwd = cmd.cwd {
                    baseCommand = "cd '\(cwd)' && \(fullCommand)"
                } else {
                    baseCommand = fullCommand
                }

                // 包装命令：追加退出码标记（用于 LogBuffer 解析）
                let shellCommand = TaskExitMarker.wrap(baseCommand)

                // 标记状态：sent
                tabManager.markTaskSent(tab.id, command: baseCommand)

                // 发送命令到终端
                terminalController?.sendCommand(shellCommand, to: tab.terminalId)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    /// 重跑任务（从 restartTask 通知触发）
    private func restartTask(tab: TerminalTab, command: String) {
        Task {
            // 如果还在运行，先停止
            if tab.taskState.isActive {
                terminalController?.sendInterrupt(to: tab.terminalId)
                try? await Task.sleep(nanoseconds: 500_000_000)
            }

            // 标记状态：sent（保存原始命令，不含包装）
            tabManager.markTaskSent(tab.id, command: command)

            // 包装命令并发送
            let wrappedCommand = TaskExitMarker.wrap(command)
            terminalController?.sendCommand(wrappedCommand, to: tab.terminalId)
        }
    }
}

#Preview {
    ContentView()
}
