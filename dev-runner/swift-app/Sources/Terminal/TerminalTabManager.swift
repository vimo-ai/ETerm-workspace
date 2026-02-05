//
//  TerminalTabManager.swift
//  DevRunner
//
//  管理多个终端 Tab + 进程监控
//

import SwiftUI
import Combine

/// 任务类型
enum TaskAction: String, Equatable {
    case build
    case run
    case shell  // 普通 shell，无关联任务
}

/// 任务状态
enum TaskState: Equatable {
    case idle                         // shell 待命 / 任务未启动
    case sent                         // 命令已发出，进程尚未起来
    case running                      // 子进程在跑
    case completed(exitCode: Int32)   // 结束，带退出码

    var isRunning: Bool {
        if case .running = self { return true }
        return false
    }

    var isActive: Bool {
        switch self {
        case .sent, .running: return true
        default: return false
        }
    }

    var isSuccess: Bool {
        if case .completed(let code) = self { return code == 0 }
        return false
    }

    var isFailed: Bool {
        if case .completed(let code) = self { return code != 0 }
        return false
    }
}

/// 任务标识（用于查找已存在的 Tab）
struct TaskKey: Hashable {
    let projectPath: String
    let action: TaskAction
    let deviceId: String?
    let deviceName: String?

    // Hashable 只用 projectPath + action + deviceId
    func hash(into hasher: inout Hasher) {
        hasher.combine(projectPath)
        hasher.combine(action)
        hasher.combine(deviceId)
    }

    static func == (lhs: TaskKey, rhs: TaskKey) -> Bool {
        lhs.projectPath == rhs.projectPath &&
        lhs.action == rhs.action &&
        lhs.deviceId == rhs.deviceId
    }

    var displayName: String {
        let projectName = URL(fileURLWithPath: projectPath).lastPathComponent
        switch action {
        case .shell:
            return "zsh"
        case .build:
            return "\(projectName):Build"
        case .run:
            if let name = deviceName {
                return "\(projectName):\(name)"
            }
            return "\(projectName):Run"
        }
    }
}

/// 单个终端 Tab
struct TerminalTab: Identifiable, Equatable {
    let id: UUID
    let terminalId: Int          // TerminalPool 中的 ID
    var title: String            // 显示名称
    var taskState: TaskState = .idle  // 任务状态
    var ports: [UInt16] = []     // 监听端口
    var cpuPercent: Double = 0   // CPU 使用率 %
    var memoryMB: Double = 0     // 内存占用 MB
    var pid: Int32 = 0           // 进程 PID
    var processName: String = "" // 前台进程名
    var startedAt: Date?         // 任务开始时间
    var completedAt: Date?       // 任务结束时间
    var commandString: String?   // 上次执行的命令（用于 restart）

    // Task 关联
    var taskKey: TaskKey?        // 关联的任务（nil 表示普通 shell）

    /// 任务耗时（秒）
    var duration: TimeInterval? {
        guard let start = startedAt else { return nil }
        let end = completedAt ?? Date()
        return end.timeIntervalSince(start)
    }

    /// 格式化耗时
    var durationText: String? {
        guard let d = duration else { return nil }
        if d < 60 { return String(format: "%.0fs", d) }
        let min = Int(d) / 60
        let sec = Int(d) % 60
        return "\(min)m\(sec)s"
    }

    static func == (lhs: TerminalTab, rhs: TerminalTab) -> Bool {
        lhs.id == rhs.id && lhs.title == rhs.title &&
        lhs.taskState == rhs.taskState && lhs.ports == rhs.ports &&
        lhs.taskKey == rhs.taskKey &&
        lhs.pid == rhs.pid &&
        lhs.processName == rhs.processName &&
        abs(lhs.cpuPercent - rhs.cpuPercent) < 0.1 &&
        abs(lhs.memoryMB - rhs.memoryMB) < 0.1
    }
}

/// 终端 Tab 管理器
class TerminalTabManager: ObservableObject {
    @Published private(set) var tabs: [TerminalTab] = []
    @Published var selectedTabId: UUID?

    private weak var pool: SimpleTerminalPoolWrapper?
    private let monitor = ProcessMonitor()
    private var pollTimer: Timer?

    var selectedTab: TerminalTab? {
        guard let id = selectedTabId else { return nil }
        return tabs.first { $0.id == id }
    }

    var selectedTerminalId: Int? {
        selectedTab?.terminalId
    }

    // MARK: - Initialization

    init() {}

    func setPool(_ pool: SimpleTerminalPoolWrapper) {
        self.pool = pool
        monitor.setPool(pool)
        startPolling()
    }

    // MARK: - Tab Management

    /// 创建新 Tab
    @discardableResult
    func createTab(cwd: String, title: String? = nil, taskKey: TaskKey? = nil) -> TerminalTab? {
        guard let pool = pool else {
            print("[TabManager] createTab: no pool")
            return nil
        }

        let terminalId = pool.createTerminal(cwd: cwd)
        guard terminalId >= 0 else {
            print("[TabManager] createTab: failed to create terminal")
            return nil
        }

        let tab = TerminalTab(
            id: UUID(),
            terminalId: terminalId,
            title: title ?? taskKey?.displayName ?? "zsh",
            taskKey: taskKey
        )

        tabs.append(tab)
        selectedTabId = tab.id
        monitor.registerTerminal(terminalId)

        print("[TabManager] createTab: tab \(tab.id.uuidString.prefix(8)) terminalId=\(terminalId) task=\(taskKey?.displayName ?? "shell")")
        return tab
    }

    /// 查找已存在的任务 Tab
    func findTab(for taskKey: TaskKey) -> TerminalTab? {
        tabs.first { $0.taskKey == taskKey }
    }

    /// 查找或创建任务 Tab
    ///
    /// - Returns: (tab, isNew, wasRunning)
    @discardableResult
    func findOrCreateTaskTab(cwd: String, taskKey: TaskKey) -> (tab: TerminalTab, isNew: Bool, wasRunning: Bool)? {
        // 查找已存在的 Tab
        if let existingTab = findTab(for: taskKey) {
            let wasRunning = existingTab.taskState.isActive
            selectedTabId = existingTab.id
            return (existingTab, false, wasRunning)
        }

        // 创建新 Tab
        guard let newTab = createTab(cwd: cwd, title: taskKey.displayName, taskKey: taskKey) else {
            return nil
        }
        return (newTab, true, false)
    }

    /// 标记任务为"已发送命令"
    func markTaskSent(_ tabId: UUID, command: String) {
        guard let index = tabs.firstIndex(where: { $0.id == tabId }) else { return }
        tabs[index].taskState = .sent
        tabs[index].commandString = command
        tabs[index].startedAt = Date()
        tabs[index].completedAt = nil
        objectWillChange.send()
    }

    /// 标记任务完成（带退出码）
    func markTaskCompleted(_ tabId: UUID, exitCode: Int32) {
        guard let index = tabs.firstIndex(where: { $0.id == tabId }) else { return }
        tabs[index].taskState = .completed(exitCode: exitCode)
        tabs[index].completedAt = Date()
        objectWillChange.send()
    }

    /// 重置任务状态为 idle
    func markTaskIdle(_ tabId: UUID) {
        guard let index = tabs.firstIndex(where: { $0.id == tabId }) else { return }
        tabs[index].taskState = .idle
        tabs[index].startedAt = nil
        tabs[index].completedAt = nil
        objectWillChange.send()
    }

    /// 关闭 Tab
    func closeTab(_ tab: TerminalTab) {
        guard let index = tabs.firstIndex(where: { $0.id == tab.id }) else { return }

        monitor.unregisterTerminal(tab.terminalId)
        pool?.destroyTerminal(tab.terminalId)

        tabs.remove(at: index)

        // 选择相邻的 tab
        if selectedTabId == tab.id {
            if tabs.isEmpty {
                selectedTabId = nil
            } else {
                let newIndex = min(index, tabs.count - 1)
                selectedTabId = tabs[newIndex].id
            }
        }
    }

    /// 选择 Tab
    func selectTab(_ tab: TerminalTab) {
        selectedTabId = tab.id
    }

    // MARK: - Polling

    private func startPolling() {
        pollTimer?.invalidate()
        // 进程名：每 1 秒
        pollTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            self?.updateTabs()
        }
        // 端口：每 3 秒（lsof 开销较大）
        monitor.start(interval: 3.0)
        monitor.onUpdate = { [weak self] infoMap in
            DispatchQueue.main.async {
                self?.applyMonitorInfo(infoMap)
            }
        }
    }

    private func updateTabs() {
        guard let pool = pool else { return }

        var needsUpdate = false
        for i in tabs.indices {
            let terminalId = tabs[i].terminalId

            // 只有普通 shell tab 才更新进程名为标题
            // 任务 tab 保持任务名（NewsLens:Build）
            if tabs[i].taskKey == nil {
                if let name = pool.getForegroundProcessName(terminalId), !name.isEmpty {
                    if tabs[i].title != name {
                        tabs[i].title = name
                        needsUpdate = true
                    }
                }
            }

            let hasProcess = pool.hasRunningProcess(terminalId)
            let oldState = tabs[i].taskState

            switch oldState {
            case .sent:
                // sent → running：检测到子进程启动
                if hasProcess {
                    tabs[i].taskState = .running
                    needsUpdate = true
                }

            case .running:
                // running → completed：子进程结束，从 LogBuffer 解析真实退出码
                if !hasProcess {
                    let exitCode = parseExitCode(terminalId: terminalId)
                    tabs[i].taskState = .completed(exitCode: exitCode)
                    tabs[i].completedAt = Date()
                    needsUpdate = true
                }

            case .idle:
                // shell tab：跟踪 hasProcess 用于 UI 指示（不走状态机）
                // 无需状态转换

                break

            case .completed:
                // 已完成，不再变化（除非 markTaskSent 重置）
                break
            }
        }

        if needsUpdate {
            objectWillChange.send()
        }
    }

    /// LogBuffer 日志行（用于解码 tailLog JSON）
    private struct LogLine: Decodable {
        let text: String
    }

    /// 从 LogBuffer 解析任务退出码
    private func parseExitCode(terminalId: Int) -> Int32 {
        // 读取最后 20 行日志，搜索退出码标记
        guard let pool = pool,
              let logJson = pool.tailLog(terminalId, count: 20) else {
            return 0  // LogBuffer 不可用，回退到 0
        }

        // tailLog 返回 JSON 数组 [{seq, text}, ...]
        if let data = logJson.data(using: .utf8),
           let lines = try? JSONDecoder().decode([LogLine].self, from: data) {
            let text = lines.map(\.text).joined(separator: "\n")
            if let exitCode = TaskExitMarker.parse(from: text) {
                return exitCode
            }
        }

        return 0  // 未找到标记，回退到 0
    }

    private func applyMonitorInfo(_ infoMap: [Int: TerminalProcessInfo]) {
        var needsUpdate = false
        for i in tabs.indices {
            if let info = infoMap[tabs[i].terminalId] {
                if tabs[i].ports != info.listeningPorts {
                    tabs[i].ports = info.listeningPorts
                    needsUpdate = true
                }
                if abs(tabs[i].cpuPercent - info.cpuPercent) >= 0.1 {
                    tabs[i].cpuPercent = info.cpuPercent
                    needsUpdate = true
                }
                if abs(tabs[i].memoryMB - info.memoryMB) >= 0.1 {
                    tabs[i].memoryMB = info.memoryMB
                    needsUpdate = true
                }
                if tabs[i].pid != info.pid {
                    tabs[i].pid = info.pid
                    needsUpdate = true
                }
                if tabs[i].processName != info.processName {
                    tabs[i].processName = info.processName
                    needsUpdate = true
                }
            }
        }

        if needsUpdate {
            objectWillChange.send()
        }
    }

    deinit {
        pollTimer?.invalidate()
        monitor.stop()
    }
}

// MARK: - Notification Names

extension Notification.Name {
    static let stopTask = Notification.Name("stopTask")
    static let restartTask = Notification.Name("restartTask")
}
