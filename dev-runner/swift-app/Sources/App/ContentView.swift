import SwiftUI

struct ContentView: View {
    @EnvironmentObject var runner: DevRunner
    @State private var isBuilding = false
    @State private var errorMessage: String?
    @State private var terminalController: SimpleTerminalController?

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
                    ForEach(runner.workspaces) { workspace in
                        SciFiSidebarItem(
                            name: workspace.name,
                            isSelected: runner.selectedWorkspace?.id == workspace.id,
                            onSelect: { runner.selectWorkspace(workspace) },
                            onRemove: { runner.removeWorkspace(workspace) }
                        )
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
        SimpleTerminalView(
            workingDirectory: runner.selectedWorkspace?.path ?? FileManager.default.currentDirectoryPath
        ) { controller in
            print("[ContentView] terminalArea: onReady called")
            DispatchQueue.main.async {
                self.terminalController = controller
                print("[ContentView] terminalArea: controller set")
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
