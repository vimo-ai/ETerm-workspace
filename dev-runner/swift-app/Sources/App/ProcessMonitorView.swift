//
//  ProcessMonitorView.swift
//  DevRunner
//
//  右栏进程监控面板 — 全局进程状态总览 + 内联详情
//

import SwiftUI

/// 右栏进程监控视图
struct ProcessMonitorView: View {
    @ObservedObject var tabManager: TerminalTabManager
    @Binding var isCollapsed: Bool
    let onNewTask: () -> Void
    let onStopTask: (TerminalTab) -> Void
    let onRestartTask: (TerminalTab) -> Void

    @State private var filterMode: FilterMode = .all
    @State private var expandedTabIds: Set<UUID> = []

    enum FilterMode: String, CaseIterable {
        case all = "All"
        case active = "Active"
        case failed = "Failed"
    }

    var body: some View {
        if isCollapsed {
            collapsedStrip
        } else {
            expandedPanel
        }
    }

    // MARK: - Collapsed Strip

    private var collapsedStrip: some View {
        let activeCount = tabManager.tabs.filter { $0.taskState.isActive }.count
        let failedCount = tabManager.tabs.filter { $0.taskState.isFailed }.count

        return VStack(spacing: 12) {
            Button {
                withAnimation(.easeInOut(duration: 0.2)) { isCollapsed = false }
            } label: {
                Image(systemName: "chevron.left")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundColor(Theme.textMuted)
            }
            .buttonStyle(.plain)
            .padding(.top, 12)

            if failedCount > 0 {
                VStack(spacing: 2) {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 8))
                        .foregroundColor(Theme.error)
                    Text("\(failedCount)")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .foregroundColor(Theme.error)
                }
            }

            if activeCount > 0 {
                VStack(spacing: 2) {
                    Circle().fill(Theme.success).frame(width: 6, height: 6)
                    Text("\(activeCount)")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .foregroundColor(Theme.textSecondary)
                }
            }

            Spacer()
        }
        .frame(maxHeight: .infinity)
        .background(Theme.bgSecondary)
    }

    // MARK: - Expanded Panel

    private var expandedPanel: some View {
        VStack(spacing: 0) {
            header

            Rectangle()
                .fill(Theme.border)
                .frame(height: 1)

            if tabManager.tabs.isEmpty {
                emptyState
            } else {
                processList
            }
        }
        .background(Theme.bgSecondary)
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 8) {
            let activeCount = tabManager.tabs.filter { $0.taskState.isActive }.count
            let failedCount = tabManager.tabs.filter { $0.taskState.isFailed }.count

            Text("PROCESSES")
                .font(.system(size: 10, weight: .bold, design: .monospaced))
                .foregroundColor(Theme.textMuted)
                .tracking(1.5)

            if failedCount > 0 {
                HStack(spacing: 3) {
                    Image(systemName: "xmark.circle.fill").font(.system(size: 8)).foregroundColor(Theme.error)
                    Text("\(failedCount)").font(.system(size: 10, weight: .medium, design: .monospaced)).foregroundColor(Theme.error)
                }
            }

            if activeCount > 0 {
                HStack(spacing: 3) {
                    Circle().fill(Theme.success).frame(width: 5, height: 5)
                    Text("\(activeCount)").font(.system(size: 10, weight: .medium, design: .monospaced)).foregroundColor(Theme.textSecondary)
                }
            }

            Spacer()

            // 过滤器（进程多时显示）
            if tabManager.tabs.count > 8 {
                filterPicker
            }

            // 批量操作
            if tabManager.tabs.count > 1 {
                batchMenu
            }

            // New Task
            Button(action: onNewTask) {
                Image(systemName: "plus")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(Theme.accent)
                    .frame(width: 20, height: 20)
                    .background(Theme.accent.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 4))
            }
            .buttonStyle(.plain)
            .help("New Task")

            // 收起按钮
            Button {
                withAnimation(.easeInOut(duration: 0.2)) { isCollapsed = true }
            } label: {
                Image(systemName: "chevron.right")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundColor(Theme.textMuted)
                    .frame(width: 20, height: 20)
            }
            .buttonStyle(.plain)
            .help("Collapse")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }

    // MARK: - Filter Picker

    private var filterPicker: some View {
        HStack(spacing: 1) {
            ForEach(FilterMode.allCases, id: \.rawValue) { mode in
                Button {
                    withAnimation(.easeInOut(duration: 0.15)) { filterMode = mode }
                } label: {
                    Text(mode.rawValue)
                        .font(.system(size: 9, weight: .medium, design: .monospaced))
                        .foregroundColor(filterMode == mode ? Theme.textPrimary : Theme.textMuted)
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(filterMode == mode ? Theme.bgPrimary : Color.clear)
                        .clipShape(RoundedRectangle(cornerRadius: 3))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(2)
        .background(Theme.bgPrimary.opacity(0.3))
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }

    // MARK: - Batch Menu

    private var batchMenu: some View {
        Menu {
            let hasActive = tabManager.tabs.contains { $0.taskState.isActive }
            let hasCompleted = tabManager.tabs.contains { !$0.taskState.isActive && $0.taskState != .idle }

            if hasActive {
                Button { stopAllTasks() } label: { Label("Stop All", systemImage: "stop.fill") }
            }
            if hasCompleted {
                Button { clearCompletedTasks() } label: { Label("Clear Completed", systemImage: "trash") }
            }
        } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 10))
                .foregroundColor(Theme.textMuted)
                .frame(width: 20, height: 20)
        }
        .menuStyle(.borderlessButton)
        .frame(width: 20)
    }

    // MARK: - Empty State

    private var emptyState: some View {
        VStack(spacing: 8) {
            Image(systemName: "waveform.path.ecg")
                .font(.system(size: 20, weight: .thin))
                .foregroundColor(Theme.textMuted)
            Text("No processes")
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(Theme.textMuted)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Process List

    private var sortedTabs: [TerminalTab] {
        let tabs: [TerminalTab]
        switch filterMode {
        case .all: tabs = tabManager.tabs
        case .active: tabs = tabManager.tabs.filter { $0.taskState.isActive }
        case .failed: tabs = tabManager.tabs.filter { $0.taskState.isFailed }
        }
        return tabs.sorted { sortOrder($0) < sortOrder($1) }
    }

    private func sortOrder(_ tab: TerminalTab) -> Int {
        switch tab.taskState {
        case .completed(let code) where code != 0: return 0  // Failed 置顶
        case .running: return 1
        case .sent: return 2
        case .idle: return 3
        case .completed: return 4  // Success 最底
        }
    }

    private var processList: some View {
        ScrollView {
            LazyVStack(spacing: 1) {
                ForEach(sortedTabs) { tab in
                    ProcessRow(
                        tab: tab,
                        isSelected: tabManager.selectedTabId == tab.id,
                        isExpanded: expandedTabIds.contains(tab.id),
                        onSelect: { tabManager.selectTab(tab) },
                        onToggleExpand: {
                            withAnimation(.easeInOut(duration: 0.15)) {
                                if expandedTabIds.contains(tab.id) {
                                    expandedTabIds.remove(tab.id)
                                } else {
                                    expandedTabIds.insert(tab.id)
                                }
                            }
                        },
                        onClose: { tabManager.closeTab(tab) },
                        onStop: { onStopTask(tab) },
                        onRestart: { onRestartTask(tab) }
                    )
                }
            }
            .padding(.vertical, 2)
            .padding(.horizontal, 4)
        }
    }

    // MARK: - Actions

    private func stopAllTasks() {
        for tab in tabManager.tabs where tab.taskState.isActive {
            onStopTask(tab)
        }
    }

    private func clearCompletedTasks() {
        let completed = tabManager.tabs.filter { !$0.taskState.isActive && $0.taskState != .idle }
        for tab in completed { tabManager.closeTab(tab) }
    }
}

// MARK: - Process Row

private struct ProcessRow: View {
    let tab: TerminalTab
    let isSelected: Bool
    let isExpanded: Bool
    let onSelect: () -> Void
    let onToggleExpand: () -> Void
    let onClose: () -> Void
    let onStop: () -> Void
    let onRestart: () -> Void

    @State private var isHovered = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // 主行
            mainRow

            // 展开详情卡片
            if isExpanded {
                detailCard
            }
        }
        .background(
            RoundedRectangle(cornerRadius: 4)
                .fill(isSelected ? Theme.bgPrimary : (isHovered ? Theme.bgTertiary : Color.clear))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 4)
                .stroke(isSelected ? Theme.accent.opacity(0.3) : Color.clear, lineWidth: 1)
        )
        .contentShape(Rectangle())
        .onTapGesture { onSelect() }
        .onHover { isHovered = $0 }
    }

    // MARK: - Main Row

    private var mainRow: some View {
        VStack(alignment: .leading, spacing: 2) {
            // 第一行：状态 + 名称 + 展开箭头
            HStack(spacing: 5) {
                statusDot

                Text(tab.title)
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundColor(isSelected ? Theme.textPrimary : Theme.textSecondary)
                    .lineLimit(1)

                Spacer()

                // hover 时显示快捷操作
                if isHovered || isSelected {
                    hoverActions
                }

                // 展开/收起箭头
                Button(action: onToggleExpand) {
                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 7, weight: .bold))
                        .foregroundColor(Theme.textMuted)
                        .frame(width: 14, height: 14)
                }
                .buttonStyle(.plain)
            }

            // 第二行：端口 + 内存 + 耗时 + exit code
            HStack(spacing: 4) {
                resourceBadges
                exitBadge

                if let d = tab.durationText {
                    Text(d)
                        .font(.system(size: 9, design: .monospaced))
                        .foregroundColor(Theme.textMuted)
                }
            }
            .padding(.leading, 11) // 对齐状态点后的内容
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
    }

    // MARK: - Status Dot

    @ViewBuilder
    private var statusDot: some View {
        switch tab.taskState {
        case .idle:
            Circle().fill(Theme.textMuted.opacity(0.3)).frame(width: 6, height: 6)
        case .sent:
            Circle().fill(Theme.warning).frame(width: 6, height: 6)
        case .running:
            Circle().fill(Theme.success).frame(width: 6, height: 6)
        case .completed(let code):
            if code == 0 {
                Image(systemName: "checkmark.circle.fill").font(.system(size: 9)).foregroundColor(Theme.success)
            } else {
                Image(systemName: "xmark.circle.fill").font(.system(size: 9)).foregroundColor(Theme.error)
            }
        }
    }

    // MARK: - Resource Badges

    @ViewBuilder
    private var resourceBadges: some View {
        if tab.taskState.isActive || !tab.ports.isEmpty {
            HStack(spacing: 3) {
                if tab.memoryMB > 0.1 {
                    Text(formatMemory(tab.memoryMB))
                        .font(.system(size: 9, weight: .medium, design: .monospaced))
                        .foregroundColor(Theme.textMuted)
                }

                if let port = tab.ports.first {
                    HStack(spacing: 1) {
                        Text(":\(port)")
                            .font(.system(size: 9, weight: .semibold, design: .monospaced))
                            .foregroundColor(Theme.accent)
                        if tab.ports.count > 1 {
                            Text("+\(tab.ports.count - 1)")
                                .font(.system(size: 8, design: .monospaced))
                                .foregroundColor(Theme.textMuted)
                        }
                    }
                    .padding(.horizontal, 4).padding(.vertical, 1)
                    .background(Theme.accent.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 3))
                }
            }
        }
    }

    // MARK: - Exit Badge

    @ViewBuilder
    private var exitBadge: some View {
        if case .completed(let code) = tab.taskState, code != 0 {
            Text("exit \(code)")
                .font(.system(size: 9, weight: .medium, design: .monospaced))
                .foregroundColor(Theme.error)
                .padding(.horizontal, 4).padding(.vertical, 1)
                .background(Theme.error.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: 3))
        }
    }

    // MARK: - Hover Actions

    private var hoverActions: some View {
        HStack(spacing: 2) {
            if tab.taskState.isActive {
                Button(action: onStop) {
                    Image(systemName: "stop.fill").font(.system(size: 7))
                        .foregroundColor(Theme.error)
                        .frame(width: 16, height: 16)
                        .background(Theme.error.opacity(0.1))
                        .clipShape(RoundedRectangle(cornerRadius: 3))
                }
                .buttonStyle(.plain).help("Stop")
            }

            if !tab.taskState.isActive && tab.commandString != nil && tab.taskKey != nil {
                Button(action: onRestart) {
                    Image(systemName: "arrow.clockwise").font(.system(size: 7))
                        .foregroundColor(Theme.accent)
                        .frame(width: 16, height: 16)
                        .background(Theme.accent.opacity(0.1))
                        .clipShape(RoundedRectangle(cornerRadius: 3))
                }
                .buttonStyle(.plain).help("Restart")
            }

            Button(action: onClose) {
                Image(systemName: "xmark").font(.system(size: 6, weight: .bold))
                    .foregroundColor(Theme.textMuted)
                    .frame(width: 16, height: 16)
            }
            .buttonStyle(.plain).help("Close")
        }
    }

    // MARK: - Detail Card

    private var detailCard: some View {
        VStack(alignment: .leading, spacing: 4) {
            if tab.pid > 0 {
                detailLine(label: "PID", value: "\(tab.pid)")
            }

            if tab.cpuPercent > 0.1 {
                detailLine(label: "CPU", value: String(format: "%.1f%%", tab.cpuPercent))
            }

            if tab.memoryMB > 0.1 {
                detailLine(label: "MEM", value: formatMemory(tab.memoryMB))
            }

            if !tab.processName.isEmpty {
                detailLine(label: "PROC", value: tab.processName)
            }

            if let cmd = tab.commandString {
                detailLine(label: "CMD", value: cmd)
            }

            if !tab.ports.isEmpty {
                detailLine(label: "PORTS", value: tab.ports.map { String($0) }.joined(separator: ", "))
            }

            // 操作按钮
            HStack(spacing: 6) {
                if tab.taskState.isActive {
                    Button(action: onStop) {
                        HStack(spacing: 3) {
                            Image(systemName: "stop.fill").font(.system(size: 8))
                            Text("Stop").font(.system(size: 9, weight: .medium, design: .monospaced))
                        }
                        .foregroundColor(Theme.error)
                        .padding(.horizontal, 8).padding(.vertical, 4)
                        .background(Theme.error.opacity(0.1))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                    }
                    .buttonStyle(.plain)
                }

                if !tab.taskState.isActive && tab.commandString != nil && tab.taskKey != nil {
                    Button(action: onRestart) {
                        HStack(spacing: 3) {
                            Image(systemName: "arrow.clockwise").font(.system(size: 8))
                            Text("Restart").font(.system(size: 9, weight: .medium, design: .monospaced))
                        }
                        .foregroundColor(Theme.accent)
                        .padding(.horizontal, 8).padding(.vertical, 4)
                        .background(Theme.accent.opacity(0.1))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                    }
                    .buttonStyle(.plain)
                }

                Button(action: onClose) {
                    HStack(spacing: 3) {
                        Image(systemName: "xmark").font(.system(size: 8))
                        Text("Close").font(.system(size: 9, weight: .medium, design: .monospaced))
                    }
                    .foregroundColor(Theme.textMuted)
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .background(Theme.bgTertiary)
                    .clipShape(RoundedRectangle(cornerRadius: 4))
                }
                .buttonStyle(.plain)
            }
            .padding(.top, 4)
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 4)
                .fill(Theme.bgTertiary)
        )
        .padding(.horizontal, 8)
        .padding(.bottom, 6)
    }

    private func detailLine(label: String, value: String) -> some View {
        HStack(spacing: 6) {
            Text(label)
                .font(.system(size: 9, weight: .bold, design: .monospaced))
                .foregroundColor(Theme.textMuted)
                .frame(width: 36, alignment: .trailing)
            Text(value)
                .font(.system(size: 9, design: .monospaced))
                .foregroundColor(Theme.textSecondary)
                .lineLimit(1)
        }
    }

    private func formatMemory(_ mb: Double) -> String {
        if mb >= 1024 { return String(format: "%.1fG", mb / 1024) }
        if mb >= 100 { return String(format: "%.0fM", mb) }
        return String(format: "%.1fM", mb)
    }
}
