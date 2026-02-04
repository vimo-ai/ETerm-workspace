import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @EnvironmentObject var runner: DevRunner
    @State private var isBuilding = false
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
            // Status
            Circle()
                .fill(tab.isRunning ? Theme.success : Theme.textMuted.opacity(0.5))
                .frame(width: 6, height: 6)

            // Title
            Text(tab.title)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .foregroundColor(Theme.textPrimary)

            Spacer()

            // Quick actions
            if tab.isRunning {
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

        // 临时设置 target/device（用于命令生成）
        if let target = target {
            runner.selectedTarget = target
        }
        if let device = device {
            runner.selectedDevice = device
        }

        switch action {
        case .shell:
            // 创建普通 shell tab
            tabManager.createTab(cwd: project.path, title: "zsh", taskKey: nil)

        case .build:
            let taskKey = TaskKey(
                projectPath: project.path,
                action: .build,
                deviceId: device?.deviceId,
                deviceName: device?.name
            )
            executeTask(taskKey: taskKey) {
                let options = BuildOptions(
                    config: "Debug",
                    deviceId: device?.deviceId
                )
                return try runner.buildCommand(options: options)
            }

        case .run:
            let taskKey = TaskKey(
                projectPath: project.path,
                action: .run,
                deviceId: device?.deviceId,
                deviceName: device?.name
            )
            executeTask(taskKey: taskKey) {
                let options = RunOptions(deviceId: device?.deviceId)
                return try runner.runCommand(options: options)
            }
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

                // 发送命令到对应 tab 的终端
                if let cwd = cmd.cwd {
                    terminalController?.sendCommand("cd '\(cwd)' && \(fullCommand)", to: tab.terminalId)
                } else {
                    terminalController?.sendCommand(fullCommand, to: tab.terminalId)
                }
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }
}

#Preview {
    ContentView()
}
