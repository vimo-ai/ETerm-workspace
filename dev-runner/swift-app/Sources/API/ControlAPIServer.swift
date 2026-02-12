//
//  ControlAPIServer.swift
//  DevRunner
//
//  Control API Server - MCP 遥控器的后端
//  提供 HTTP API 让 MCP Server 控制 dev-runner
//

import Foundation
import SwiftUI

/// API 响应模型
struct HealthResponse: Codable {
    let status: String
    let version: String
    let uptimeSecs: Int

    enum CodingKeys: String, CodingKey {
        case status, version
        case uptimeSecs = "uptime_secs"
    }
}

struct ProjectListResponse: Codable {
    let projects: [ProjectResponse]
}

struct ProjectResponse: Codable {
    let path: String
    let name: String
    let type: String
    let status: String  // running | stopped | crashed
    let pid: Int32?
    let uptimeSecs: Int?

    enum CodingKeys: String, CodingKey {
        case path, name, type, status, pid
        case uptimeSecs = "uptime_secs"
    }
}

struct AddProjectRequest: Codable {
    let path: String
}

struct AddProjectResponse: Codable {
    let path: String
    let name: String
    let type: String
    let targets: [String]
}

struct BuildRequest: Codable {
    let path: String
    let target: String?
    let device: String?
    let config: String?
    let clean: Bool?
}

struct RunRequest: Codable {
    let path: String
    let target: String?
    let device: String?
}

struct StartRequest: Codable {
    let path: String
    let target: String?
    let device: String?
    let config: String?
    let clean: Bool?
}

struct StartResponse: Codable {
    let success: Bool
    let pid: Int32?
    let message: String
    let alreadyRunning: Bool?

    enum CodingKeys: String, CodingKey {
        case success, pid, message
        case alreadyRunning = "already_running"
    }
}

struct StopRequest: Codable {
    let path: String
    let force: Bool?
    let action: String? // "build" | "run"，不指定则停最活跃的
}

struct StopResponse: Codable {
    let success: Bool
    let exitCode: Int?

    enum CodingKeys: String, CodingKey {
        case success
        case exitCode = "exit_code"
    }
}

struct StatusResponse: Codable {
    let status: String  // running | stopped | crashed
    let action: String? // "build" | "run" — 当前查到的 tab 类型
    let pid: Int32?
    let uptimeSecs: Int?
    let exitCode: Int?

    enum CodingKeys: String, CodingKey {
        case status, action, pid
        case uptimeSecs = "uptime_secs"
        case exitCode = "exit_code"
    }
}

struct DeviceListResponse: Codable {
    let devices: [DeviceResponse]
}

struct DeviceResponse: Codable {
    let id: String
    let name: String
    let type: String
    let osVersion: String?
    let state: String

    enum CodingKeys: String, CodingKey {
        case id, name, type
        case osVersion = "os_version"
        case state
    }
}

struct BootDeviceRequest: Codable {
    let id: String
}

struct SuccessResponse: Codable {
    let success: Bool
}

/// 终端日志响应（LogBuffer 分页查询）
struct TerminalLogsResponse: Codable {
    let lines: [LogLineResponse]
    let nextSeq: UInt64
    let hasMore: Bool
    let truncated: Bool

    enum CodingKeys: String, CodingKey {
        case lines
        case nextSeq = "next_seq"
        case hasMore = "has_more"
        case truncated
    }
}

struct LogLineResponse: Codable {
    let seq: UInt64
    let text: String
}

/// Control API Server
///
/// 管理与 DevRunner 和 TerminalTabManager 的集成
@MainActor
final class ControlAPIServer {

    // MARK: - Properties

    private let server: HTTPServer
    private let port: UInt16

    private weak var runner: DevRunner?
    private weak var tabManager: TerminalTabManager?
    private weak var terminalPool: SimpleTerminalPoolWrapper?

    /// 启动时间
    private let startTime = Date()

    /// 版本号
    private let version = "0.1.0"

    // MARK: - Initialization

    init(port: UInt16 = 9274) {
        self.port = port
        self.server = HTTPServer(port: port)
    }

    /// 设置依赖
    func configure(runner: DevRunner, tabManager: TerminalTabManager, terminalPool: SimpleTerminalPoolWrapper) {
        self.runner = runner
        self.tabManager = tabManager
        self.terminalPool = terminalPool

        setupRoutes()
    }

    // MARK: - Lifecycle

    func start() {
        do {
            try server.start()
            print("[ControlAPI] Started on port \(port)")
        } catch {
            print("[ControlAPI] Failed to start: \(error)")
        }
    }

    func stop() {
        server.stop()
        print("[ControlAPI] Stopped")
    }

    // MARK: - Route Setup

    private func setupRoutes() {
        // Health check
        server.get("/health") { [weak self] _ in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run {
                let uptime = Int(Date().timeIntervalSince(self.startTime))
                return .json(HealthResponse(status: "ok", version: self.version, uptimeSecs: uptime))
            }
        }

        server.get("/api/v1/health") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run {
                let uptime = Int(Date().timeIntervalSince(self.startTime))
                return .json(HealthResponse(status: "ok", version: self.version, uptimeSecs: uptime))
            }
        }

        // List projects
        server.get("/api/v1/projects") { [weak self] _ in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleListProjects() }
        }

        // Add project
        server.post("/api/v1/projects") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleAddProject(req) }
        }

        // Remove project
        server.delete("/api/v1/projects") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleRemoveProject(req) }
        }

        // Build project (compile only, auto-installs on simulator)
        server.post("/api/v1/projects/build") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleBuild(req) }
        }

        // Run project (launch only, assumes already built)
        server.post("/api/v1/projects/run") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleRun(req) }
        }

        // Start project (build + run combined)
        server.post("/api/v1/projects/start") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleStart(req) }
        }

        // Stop project
        server.post("/api/v1/projects/stop") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleStop(req) }
        }

        // Get status
        server.get("/api/v1/projects/status") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleStatus(req) }
        }

        // Get logs
        server.get("/api/v1/projects/logs") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleLogs(req) }
        }

        // List devices
        server.get("/api/v1/devices") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleListDevices(req) }
        }

        // Boot simulator
        server.post("/api/v1/devices/boot") { [weak self] req in
            guard let self = self else { return .error(500, "Server not available") }
            return await MainActor.run { self.handleBootDevice(req) }
        }
    }

    // MARK: - Route Handlers

    private func handleListProjects() -> HTTPResponse {
        guard let runner = runner else {
            return .error(500, "Runner not available")
        }

        var projects: [ProjectResponse] = []

        for workspace in runner.workspaces {
            for project in workspace.projects {
                let status = getProjectStatus(project.path)
                projects.append(ProjectResponse(
                    path: project.path,
                    name: project.name,
                    type: project.adapterType,
                    status: status.status,
                    pid: status.pid,
                    uptimeSecs: status.uptimeSecs
                ))
            }
        }

        return .json(ProjectListResponse(projects: projects))
    }

    private func handleAddProject(_ req: HTTPRequest) -> HTTPResponse {
        guard let runner = runner else {
            return .error(500, "Runner not available")
        }

        guard let body: AddProjectRequest = req.jsonBody(AddProjectRequest.self) else {
            return .error(400, "Invalid request body")
        }

        let path = normalizePath(body.path)

        // 检查路径是否存在
        guard FileManager.default.fileExists(atPath: path) else {
            return .error(400, "Path does not exist: \(path)")
        }

        do {
            // 检测并添加
            try runner.addWorkspace(path: path)

            // 获取 targets
            var targets: [String] = []
            if let workspace = runner.workspaces.first(where: { $0.path == path }),
               let project = workspace.projects.first {
                let targetInfos = try? runner.listTargets(projectPath: project.path)
                targets = targetInfos?.map(\.name) ?? []
            }

            return .json(AddProjectResponse(
                path: path,
                name: URL(fileURLWithPath: path).lastPathComponent,
                type: runner.selectedProject?.adapterType ?? "unknown",
                targets: targets
            ))
        } catch {
            return .error(500, "Failed to add project: \(error.localizedDescription)")
        }
    }

    private func handleRemoveProject(_ req: HTTPRequest) -> HTTPResponse {
        guard let runner = runner else {
            return .error(500, "Runner not available")
        }

        guard let path = req.queryParam("path") else {
            return .error(400, "Missing path parameter")
        }

        let normalized = normalizePath(path)

        if let workspace = runner.workspaces.first(where: { $0.path == normalized }) {
            runner.removeWorkspace(workspace)
            return .json(SuccessResponse(success: true))
        } else {
            return .error(404, "Project not found")
        }
    }

    private func handleBuild(_ req: HTTPRequest) -> HTTPResponse {
        guard let runner = runner,
              let tabManager = tabManager,
              let pool = terminalPool else {
            return .error(500, "Server not configured")
        }

        guard let body: BuildRequest = req.jsonBody(BuildRequest.self) else {
            return .error(400, "Invalid request body")
        }

        let path = normalizePath(body.path)

        guard let (_, project) = findProject(path: path) else {
            return .error(404, "Project not found: \(path)")
        }

        guard let context = runner.resolveProjectContext(
            projectPath: project.path,
            targetName: body.target,
            deviceName: body.device
        ) else {
            return .error(400, "No targets found for project: \(project.name)")
        }

        let taskKey = TaskKey(
            projectPath: project.path,
            action: .build,
            deviceId: context.device?.deviceId,
            deviceName: context.device?.name
        )

        if let existingTab = tabManager.findTab(for: taskKey), existingTab.taskState.isActive {
            return .json(StartResponse(
                success: true,
                pid: nil,
                message: "\(project.name) is already building",
                alreadyRunning: true
            ))
        }

        guard let (tab, _, _) = tabManager.findOrCreateTaskTab(cwd: project.path, taskKey: taskKey) else {
            return .error(500, "Failed to create terminal")
        }

        do {
            let buildOpts = BuildOptions(
                config: body.config ?? "Debug",
                clean: body.clean ?? false,
                deviceId: context.device?.deviceId
            )
            let buildCmd = try runner.buildCommand(
                projectPath: project.path,
                target: context.target.name,
                options: buildOpts
            )

            pool.sendCommand(buildCmd.display, to: tab.terminalId)
            tabManager.markTaskSent(tab.id, command: buildCmd.display)

            return .json(StartResponse(
                success: true,
                pid: nil,
                message: "\(project.name) building for \(context.device?.name ?? "default device")",
                alreadyRunning: false
            ))
        } catch {
            return .error(500, "Failed to generate command: \(error.localizedDescription)")
        }
    }

    private func handleRun(_ req: HTTPRequest) -> HTTPResponse {
        guard let runner = runner,
              let tabManager = tabManager,
              let pool = terminalPool else {
            return .error(500, "Server not configured")
        }

        guard let body: RunRequest = req.jsonBody(RunRequest.self) else {
            return .error(400, "Invalid request body")
        }

        let path = normalizePath(body.path)

        guard let (_, project) = findProject(path: path) else {
            return .error(404, "Project not found: \(path)")
        }

        guard let context = runner.resolveProjectContext(
            projectPath: project.path,
            targetName: body.target,
            deviceName: body.device
        ) else {
            return .error(400, "No targets found for project: \(project.name)")
        }

        let taskKey = TaskKey(
            projectPath: project.path,
            action: .run,
            deviceId: context.device?.deviceId,
            deviceName: context.device?.name
        )

        if let existingTab = tabManager.findTab(for: taskKey), existingTab.taskState.isActive {
            let info = ProcessMonitor().info(for: existingTab.terminalId)
            return .json(StartResponse(
                success: true,
                pid: info.pid,
                message: "\(project.name) is already running",
                alreadyRunning: true
            ))
        }

        guard let (tab, _, _) = tabManager.findOrCreateTaskTab(cwd: project.path, taskKey: taskKey) else {
            return .error(500, "Failed to create terminal")
        }

        do {
            let runOpts = RunOptions(deviceId: context.device?.deviceId)

            // Simulator 需要先 install 再 terminate 再 launch
            var fullCommand: String
            let runCmd = try runner.runCommand(
                projectPath: project.path,
                target: context.target.name,
                options: runOpts
            )
            if let installCmd = try? runner.installCommand(
                projectPath: project.path,
                target: context.target.name,
                options: runOpts
            ) {
                fullCommand = installCmd.display
            } else {
                fullCommand = ""
            }
            // Simulator: terminate 旧实例，确保 --console-pty 能建立 PTY 连接
            if let deviceId = context.device?.deviceId,
               let bundleId = project.bundleId,
               context.device?.isSimulator == true {
                let terminateCmd = "(xcrun simctl terminate \(deviceId) \(bundleId) 2>/dev/null || true)"
                if fullCommand.isEmpty {
                    fullCommand = terminateCmd
                } else {
                    fullCommand += " && \(terminateCmd)"
                }
            }
            if fullCommand.isEmpty {
                fullCommand = runCmd.display
            } else {
                fullCommand += " && \(runCmd.display)"
            }

            pool.sendCommand(fullCommand, to: tab.terminalId)
            tabManager.markTaskSent(tab.id, command: fullCommand)

            return .json(StartResponse(
                success: true,
                pid: nil,
                message: "\(project.name) launching on \(context.device?.name ?? "default device")",
                alreadyRunning: false
            ))
        } catch {
            return .error(500, "Failed to generate command: \(error.localizedDescription)")
        }
    }

    private func handleStart(_ req: HTTPRequest) -> HTTPResponse {
        guard let runner = runner,
              let tabManager = tabManager,
              let pool = terminalPool else {
            return .error(500, "Server not configured")
        }

        guard let body: StartRequest = req.jsonBody(StartRequest.self) else {
            return .error(400, "Invalid request body")
        }

        let path = normalizePath(body.path)

        // 查找项目（只读，不修改全局状态）
        guard let (_, project) = findProject(path: path) else {
            return .error(404, "Project not found: \(path)")
        }

        // 无状态解析：open 项目、加载 targets/devices、自动选择
        guard let context = runner.resolveProjectContext(
            projectPath: project.path,
            targetName: body.target,
            deviceName: body.device
        ) else {
            return .error(400, "No targets found for project: \(project.name)")
        }

        // 检查是否已运行
        let taskKey = TaskKey(
            projectPath: project.path,
            action: .run,
            deviceId: context.device?.deviceId,
            deviceName: context.device?.name
        )

        if let existingTab = tabManager.findTab(for: taskKey), existingTab.taskState.isActive {
            let info = ProcessMonitor().info(for: existingTab.terminalId)
            return .json(StartResponse(
                success: true,
                pid: info.pid,
                message: "\(project.name) is already running",
                alreadyRunning: true
            ))
        }

        // 创建或获取 Tab
        guard let (tab, _, _) = tabManager.findOrCreateTaskTab(cwd: project.path, taskKey: taskKey) else {
            return .error(500, "Failed to create terminal")
        }

        // 生成命令（无状态，直接传参）
        do {
            let buildOpts = BuildOptions(
                config: body.config ?? "Debug",
                clean: body.clean ?? false,
                deviceId: context.device?.deviceId
            )
            let buildCmd = try runner.buildCommand(
                projectPath: project.path,
                target: context.target.name,
                options: buildOpts
            )

            let runOpts = RunOptions(deviceId: context.device?.deviceId)

            // 组合命令：build && install(如有) && terminate(如有) && run
            var fullCommand = buildCmd.display
            if let installCmd = try? runner.installCommand(
                projectPath: project.path,
                target: context.target.name,
                options: runOpts
            ) {
                fullCommand += " && \(installCmd.display)"
            }
            // Simulator: terminate 旧实例，确保 --console-pty 能建立 PTY 连接
            if let deviceId = context.device?.deviceId,
               let bundleId = project.bundleId,
               context.device?.isSimulator == true {
                fullCommand += " && (xcrun simctl terminate \(deviceId) \(bundleId) 2>/dev/null || true)"
            }
            let runCmd = try runner.runCommand(
                projectPath: project.path,
                target: context.target.name,
                options: runOpts
            )
            fullCommand += " && \(runCmd.display)"

            // 发送到终端
            pool.sendCommand(fullCommand, to: tab.terminalId)
            tabManager.markTaskSent(tab.id, command: fullCommand)

            return .json(StartResponse(
                success: true,
                pid: nil,
                message: "\(project.name) starting on \(context.device?.name ?? "default device")",
                alreadyRunning: false
            ))
        } catch {
            return .error(500, "Failed to generate command: \(error.localizedDescription)")
        }
    }

    private func handleStop(_ req: HTTPRequest) -> HTTPResponse {
        guard let tabManager = tabManager,
              let pool = terminalPool else {
            return .error(500, "Server not configured")
        }

        guard let body: StopRequest = req.jsonBody(StopRequest.self) else {
            return .error(400, "Invalid request body")
        }

        let path = normalizePath(body.path)
        let actionFilter: TaskAction? = {
            switch body.action {
            case "build": return .build
            case "run": return .run
            default: return nil
            }
        }()

        // 查找对应的 Tab（支持 action 过滤，不指定则匹配 build/run）
        let matchingTabs = tabManager.tabs.filter { tab in
            guard tab.taskKey?.projectPath == path else { return false }
            if let action = actionFilter {
                return tab.taskKey?.action == action
            }
            return tab.taskKey?.action == .build || tab.taskKey?.action == .run
        }
        guard let tab = matchingTabs.first(where: {
            if case .running = $0.taskState { return true }; return false
        }) ?? matchingTabs.first(where: {
            if case .sent = $0.taskState { return true }; return false
        }) ?? matchingTabs.first else {
            return .json(StopResponse(success: true, exitCode: 0))  // 已经停止
        }

        // 发送中断
        if body.force == true {
            // SIGKILL: 发送多次 Ctrl+C
            for _ in 0..<3 {
                pool.sendInterrupt(to: tab.terminalId)
            }
        } else {
            // SIGINT
            pool.sendInterrupt(to: tab.terminalId)
        }

        return .json(StopResponse(success: true, exitCode: nil))
    }

    private func handleStatus(_ req: HTTPRequest) -> HTTPResponse {
        guard let path = req.queryParam("path") else {
            return .error(400, "Missing path parameter")
        }

        let action: TaskAction? = {
            switch req.queryParam("action") {
            case "build": return .build
            case "run": return .run
            default: return nil
            }
        }()

        let status = getProjectStatus(normalizePath(path), action: action)
        return .json(status)
    }

    private func handleLogs(_ req: HTTPRequest) -> HTTPResponse {
        guard let path = req.queryParam("path"),
              let tabManager = tabManager,
              let pool = terminalPool else {
            return .error(400, "Missing path parameter or server not configured")
        }

        let normalized = normalizePath(path)
        let actionFilter: TaskAction? = {
            switch req.queryParam("action") {
            case "build": return .build
            case "run": return .run
            default: return nil
            }
        }()

        // 查找项目对应的终端 Tab（支持 action 过滤，优先最活跃）
        let matchingTabs = tabManager.tabs.filter { tab in
            guard tab.taskKey?.projectPath == normalized else { return false }
            if let action = actionFilter {
                return tab.taskKey?.action == action
            }
            return tab.taskKey?.action == .build || tab.taskKey?.action == .run
        }
        guard let tab = matchingTabs.first(where: {
            if case .running = $0.taskState { return true }; return false
        }) ?? matchingTabs.first(where: {
            if case .sent = $0.taskState { return true }; return false
        }) ?? matchingTabs.first else {
            // 没有运行的终端，返回空日志
            return .json(TerminalLogsResponse(lines: [], nextSeq: 0, hasMore: false, truncated: false))
        }

        // 解析查询参数
        let since = UInt64(req.queryParam("since") ?? req.queryParam("after") ?? "0") ?? 0
        let before = UInt64(req.queryParam("before") ?? "0") ?? 0
        let limit = Int(req.queryParam("limit") ?? "200") ?? 200
        let search = req.queryParam("search")
        let isRegex = req.queryParam("regex") == "true"
        let caseInsensitive = req.queryParam("case_insensitive") != "false" // 默认 true，向后兼容
        let backward = req.queryParam("direction") == "backward"
        let currentRun = req.queryParam("current_run") == "true"

        // 使用 LogBuffer 查询（Rust 层，已剥离 ANSI、处理 \r）
        if let json = pool.queryLog(tab.terminalId, since: since, before: before, limit: limit, search: search, isRegex: isRegex, caseInsensitive: caseInsensitive, backward: backward, currentRun: currentRun) {
            // Rust 返回的 JSON 已是完整格式，直接透传
            return .rawJSON(json)
        }

        // LogBuffer 未启用，回退到可见行
        let visibleLines = pool.getVisibleLines(tab.terminalId)
        let lines = visibleLines.enumerated().map { LogLineResponse(seq: UInt64($0.offset), text: $0.element) }
        return .json(TerminalLogsResponse(lines: lines, nextSeq: UInt64(visibleLines.count), hasMore: false, truncated: false))
    }

    private func handleListDevices(_ req: HTTPRequest) -> HTTPResponse {
        guard let runner = runner else {
            return .error(500, "Runner not available")
        }

        let devices = runner.devices.map { device in
            DeviceResponse(
                id: device.deviceId,
                name: device.name,
                type: device.deviceType,
                osVersion: device.osVersion,
                state: device.state
            )
        }

        return .json(DeviceListResponse(devices: devices))
    }

    private func handleBootDevice(_ req: HTTPRequest) -> HTTPResponse {
        guard let body: BootDeviceRequest = req.jsonBody(BootDeviceRequest.self) else {
            return .error(400, "Invalid request body")
        }

        // 使用 xcrun simctl boot
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/xcrun")
        process.arguments = ["simctl", "boot", body.id]

        do {
            try process.run()
            process.waitUntilExit()

            if process.terminationStatus == 0 {
                return .json(SuccessResponse(success: true))
            } else {
                return .error(500, "Failed to boot simulator")
            }
        } catch {
            return .error(500, "Failed to boot simulator: \(error.localizedDescription)")
        }
    }

    // MARK: - Helpers

    private func normalizePath(_ path: String) -> String {
        // 展开 ~
        let expanded = NSString(string: path).expandingTildeInPath
        // realpath 规范化
        if let realPath = (expanded as NSString).resolvingSymlinksInPath as String? {
            return realPath
        }
        return expanded
    }

    private func findProject(path: String) -> (Workspace, ProjectInfo)? {
        guard let runner = runner else { return nil }

        for workspace in runner.workspaces {
            if workspace.path == path {
                if let project = workspace.projects.first {
                    return (workspace, project)
                }
            }
            for project in workspace.projects {
                if project.path == path {
                    return (workspace, project)
                }
            }
        }

        return nil
    }

    private func getProjectStatus(_ path: String, action: TaskAction? = nil) -> StatusResponse {
        guard let tabManager = tabManager else {
            return StatusResponse(status: "stopped", action: nil, pid: nil, uptimeSecs: nil, exitCode: nil)
        }

        // 查找匹配的 Tab（可选 action 过滤）
        let matchingTabs = tabManager.tabs.filter { tab in
            guard tab.taskKey?.projectPath == path else { return false }
            if let action = action {
                return tab.taskKey?.action == action
            }
            // 无 action 过滤时，匹配 build 和 run
            return tab.taskKey?.action == .build || tab.taskKey?.action == .run
        }

        // 优先返回最活跃的 tab: running > sent > completed > idle
        let tab = matchingTabs.first(where: {
            if case .running = $0.taskState { return true }; return false
        }) ?? matchingTabs.first(where: {
            if case .sent = $0.taskState { return true }; return false
        }) ?? matchingTabs.first(where: {
            if case .completed = $0.taskState { return true }; return false
        }) ?? matchingTabs.first

        guard let tab = tab else {
            return StatusResponse(status: "not_found", action: nil, pid: nil, uptimeSecs: nil, exitCode: nil)
        }

        let actionStr = tab.taskKey?.action == .build ? "build" : "run"

        switch tab.taskState {
        case .idle:
            return StatusResponse(status: "idle", action: actionStr, pid: nil, uptimeSecs: nil, exitCode: nil)
        case .sent:
            return StatusResponse(status: "starting", action: actionStr, pid: nil, uptimeSecs: nil, exitCode: nil)
        case .running:
            return StatusResponse(
                status: "running",
                action: actionStr,
                pid: tab.pid,
                uptimeSecs: tab.duration.map { Int($0) },
                exitCode: nil
            )
        case .completed(let exitCode):
            return StatusResponse(
                status: exitCode == 0 ? "success" : "failed",
                action: actionStr,
                pid: nil,
                uptimeSecs: tab.duration.map { Int($0) },
                exitCode: Int(exitCode)
            )
        }
    }
}
