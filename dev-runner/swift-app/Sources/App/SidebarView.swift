import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// 项目树节点（支持 OutlineGroup）
struct ProjectTreeNode: Identifiable {
    let id: String
    let name: String
    let icon: String
    let iconColor: Color
    let isProject: Bool
    let project: ProjectInfo?
    let fullPath: String
    let children: [ProjectTreeNode]?

    static func folder(name: String, path: String, fullPath: String, children: [ProjectTreeNode]) -> ProjectTreeNode {
        ProjectTreeNode(id: "folder:\(path)", name: name, icon: "folder", iconColor: Theme.textMuted,
                       isProject: false, project: nil, fullPath: fullPath, children: children.isEmpty ? nil : children)
    }

    static func project(_ project: ProjectInfo) -> ProjectTreeNode {
        ProjectTreeNode(id: "project:\(project.path)", name: project.name,
                       icon: project.adapterType == "xcode" ? "hammer.fill" : "cube.fill",
                       iconColor: project.adapterType == "xcode" ? Theme.xcode : Theme.node,
                       isProject: true, project: project, fullPath: project.path, children: nil)
    }
}

/// Sidebar 视图
struct SidebarView: View {
    @EnvironmentObject var runner: DevRunner
    @Binding var isDraggingOver: Bool
    var onAddWorkspace: () -> Void
    var onDrop: ([NSItemProvider]) -> Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("PROJECTS").font(.system(size: 10, weight: .bold, design: .monospaced)).foregroundColor(Theme.textMuted).tracking(1.5)
                Spacer()
            }.padding(.horizontal, 16).padding(.top, 20).padding(.bottom, 12)

            List {
                ForEach(groupedWorkspaces, id: \.id) { group in
                    if let groupName = group.groupName {
                        Section { workspacesContent(group.workspaces) } header: {
                            Text(groupName).font(.system(size: 10, design: .monospaced)).foregroundColor(Theme.textMuted)
                        }
                    } else {
                        workspacesContent(group.workspaces)
                    }
                }
            }.listStyle(.sidebar).scrollContentBackground(.hidden)

            Button(action: onAddWorkspace) {
                HStack(spacing: 8) {
                    Image(systemName: "plus").font(.system(size: 12, weight: .semibold))
                    Text("Add Workspace").font(.system(size: 12, weight: .medium, design: .monospaced))
                }
                .foregroundColor(Theme.accent).frame(maxWidth: .infinity).padding(.vertical, 12)
                .background(RoundedRectangle(cornerRadius: 6).stroke(Theme.accent.opacity(0.3), lineWidth: 1).background(Theme.accent.opacity(0.05)))
                .clipShape(RoundedRectangle(cornerRadius: 6))
            }.buttonStyle(.plain).padding(12)
        }
        .background(Theme.bgSecondary)
        .overlay(RoundedRectangle(cornerRadius: 0).stroke(Theme.accent, lineWidth: isDraggingOver ? 2 : 0).animation(.easeInOut(duration: 0.2), value: isDraggingOver))
        .onDrop(of: [.fileURL], isTargeted: $isDraggingOver, perform: onDrop)
    }

    private var groupedWorkspaces: [(id: String, groupName: String?, workspaces: [Workspace])] {
        let homeDir = FileManager.default.homeDirectoryForCurrentUser.path

        // 过滤掉是其他 workspace 子目录的 workspace
        let filteredWorkspaces = runner.workspaces.filter { workspace in
            for other in runner.workspaces where other.path != workspace.path {
                if workspace.path.hasPrefix(other.path + "/") {
                    return false
                }
            }
            return true
        }

        // 按前两级目录分组
        let grouped = Dictionary(grouping: filteredWorkspaces) { workspace -> String in
            let path = workspace.path
            guard path.hasPrefix(homeDir) else { return "__root__" }
            let relativePath = String(path.dropFirst(homeDir.count)).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let components = relativePath.split(separator: "/").map(String.init)
            return components.count >= 3 ? "\(homeDir)/\(components[0])/\(components[1])" : "__standalone__\(path)"
        }

        var result: [(id: String, groupName: String?, workspaces: [Workspace])] = []
        for (groupKey, workspaces) in grouped {
            if groupKey.hasPrefix("__standalone__") || groupKey == "__root__" {
                result.append((id: groupKey, groupName: nil, workspaces: workspaces))
            } else {
                result.append((id: groupKey, groupName: groupKey.replacingOccurrences(of: homeDir, with: "~"), workspaces: workspaces))
            }
        }
        return result.sorted { ($0.groupName ?? "") < ($1.groupName ?? "") }
    }

    private func buildProjectTree(workspace: Workspace) -> [ProjectTreeNode] {
        var tree: [String: [ProjectInfo]] = [:]
        for project in workspace.projects {
            let relativePath = project.path.replacingOccurrences(of: workspace.path, with: "").trimmingCharacters(in: CharacterSet(charactersIn: "/"))
            let components = relativePath.split(separator: "/").map(String.init)
            let key = components.count == 1 ? "" : components.dropLast().joined(separator: "/")
            tree[key, default: []].append(project)
        }

        // 收集所有 folder 路径（含中间路径），解决 vlaude/packages 等深层路径的父节点缺失
        var allFolders: Set<String> = []
        for key in tree.keys where !key.isEmpty {
            var current = key
            while !current.isEmpty {
                allFolders.insert(current)
                let parent = (current as NSString).deletingLastPathComponent
                current = parent
            }
        }

        let basePath = workspace.path
        func buildNodes(at path: String) -> [ProjectTreeNode] {
            var nodes: [ProjectTreeNode] = []
            let childFolders = allFolders.filter { folderPath in
                if path.isEmpty {
                    return !folderPath.contains("/")
                } else {
                    return (folderPath as NSString).deletingLastPathComponent == path
                }
            }.sorted()
            for folderPath in childFolders {
                let absPath = basePath + "/" + folderPath
                nodes.append(.folder(name: (folderPath as NSString).lastPathComponent, path: folderPath, fullPath: absPath, children: buildNodes(at: folderPath)))
            }
            if let projects = tree[path] { nodes.append(contentsOf: projects.map { .project($0) }) }
            return nodes
        }
        return buildNodes(at: "")
    }

    @ViewBuilder
    private func workspacesContent(_ workspaces: [Workspace]) -> some View {
        ForEach(workspaces) { workspace in
            DisclosureGroup {
                OutlineGroup(buildProjectTree(workspace: workspace), children: \.children) { treeNodeRow($0) }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "folder.fill").font(.system(size: 12)).foregroundColor(Theme.accent)
                    Text(workspace.name).font(.system(size: 12, weight: .medium, design: .monospaced)).foregroundColor(Theme.textPrimary)
                    Spacer()
                    if workspace.projects.count > 1 {
                        Text("\(workspace.projects.count)").font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundColor(Theme.textMuted).padding(.horizontal, 5).padding(.vertical, 2)
                            .background(Theme.bgPrimary.opacity(0.5)).clipShape(RoundedRectangle(cornerRadius: 3))
                    }
                }
                .contextMenu {
                    Button {
                        NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: workspace.path)
                    } label: {
                        Label("在访达中打开", systemImage: "folder")
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func treeNodeRow(_ node: ProjectTreeNode) -> some View {
        let isSelected = node.project.map { runner.selectedProject?.path == $0.path } ?? false
        HStack(spacing: 6) {
            Image(systemName: node.icon).font(.system(size: node.isProject ? 11 : 10)).foregroundColor(node.iconColor)
            Text(node.name).font(.system(size: 12, design: .monospaced))
                .foregroundColor(node.isProject ? (isSelected ? Theme.textPrimary : Theme.textSecondary) : Theme.textMuted).lineLimit(1)
            Spacer()
            if let project = node.project {
                Text(project.adapterType == "xcode" ? "Xcode" : "Node").font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundColor(node.iconColor).padding(.horizontal, 5).padding(.vertical, 2)
                    .background(node.iconColor.opacity(0.1)).clipShape(RoundedRectangle(cornerRadius: 3))
            }
        }
        .padding(.vertical, 3).padding(.trailing, 8).background(isSelected ? Theme.bgHover : Color.clear)
        .contentShape(Rectangle())
        .onTapGesture { if let project = node.project { runner.selectProject(project) } }
        .contextMenu {
            Button {
                NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: node.fullPath)
            } label: {
                Label("在访达中打开", systemImage: "folder")
            }
        }
    }
}
