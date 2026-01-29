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

            // Port badges
            if !tab.ports.isEmpty {
                ForEach(tab.ports, id: \.self) { port in
                    Text(":\(port)")
                        .font(.system(size: 9, weight: .semibold, design: .monospaced))
                        .foregroundColor(Theme.accent)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1)
                        .background(Theme.accent.opacity(0.15))
                        .clipShape(RoundedRectangle(cornerRadius: 3))
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
