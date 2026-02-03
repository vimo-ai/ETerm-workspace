//
//  TerminalTabBar.swift
//  DevRunner
//
//  终端 Tab 栏
//

import SwiftUI

struct TerminalTabBar: View {
    @ObservedObject var tabManager: TerminalTabManager
    let onAddTab: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            // Tabs
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 1) {
                    ForEach(tabManager.tabs) { tab in
                        TabItem(
                            tab: tab,
                            isSelected: tabManager.selectedTabId == tab.id,
                            onSelect: { tabManager.selectTab(tab) },
                            onClose: { tabManager.closeTab(tab) }
                        )
                    }
                }
                .padding(.horizontal, 4)
            }

            Spacer()

            // Add button
            Button(action: onAddTab) {
                Image(systemName: "plus")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(Theme.textSecondary)
                    .frame(width: 28, height: 28)
                    .background(Theme.bgSecondary.opacity(0.5))
                    .clipShape(RoundedRectangle(cornerRadius: 4))
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 8)
        }
        .frame(height: 36)
        .background(Theme.bgSecondary)
    }
}

// MARK: - Tab Item

private struct TabItem: View {
    let tab: TerminalTab
    let isSelected: Bool
    let onSelect: () -> Void
    let onClose: () -> Void

    @State private var isHovered = false

    private func formatMemory(_ mb: Double) -> String {
        if mb >= 1024 {
            return String(format: "%.1fG", mb / 1024)
        } else if mb >= 100 {
            return String(format: "%.0fM", mb)
        } else {
            return String(format: "%.1fM", mb)
        }
    }

    var body: some View {
        HStack(spacing: 6) {
            // Status indicator
            Circle()
                .fill(tab.isRunning ? Theme.success : Theme.textMuted.opacity(0.5))
                .frame(width: 6, height: 6)

            // Title
            Text(tab.title)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .foregroundColor(isSelected ? Theme.textPrimary : Theme.textSecondary)
                .lineLimit(1)

            // Resource info (only when running)
            if tab.isRunning && (tab.cpuPercent > 0.1 || tab.memoryMB > 0.1) {
                HStack(spacing: 4) {
                    // CPU
                    if tab.cpuPercent > 0.1 {
                        Text(String(format: "%.0f%%", tab.cpuPercent))
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundColor(tab.cpuPercent > 50 ? Theme.warning : Theme.textMuted)
                    }
                    // Memory
                    if tab.memoryMB > 0.1 {
                        Text(formatMemory(tab.memoryMB))
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundColor(Theme.textMuted)
                    }
                }
                .padding(.horizontal, 4)
                .padding(.vertical, 1)
                .background(Theme.bgPrimary.opacity(0.5))
                .clipShape(RoundedRectangle(cornerRadius: 3))
            }

            // Port badge (compact: show first port only)
            if let port = tab.ports.first {
                Text(":\(String(port))")
                    .font(.system(size: 9, weight: .semibold, design: .monospaced))
                    .foregroundColor(Theme.accent)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 1)
                    .background(Theme.accent.opacity(0.15))
                    .clipShape(RoundedRectangle(cornerRadius: 3))
                if tab.ports.count > 1 {
                    Text("+\(String(tab.ports.count - 1))")
                        .font(.system(size: 9, weight: .medium, design: .monospaced))
                        .foregroundColor(Theme.textMuted)
                }
            }

            // Close button
            if isHovered || isSelected {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundColor(Theme.textMuted)
                        .frame(width: 14, height: 14)
                        .background(isHovered ? Theme.textMuted.opacity(0.2) : Color.clear)
                        .clipShape(RoundedRectangle(cornerRadius: 2))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(
            RoundedRectangle(cornerRadius: 4)
                .fill(isSelected ? Theme.bgPrimary : (isHovered ? Theme.bgSecondary.opacity(0.8) : Color.clear))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 4)
                .stroke(isSelected ? Theme.accent.opacity(0.3) : Color.clear, lineWidth: 1)
        )
        .onTapGesture(perform: onSelect)
        .onHover { isHovered = $0 }
    }
}
