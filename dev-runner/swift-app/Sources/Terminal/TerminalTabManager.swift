//
//  TerminalTabManager.swift
//  DevRunner
//
//  管理多个终端 Tab + 进程监控
//

import SwiftUI
import Combine

/// 单个终端 Tab
struct TerminalTab: Identifiable, Equatable {
    let id: UUID
    let terminalId: Int          // TerminalPool 中的 ID
    var title: String            // 显示名称（前台进程名）
    var isRunning: Bool = false  // 是否有子进程在跑
    var ports: [UInt16] = []     // 监听端口

    static func == (lhs: TerminalTab, rhs: TerminalTab) -> Bool {
        lhs.id == rhs.id && lhs.title == rhs.title &&
        lhs.isRunning == rhs.isRunning && lhs.ports == rhs.ports
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
    func createTab(cwd: String, title: String? = nil) -> TerminalTab? {
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
            title: title ?? "zsh"
        )

        tabs.append(tab)
        selectedTabId = tab.id
        monitor.registerTerminal(terminalId)

        print("[TabManager] createTab: tab \(tab.id.uuidString.prefix(8)) terminalId=\(terminalId)")
        return tab
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

            if let name = pool.getForegroundProcessName(terminalId), !name.isEmpty {
                if tabs[i].title != name {
                    tabs[i].title = name
                    needsUpdate = true
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
