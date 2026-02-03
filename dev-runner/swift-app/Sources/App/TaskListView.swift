//
//  TaskListView.swift
//  DevRunner
//
//  任务列表视图 - 显示运行中/已完成的任务
//

import SwiftUI

/// 任务列表视图
struct TaskListView: View {
    @ObservedObject var tabManager: TerminalTabManager
    @EnvironmentObject var runner: DevRunner
    let onNewTask: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            // Header
            header

            Rectangle()
                .fill(Theme.border)
                .frame(height: 1)

            // Task list
            if tabManager.tabs.isEmpty {
                emptyState
            } else {
                taskList
            }
        }
        .background(Theme.bgSecondary)
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 12) {
            // Project info
            if let project = runner.selectedProject {
                HStack(spacing: 6) {
                    Image(systemName: project.adapterType == "xcode" ? "hammer.fill" : "cube.fill")
                        .font(.system(size: 12))
                        .foregroundColor(project.adapterType == "xcode" ? Theme.xcode : Theme.node)

                    Text(project.name)
                        .font(.system(size: 13, weight: .semibold, design: .monospaced))
                        .foregroundColor(Theme.textPrimary)
                }
            }

            Spacer()

            // Running count
            let runningCount = tabManager.tabs.filter(\.isRunning).count
            if runningCount > 0 {
                HStack(spacing: 4) {
                    Circle()
                        .fill(Theme.success)
                        .frame(width: 6, height: 6)
                    Text("\(runningCount) running")
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundColor(Theme.textSecondary)
                }
            }

            // New task button
            Button(action: onNewTask) {
                HStack(spacing: 4) {
                    Image(systemName: "plus")
                        .font(.system(size: 10, weight: .bold))
                    Text("New")
                        .font(.system(size: 11, weight: .medium, design: .monospaced))
                }
                .foregroundColor(Theme.accent)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(Theme.accent.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: 4))
                .overlay(
                    RoundedRectangle(cornerRadius: 4)
                        .stroke(Theme.accent.opacity(0.3), lineWidth: 1)
                )
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    // MARK: - Empty State

    private var emptyState: some View {
        VStack(spacing: 12) {
            Image(systemName: "terminal")
                .font(.system(size: 24, weight: .thin))
                .foregroundColor(Theme.textMuted)

            Text("No tasks")
                .font(.system(size: 12, weight: .medium, design: .monospaced))
                .foregroundColor(Theme.textSecondary)

            Text("Click 'New' to start a task")
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(Theme.textMuted)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }

    // MARK: - Task List

    private var taskList: some View {
        ScrollView {
            LazyVStack(spacing: 2) {
                ForEach(tabManager.tabs) { tab in
                    TaskRow(
                        tab: tab,
                        isSelected: tabManager.selectedTabId == tab.id,
                        onSelect: { tabManager.selectTab(tab) },
                        onClose: { tabManager.closeTab(tab) },
                        onStop: { onStopTask(tab) }
                    )
                }
            }
            .padding(.vertical, 4)
            .padding(.horizontal, 6)
        }
    }

    private func onStopTask(_ tab: TerminalTab) {
        // 通过 notification 让 ContentView 发送 interrupt
        NotificationCenter.default.post(
            name: .stopTask,
            object: nil,
            userInfo: ["terminalId": tab.terminalId]
        )
    }
}

// MARK: - Task Row

private struct TaskRow: View {
    let tab: TerminalTab
    let isSelected: Bool
    let onSelect: () -> Void
    let onClose: () -> Void
    let onStop: () -> Void

    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 8) {
            // Status indicator
            statusIndicator

            // Task info
            VStack(alignment: .leading, spacing: 2) {
                // Title
                Text(tab.title)
                    .font(.system(size: 12, weight: .medium, design: .monospaced))
                    .foregroundColor(isSelected ? Theme.textPrimary : Theme.textSecondary)
                    .lineLimit(1)

                // Subtitle (task type)
                if let taskKey = tab.taskKey {
                    Text(taskKey.action.rawValue.capitalized)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(Theme.textMuted)
                }
            }

            Spacer()

            // Resource badges
            if tab.isRunning {
                resourceBadges
            }

            // Actions
            if isHovered || isSelected {
                actionButtons
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Theme.bgPrimary : (isHovered ? Theme.bgTertiary : Color.clear))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .stroke(isSelected ? Theme.accent.opacity(0.3) : Color.clear, lineWidth: 1)
        )
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .onHover { isHovered = $0 }
    }

    private var statusIndicator: some View {
        Circle()
            .fill(statusColor)
            .frame(width: 8, height: 8)
            .overlay(
                Circle()
                    .stroke(statusColor.opacity(0.5), lineWidth: tab.isRunning ? 2 : 0)
                    .scaleEffect(tab.isRunning ? 1.5 : 1)
                    .opacity(tab.isRunning ? 0.5 : 0)
            )
    }

    private var statusColor: Color {
        if tab.isRunning {
            return Theme.success
        } else if tab.taskKey != nil {
            return Theme.textMuted
        } else {
            return Theme.textMuted.opacity(0.5)
        }
    }

    private var resourceBadges: some View {
        HStack(spacing: 4) {
            // CPU
            if tab.cpuPercent > 0.1 {
                Text(String(format: "%.0f%%", tab.cpuPercent))
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundColor(tab.cpuPercent > 50 ? Theme.warning : Theme.textMuted)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 2)
                    .background(Theme.bgPrimary.opacity(0.5))
                    .clipShape(RoundedRectangle(cornerRadius: 3))
            }

            // Memory
            if tab.memoryMB > 0.1 {
                Text(formatMemory(tab.memoryMB))
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundColor(Theme.textMuted)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 2)
                    .background(Theme.bgPrimary.opacity(0.5))
                    .clipShape(RoundedRectangle(cornerRadius: 3))
            }

            // Port
            if let port = tab.ports.first {
                HStack(spacing: 2) {
                    Text(":\(port)")
                        .font(.system(size: 9, weight: .semibold, design: .monospaced))
                        .foregroundColor(Theme.accent)
                    if tab.ports.count > 1 {
                        Text("+\(tab.ports.count - 1)")
                            .font(.system(size: 8, design: .monospaced))
                            .foregroundColor(Theme.textMuted)
                    }
                }
                .padding(.horizontal, 4)
                .padding(.vertical, 2)
                .background(Theme.accent.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: 3))
            }
        }
    }

    private var actionButtons: some View {
        HStack(spacing: 4) {
            // Stop button (only for running tasks)
            if tab.isRunning {
                Button(action: onStop) {
                    Image(systemName: "stop.fill")
                        .font(.system(size: 9))
                        .foregroundColor(Theme.error)
                        .frame(width: 20, height: 20)
                        .background(Theme.error.opacity(0.1))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
                .buttonStyle(.plain)
                .help("Stop task")
            }

            // Close button
            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundColor(Theme.textMuted)
                    .frame(width: 20, height: 20)
                    .background(isHovered ? Theme.textMuted.opacity(0.1) : Color.clear)
                    .clipShape(RoundedRectangle(cornerRadius: 4))
            }
            .buttonStyle(.plain)
            .help("Close tab")
        }
    }

    private func formatMemory(_ mb: Double) -> String {
        if mb >= 1024 {
            return String(format: "%.1fG", mb / 1024)
        } else if mb >= 100 {
            return String(format: "%.0fM", mb)
        } else {
            return String(format: "%.1fM", mb)
        }
    }
}

// MARK: - Notification

extension Notification.Name {
    static let stopTask = Notification.Name("stopTask")
}
