//
//  NewTaskSheet.swift
//  DevRunner
//
//  新建任务配置弹窗
//

import SwiftUI

/// 新建任务配置弹窗
struct NewTaskSheet: View {
    @EnvironmentObject var runner: DevRunner
    @Binding var isPresented: Bool
    let onStart: (TaskAction, TargetInfo?, DeviceInfo?) -> Void

    @State private var selectedAction: TaskAction = .run
    @State private var selectedTarget: TargetInfo?
    @State private var selectedDevice: DeviceInfo?

    var body: some View {
        VStack(spacing: 0) {
            // Header
            header

            Rectangle()
                .fill(Theme.border)
                .frame(height: 1)

            // Content
            content

            Rectangle()
                .fill(Theme.border)
                .frame(height: 1)

            // Footer
            footer
        }
        .frame(width: 360)
        .background(Theme.bgSecondary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(Theme.border, lineWidth: 1)
        )
        .shadow(color: .black.opacity(0.5), radius: 20)
        .onAppear {
            // 初始化选择
            selectedTarget = runner.selectedTarget
            selectedDevice = runner.selectedDevice
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack {
            Text("NEW TASK")
                .font(.system(size: 11, weight: .bold, design: .monospaced))
                .foregroundColor(Theme.textPrimary)
                .tracking(1.5)

            Spacer()

            Button {
                isPresented = false
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(Theme.textMuted)
                    .frame(width: 24, height: 24)
                    .background(Theme.bgTertiary)
                    .clipShape(RoundedRectangle(cornerRadius: 4))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    // MARK: - Content

    private var content: some View {
        VStack(spacing: 16) {
            // Action selector
            actionSelector

            // Target selector
            targetSelector

            // Device selector (for run action)
            if selectedAction == .run {
                deviceSelector
            }
        }
        .padding(16)
    }

    private var actionSelector: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("ACTION")
                .font(.system(size: 9, weight: .bold, design: .monospaced))
                .foregroundColor(Theme.textMuted)
                .tracking(1)

            HStack(spacing: 8) {
                ActionButton(
                    action: .build,
                    icon: "hammer.fill",
                    isSelected: selectedAction == .build
                ) {
                    selectedAction = .build
                }

                ActionButton(
                    action: .run,
                    icon: "play.fill",
                    isSelected: selectedAction == .run
                ) {
                    selectedAction = .run
                }

                ActionButton(
                    action: .shell,
                    icon: "terminal",
                    isSelected: selectedAction == .shell
                ) {
                    selectedAction = .shell
                }
            }
        }
    }

    private var targetSelector: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("TARGET")
                .font(.system(size: 9, weight: .bold, design: .monospaced))
                .foregroundColor(Theme.textMuted)
                .tracking(1)

            Menu {
                ForEach(runner.targets, id: \.name) { target in
                    Button {
                        selectedTarget = target
                    } label: {
                        HStack {
                            Text(target.name)
                            Spacer()
                            Text(targetTypeBadge(target).0)
                                .foregroundColor(targetTypeBadge(target).1)
                        }
                    }
                }
            } label: {
                HStack {
                    if let target = selectedTarget {
                        let badge = targetTypeBadge(target)
                        Text(target.name)
                            .font(.system(size: 12, weight: .medium, design: .monospaced))
                            .foregroundColor(Theme.textPrimary)

                        Spacer()

                        Text(badge.0)
                            .font(.system(size: 9, weight: .bold, design: .monospaced))
                            .foregroundColor(badge.1)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(badge.1.opacity(0.15))
                            .clipShape(RoundedRectangle(cornerRadius: 3))
                    } else {
                        Text("Select target...")
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundColor(Theme.textSecondary)
                        Spacer()
                    }

                    Image(systemName: "chevron.down")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(Theme.textMuted)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(Theme.bgTertiary)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Theme.border, lineWidth: 1)
                )
            }
            .buttonStyle(.plain)
        }
    }

    private var deviceSelector: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("DEVICE")
                .font(.system(size: 9, weight: .bold, design: .monospaced))
                .foregroundColor(Theme.textMuted)
                .tracking(1)

            Menu {
                ForEach(runner.devices.filter(\.isAvailable), id: \.deviceId) { device in
                    Button {
                        selectedDevice = device
                    } label: {
                        HStack {
                            Image(systemName: deviceIcon(device))
                            Text(device.name)
                        }
                    }
                }
            } label: {
                HStack {
                    if let device = selectedDevice {
                        Image(systemName: deviceIcon(device))
                            .font(.system(size: 11))
                            .foregroundColor(Theme.accent)
                            .frame(width: 16)

                        Text(device.name)
                            .font(.system(size: 12, weight: .medium, design: .monospaced))
                            .foregroundColor(Theme.textPrimary)
                    } else {
                        Text("Select device...")
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundColor(Theme.textSecondary)
                    }

                    Spacer()

                    Image(systemName: "chevron.down")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(Theme.textMuted)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(Theme.bgTertiary)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Theme.border, lineWidth: 1)
                )
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: - Footer

    private var footer: some View {
        HStack {
            // Preview
            if let project = runner.selectedProject {
                let taskName = previewTaskName(project: project)
                Text(taskName)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(Theme.textMuted)
            }

            Spacer()

            // Cancel
            Button("Cancel") {
                isPresented = false
            }
            .buttonStyle(.plain)
            .font(.system(size: 12, weight: .medium, design: .monospaced))
            .foregroundColor(Theme.textSecondary)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)

            // Start
            Button {
                onStart(selectedAction, selectedTarget, selectedDevice)
                isPresented = false
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: actionIcon)
                        .font(.system(size: 10, weight: .semibold))
                    Text("Start")
                        .font(.system(size: 12, weight: .semibold, design: .monospaced))
                }
                .foregroundColor(Theme.bgPrimary)
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .background(Theme.accent)
                .clipShape(RoundedRectangle(cornerRadius: 6))
            }
            .buttonStyle(.plain)
            .disabled(!canStart)
            .opacity(canStart ? 1 : 0.5)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    // MARK: - Helpers

    private var canStart: Bool {
        switch selectedAction {
        case .shell:
            return true
        case .build:
            return selectedTarget != nil
        case .run:
            return selectedTarget != nil
        }
    }

    private var actionIcon: String {
        switch selectedAction {
        case .build: return "hammer.fill"
        case .run: return "play.fill"
        case .shell: return "terminal"
        }
    }

    private func previewTaskName(project: ProjectInfo) -> String {
        switch selectedAction {
        case .shell:
            return "zsh"
        case .build:
            return "\(project.name):Build"
        case .run:
            if let device = selectedDevice {
                return "\(project.name):\(device.name)"
            }
            return "\(project.name):Run"
        }
    }

    private func targetTypeBadge(_ target: TargetInfo) -> (String, Color) {
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

    private func deviceIcon(_ device: DeviceInfo) -> String {
        if device.isMac { return "desktopcomputer" }
        if device.isSimulator { return "iphone.gen2" }
        return "iphone"
    }
}

// MARK: - Action Button

private struct ActionButton: View {
    let action: TaskAction
    let icon: String
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        Button(action: onSelect) {
            VStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.system(size: 16, weight: .medium))

                Text(action.rawValue.capitalized)
                    .font(.system(size: 10, weight: .medium, design: .monospaced))
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 12)
            .foregroundColor(isSelected ? Theme.accent : Theme.textSecondary)
            .background(isSelected ? Theme.accent.opacity(0.1) : Theme.bgTertiary)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(isSelected ? Theme.accent : Theme.border, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }
}
