import Foundation

// MARK: - FFI Models

struct ProjectInfo: Codable, Identifiable, Hashable {
    var id: String { path }
    let adapterType: String
    let name: String
    let path: String
    let bundleId: String?

    enum CodingKeys: String, CodingKey {
        case adapterType = "adapter_type"
        case name, path
        case bundleId = "bundle_id"
    }
}

struct TargetInfo: Codable, Identifiable, Hashable {
    var id: String { name }
    let name: String
    let targetType: String
    let description: String?

    enum CodingKeys: String, CodingKey {
        case name
        case targetType = "target_type"
        case description
    }
}

struct DeviceInfo: Codable, Identifiable, Hashable {
    var id: String { deviceId }
    let deviceId: String
    let name: String
    let deviceType: String
    let osVersion: String?
    let state: String

    enum CodingKeys: String, CodingKey {
        case deviceId = "id"
        case name
        case deviceType = "device_type"
        case osVersion = "os_version"
        case state
    }

    var isAvailable: Bool { state == "available" }
    var isMac: Bool { deviceType == "mac" }
    var isSimulator: Bool { deviceType == "simulator" }
    var isPhysical: Bool { deviceType == "physical" }
}

struct CommandInfo: Codable {
    let program: String
    let args: [String]
    let cwd: String?
    let env: [String: String]
    let display: String
}

struct ProcessStartResult: Codable {
    let processId: String

    enum CodingKeys: String, CodingKey {
        case processId = "process_id"
    }
}

struct ProcessInfo: Codable, Identifiable {
    var id: String { processId }
    let processId: String
    let projectPath: String
    let adapterType: String
    let target: String
    let pid: UInt32?
    let status: String
    let errorMessage: String?
    let startedAt: Int64
    let endedAt: Int64?

    enum CodingKeys: String, CodingKey {
        case processId = "id"
        case projectPath = "project_path"
        case adapterType = "adapter_type"
        case target, pid, status
        case errorMessage = "error_message"
        case startedAt = "started_at"
        case endedAt = "ended_at"
    }

    var isRunning: Bool { status == "running" }
}

struct OutputReadResult: Codable {
    let lines: [String]
    let nextLine: UInt64

    enum CodingKeys: String, CodingKey {
        case lines
        case nextLine = "next_line"
    }
}

// MARK: - Workspace Model

struct Workspace: Identifiable, Hashable {
    let id: UUID
    let path: String
    var name: String { URL(fileURLWithPath: path).lastPathComponent }
    var projects: [ProjectInfo]

    init(path: String, projects: [ProjectInfo]) {
        self.id = UUID()
        self.path = path
        self.projects = projects
    }
}

// MARK: - DevRunner Error

enum DevRunnerError: Error, LocalizedError {
    case initFailed
    case ffiError(String)
    case decodingError(String)
    case noWorkspaceSelected
    case noProjectSelected

    var errorDescription: String? {
        switch self {
        case .initFailed:
            return "Failed to initialize DevRunner"
        case .ffiError(let msg):
            return msg
        case .decodingError(let msg):
            return "JSON decoding error: \(msg)"
        case .noWorkspaceSelected:
            return "No workspace selected"
        case .noProjectSelected:
            return "No project selected"
        }
    }
}

// MARK: - DevRunner

@MainActor
final class DevRunner: ObservableObject {
    static let shared: DevRunner = {
        do {
            return try DevRunner()
        } catch {
            fatalError("Failed to init DevRunner: \(error)")
        }
    }()

    private var handle: OpaquePointer?

    /// All workspaces
    @Published var workspaces: [Workspace] = []

    /// Currently selected workspace
    @Published var selectedWorkspace: Workspace?

    /// Currently selected project within workspace
    @Published var selectedProject: ProjectInfo?

    /// Currently selected target
    @Published var selectedTarget: TargetInfo?

    /// Currently selected device
    @Published var selectedDevice: DeviceInfo?

    /// Targets for selected project
    @Published private(set) var targets: [TargetInfo] = []

    /// Devices for selected project
    @Published private(set) var devices: [DeviceInfo] = []

    /// All processes
    @Published private(set) var processes: [ProcessInfo] = []

    private static let configDir = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(".vimo/dev-runner")
    private static let configFile = configDir.appendingPathComponent("config.json")

    private init() throws {
        guard let h = dev_runner_init() else {
            throw DevRunnerError.initFailed
        }
        handle = h

        // Load saved workspaces
        loadWorkspaces()
    }

    // MARK: - Persistence

    private struct Config: Codable {
        var workspaces: [String]
        var lastSelected: String?
    }

    private func loadWorkspaces() {
        guard FileManager.default.fileExists(atPath: Self.configFile.path) else {
            return
        }

        do {
            let data = try Data(contentsOf: Self.configFile)
            let config = try JSONDecoder().decode(Config.self, from: data)

            for path in config.workspaces {
                if FileManager.default.fileExists(atPath: path) {
                    try? addWorkspace(path: path)
                }
            }

            // Restore last selected
            if let lastPath = config.lastSelected,
               let ws = workspaces.first(where: { $0.path == lastPath }) {
                selectWorkspace(ws)
            }
        } catch {
            print("Failed to load config: \(error)")
        }
    }

    private func saveWorkspaces() {
        do {
            // Ensure directory exists
            try FileManager.default.createDirectory(at: Self.configDir, withIntermediateDirectories: true)

            let config = Config(
                workspaces: workspaces.map(\.path),
                lastSelected: selectedWorkspace?.path
            )
            let data = try JSONEncoder().encode(config)
            try data.write(to: Self.configFile)
        } catch {
            print("Failed to save config: \(error)")
        }
    }

    deinit {
        if let h = handle {
            dev_runner_free(h)
        }
    }

    // MARK: - Workspace Management

    /// Add a new workspace
    func addWorkspace(path: String) throws {
        // Check if already exists
        if workspaces.contains(where: { $0.path == path }) {
            // Just select it
            if let existing = workspaces.first(where: { $0.path == path }) {
                selectWorkspace(existing)
            }
            return
        }

        // Detect projects
        let projects = try detect(path: path)
        let workspace = Workspace(path: path, projects: projects)
        workspaces.append(workspace)

        // Save
        saveWorkspaces()

        // Auto-select
        selectWorkspace(workspace)
    }

    /// Remove a workspace
    func removeWorkspace(_ workspace: Workspace) {
        workspaces.removeAll { $0.id == workspace.id }

        // Save
        saveWorkspaces()

        if selectedWorkspace?.id == workspace.id {
            selectedWorkspace = workspaces.first
            if let ws = selectedWorkspace {
                selectWorkspace(ws)
            } else {
                selectedProject = nil
                selectedTarget = nil
                selectedDevice = nil
                targets = []
                devices = []
            }
        }
    }

    /// Select a workspace
    func selectWorkspace(_ workspace: Workspace) {
        selectedWorkspace = workspace

        // Save last selected
        saveWorkspaces()

        // Auto-select first project
        if let first = workspace.projects.first {
            selectProject(first)
        } else {
            selectedProject = nil
            selectedTarget = nil
            selectedDevice = nil
            targets = []
            devices = []
        }
    }

    /// Select a project（异步加载，不阻塞 UI）
    func selectProject(_ project: ProjectInfo) {
        selectedProject = project

        // 先清空，显示 loading 状态
        targets = []
        devices = []
        selectedTarget = nil
        selectedDevice = nil

        let projectPath = project.path
        let handleCopy = handle  // 捕获 handle

        // 后台加载 targets 和 devices
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            // 先 open 项目（必须在 list 之前）
            Self.openProjectSync(handle: handleCopy, projectPath: projectPath)

            // FFI 调用（在后台线程）
            let loadedTargets = Self.listTargetsSync(handle: handleCopy, projectPath: projectPath)
            let loadedDevices = Self.listDevicesSync(handle: handleCopy, projectPath: projectPath)

            // 回到主线程更新 UI
            DispatchQueue.main.async {
                guard let self = self else { return }
                guard self.selectedProject?.path == projectPath else { return }  // 防止切换后覆盖

                self.targets = loadedTargets
                self.devices = loadedDevices

                // Smart auto-select target: prefer App > first
                self.selectedTarget = loadedTargets.first(where: { $0.targetType.lowercased().contains("app") })
                    ?? loadedTargets.first

                // Smart auto-select device: prefer Simulator > Physical > Mac
                self.selectedDevice = loadedDevices.first(where: { $0.isSimulator && $0.isAvailable })
                    ?? loadedDevices.first(where: { $0.isAvailable })
            }
        }
    }

    /// 同步版本的 openProject（静态方法，用于后台线程）
    private static func openProjectSync(handle: OpaquePointer?, projectPath: String) {
        guard let handle = handle else { return }
        var errorPtr: UnsafeMutablePointer<CChar>?
        let resultPtr = projectPath.withCString { pathPtr in
            dev_runner_open(handle, pathPtr, &errorPtr)
        }
        if let resultPtr = resultPtr {
            dev_runner_free_string(resultPtr)
        }
        errorPtr.map { dev_runner_free_string($0) }
    }

    /// 同步版本的 listTargets（静态方法，用于后台线程）
    private static func listTargetsSync(handle: OpaquePointer?, projectPath: String) -> [TargetInfo] {
        guard let handle = handle else { return [] }
        var errorPtr: UnsafeMutablePointer<CChar>?
        guard let resultPtr = projectPath.withCString({ pathPtr in
            dev_runner_list_targets(handle, pathPtr, &errorPtr)
        }) else {
            errorPtr.map { dev_runner_free_string($0) }
            return []
        }
        let json = String(cString: resultPtr)
        dev_runner_free_string(resultPtr)

        guard let data = json.data(using: .utf8),
              let targets = try? JSONDecoder().decode([TargetInfo].self, from: data) else {
            return []
        }
        return targets
    }

    /// 同步版本的 listDevices（静态方法，用于后台线程）
    private static func listDevicesSync(handle: OpaquePointer?, projectPath: String) -> [DeviceInfo] {
        guard let handle = handle else { return [] }
        var errorPtr: UnsafeMutablePointer<CChar>?
        guard let resultPtr = projectPath.withCString({ pathPtr in
            dev_runner_list_devices(handle, pathPtr, &errorPtr)
        }) else {
            errorPtr.map { dev_runner_free_string($0) }
            return []
        }
        let json = String(cString: resultPtr)
        dev_runner_free_string(resultPtr)

        guard let data = json.data(using: .utf8),
              let devices = try? JSONDecoder().decode([DeviceInfo].self, from: data) else {
            return []
        }
        return devices
    }

    // MARK: - Project Detection

    func detect(path: String) throws -> [ProjectInfo] {
        let json = try callFFI { errorPtr in
            path.withCString { pathPtr in
                dev_runner_detect(pathPtr, errorPtr)
            }
        }
        return try decode([ProjectInfo].self, from: json)
    }

    // MARK: - Internal

    private func openProjectInternal(path: String) throws {
        let _ = try callFFI { errorPtr in
            path.withCString { pathPtr in
                dev_runner_open(handle, pathPtr, errorPtr)
            }
        }
    }

    func listTargets(projectPath: String) throws -> [TargetInfo] {
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                dev_runner_list_targets(handle, pathPtr, errorPtr)
            }
        }
        return try decode([TargetInfo].self, from: json)
    }

    func listDevices(projectPath: String) throws -> [DeviceInfo] {
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                dev_runner_list_devices(handle, pathPtr, errorPtr)
            }
        }
        return try decode([DeviceInfo].self, from: json)
    }

    // MARK: - Stateless API (for ControlAPIServer, no side effects on UI state)

    /// Project context resolved for API use, without touching @Published state
    struct ProjectContext {
        let projectPath: String
        let target: TargetInfo
        let device: DeviceInfo?
        let allTargets: [TargetInfo]
        let allDevices: [DeviceInfo]
    }

    /// Resolve project context: open project in FFI, list targets/devices, auto-select.
    /// Does NOT modify any @Published UI state.
    func resolveProjectContext(
        projectPath: String,
        targetName: String?,
        deviceName: String?
    ) -> ProjectContext? {
        // Ensure project metadata is loaded in the Rust handle
        Self.openProjectSync(handle: handle, projectPath: projectPath)

        let targets = Self.listTargetsSync(handle: handle, projectPath: projectPath)
        let devices = Self.listDevicesSync(handle: handle, projectPath: projectPath)

        // Resolve target: explicit name > app target > first
        let target: TargetInfo
        if let name = targetName,
           let found = targets.first(where: { $0.name == name }) {
            target = found
        } else if let app = targets.first(where: { $0.targetType.lowercased().contains("app") }) {
            target = app
        } else if let first = targets.first {
            target = first
        } else {
            return nil
        }

        // Resolve device: explicit name > simulator > physical > mac
        let device: DeviceInfo?
        if let name = deviceName {
            device = devices.first(where: { $0.name == name })
        } else {
            device = devices.first(where: { $0.isSimulator && $0.isAvailable })
                ?? devices.first(where: { $0.isAvailable })
        }

        return ProjectContext(
            projectPath: projectPath,
            target: target,
            device: device,
            allTargets: targets,
            allDevices: devices
        )
    }

    /// Generate build command with explicit parameters (no global state dependency)
    func buildCommand(projectPath: String, target: String, options: BuildOptions?) throws -> CommandInfo {
        let optionsJson = try options.map { try encode($0) }
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                target.withCString { targetPtr in
                    if let opts = optionsJson {
                        return opts.withCString { optsPtr in
                            dev_runner_build_cmd(handle, pathPtr, targetPtr, optsPtr, errorPtr)
                        }
                    } else {
                        return dev_runner_build_cmd(handle, pathPtr, targetPtr, nil, errorPtr)
                    }
                }
            }
        }
        return try decode(CommandInfo.self, from: json)
    }

    /// Generate install command with explicit parameters (no global state dependency)
    func installCommand(projectPath: String, target: String, options: RunOptions?) throws -> CommandInfo {
        let optionsJson = try options.map { try encode($0) }
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                target.withCString { targetPtr in
                    if let opts = optionsJson {
                        return opts.withCString { optsPtr in
                            dev_runner_install_cmd(handle, pathPtr, targetPtr, optsPtr, errorPtr)
                        }
                    } else {
                        return dev_runner_install_cmd(handle, pathPtr, targetPtr, nil, errorPtr)
                    }
                }
            }
        }
        return try decode(CommandInfo.self, from: json)
    }

    /// Generate run command with explicit parameters (no global state dependency)
    func runCommand(projectPath: String, target: String, options: RunOptions?) throws -> CommandInfo {
        let optionsJson = try options.map { try encode($0) }
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                target.withCString { targetPtr in
                    if let opts = optionsJson {
                        return opts.withCString { optsPtr in
                            dev_runner_run_cmd(handle, pathPtr, targetPtr, optsPtr, errorPtr)
                        }
                    } else {
                        return dev_runner_run_cmd(handle, pathPtr, targetPtr, nil, errorPtr)
                    }
                }
            }
        }
        return try decode(CommandInfo.self, from: json)
    }

    // MARK: - Command Generation (UI-stateful, reads from selectedProject/selectedTarget)

    func buildCommand(options: BuildOptions? = nil) throws -> CommandInfo {
        guard let projectPath = selectedProject?.path else {
            throw DevRunnerError.noProjectSelected
        }
        guard let target = selectedTarget?.name else {
            throw DevRunnerError.noProjectSelected
        }
        let optionsJson = try options.map { try encode($0) }
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                target.withCString { targetPtr in
                    if let opts = optionsJson {
                        return opts.withCString { optsPtr in
                            dev_runner_build_cmd(handle, pathPtr, targetPtr, optsPtr, errorPtr)
                        }
                    } else {
                        return dev_runner_build_cmd(handle, pathPtr, targetPtr, nil, errorPtr)
                    }
                }
            }
        }
        return try decode(CommandInfo.self, from: json)
    }

    func runCommand(options: RunOptions? = nil) throws -> CommandInfo {
        guard let projectPath = selectedProject?.path else {
            throw DevRunnerError.noProjectSelected
        }
        guard let target = selectedTarget?.name else {
            throw DevRunnerError.noProjectSelected
        }
        let optionsJson = try options.map { try encode($0) }
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                target.withCString { targetPtr in
                    if let opts = optionsJson {
                        return opts.withCString { optsPtr in
                            dev_runner_run_cmd(handle, pathPtr, targetPtr, optsPtr, errorPtr)
                        }
                    } else {
                        return dev_runner_run_cmd(handle, pathPtr, targetPtr, nil, errorPtr)
                    }
                }
            }
        }
        return try decode(CommandInfo.self, from: json)
    }

    // MARK: - Process Management

    func startProcess(command: CommandInfo) throws -> String {
        guard let projectPath = selectedProject?.path else {
            throw DevRunnerError.noProjectSelected
        }
        guard let target = selectedTarget?.name else {
            throw DevRunnerError.noProjectSelected
        }
        let commandJson = try encode(command)
        let json = try callFFI { errorPtr in
            projectPath.withCString { pathPtr in
                target.withCString { targetPtr in
                    commandJson.withCString { cmdPtr in
                        dev_runner_start_process(handle, pathPtr, targetPtr, cmdPtr, errorPtr)
                    }
                }
            }
        }
        let result = try decode(ProcessStartResult.self, from: json)
        refreshProcesses()
        return result.processId
    }

    func stopProcess(id: String) throws {
        var errorPtr: UnsafeMutablePointer<CChar>?
        let success = id.withCString { idPtr in
            dev_runner_stop_process(handle, idPtr, &errorPtr)
        }
        if !success {
            let error = errorPtr.map { String(cString: $0) } ?? "Unknown error"
            errorPtr.map { dev_runner_free_string($0) }
            throw DevRunnerError.ffiError(error)
        }
        refreshProcesses()
    }

    func refreshProcesses() {
        processes = (try? listProcesses()) ?? []
    }

    func listProcesses() throws -> [ProcessInfo] {
        let json = try callFFI { errorPtr in
            dev_runner_list_processes(handle, errorPtr)
        }
        return try decode([ProcessInfo].self, from: json)
    }

    func getProcess(id: String) throws -> ProcessInfo {
        let json = try callFFI { errorPtr in
            id.withCString { idPtr in
                dev_runner_get_process(handle, idPtr, errorPtr)
            }
        }
        return try decode(ProcessInfo.self, from: json)
    }

    func readOutput(processId: String, sinceLine: UInt64) throws -> OutputReadResult {
        let json = try callFFI { errorPtr in
            processId.withCString { idPtr in
                dev_runner_read_output(handle, idPtr, sinceLine, errorPtr)
            }
        }
        return try decode(OutputReadResult.self, from: json)
    }

    // MARK: - Private Helpers

    private func callFFI(_ block: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> UnsafeMutablePointer<CChar>?) throws -> String {
        var errorPtr: UnsafeMutablePointer<CChar>?
        guard let resultPtr = block(&errorPtr) else {
            let error = errorPtr.map { String(cString: $0) } ?? "Unknown error"
            errorPtr.map { dev_runner_free_string($0) }
            throw DevRunnerError.ffiError(error)
        }
        let result = String(cString: resultPtr)
        dev_runner_free_string(resultPtr)
        return result
    }

    private func decode<T: Decodable>(_ type: T.Type, from json: String) throws -> T {
        guard let data = json.data(using: .utf8) else {
            throw DevRunnerError.decodingError("Invalid UTF-8")
        }
        do {
            return try JSONDecoder().decode(type, from: data)
        } catch {
            throw DevRunnerError.decodingError(error.localizedDescription)
        }
    }

    private func encode<T: Encodable>(_ value: T) throws -> String {
        let data = try JSONEncoder().encode(value)
        guard let json = String(data: data, encoding: .utf8) else {
            throw DevRunnerError.decodingError("Failed to encode to UTF-8")
        }
        return json
    }
}

// MARK: - Options

struct BuildOptions: Codable {
    var config: String?
    var clean: Bool = false
    var deviceId: String?
    var env: [String: String] = [:]

    enum CodingKeys: String, CodingKey {
        case config, clean
        case deviceId = "device_id"
        case env
    }
}

struct RunOptions: Codable {
    var deviceId: String?
    var env: [String: String] = [:]
    var args: [String] = []

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case env, args
    }
}
