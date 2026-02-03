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
    var isRunning: Bool = false  // 是否有子进程在跑
    var ports: [UInt16] = []     // 监听端口
    var cpuPercent: Double = 0   // CPU 使用率 %
    var memoryMB: Double = 0     // 内存占用 MB

    // Task 关联
    var taskKey: TaskKey?        // 关联的任务（nil 表示普通 shell）

    static func == (lhs: TerminalTab, rhs: TerminalTab) -> Bool {
        lhs.id == rhs.id && lhs.title == rhs.title &&
        lhs.isRunning == rhs.isRunning && lhs.ports == rhs.ports &&
        lhs.taskKey == rhs.taskKey &&
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
            let wasRunning = existingTab.isRunning
            selectedTabId = existingTab.id
            return (existingTab, false, wasRunning)
        }

        // 创建新 Tab
        guard let newTab = createTab(cwd: cwd, title: taskKey.displayName, taskKey: taskKey) else {
            return nil
        }
        return (newTab, true, false)
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

            let running = pool.hasRunningProcess(terminalId)
            if tabs[i].isRunning != running {
                tabs[i].isRunning = running
                needsUpdate = true
            }
        }

        if needsUpdate {
            objectWillChange.send()
        }
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
