//
//  ProcessMonitor.swift
//  DevRunner
//
//  进程监控：端口检测、子进程追踪
//

import Foundation

/// 单个进程的监控信息
struct TerminalProcessInfo: Equatable {
    var processName: String = ""
    var isRunning: Bool = false
    var listeningPorts: [UInt16] = []
}

/// 进程监控器
///
/// 通过基线快照机制检测端口：
/// - 终端创建时记录系统端口基线
/// - 只显示基线之后新增的端口
class ProcessMonitor {

    private weak var pool: SimpleTerminalPoolWrapper?
    private var timer: Timer?
    private var processInfoCache: [Int: TerminalProcessInfo] = [:]

    /// 每个终端创建时的端口基线
    private var portBaseline: [Int: Set<UInt16>] = [:]

    /// 监控信息更新回调
    var onUpdate: (([Int: TerminalProcessInfo]) -> Void)?

    init() {}

    func setPool(_ pool: SimpleTerminalPoolWrapper) {
        self.pool = pool
    }

    func start(interval: TimeInterval = 3.0) {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            DispatchQueue.global(qos: .utility).async {
                self?.poll()
            }
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    func info(for terminalId: Int) -> TerminalProcessInfo {
        processInfoCache[terminalId] ?? TerminalProcessInfo()
    }

    /// 注册终端 - 同时记录端口基线
    func registerTerminal(_ terminalId: Int) {
        processInfoCache[terminalId] = TerminalProcessInfo()
        // 快照当前系统所有端口作为基线
        portBaseline[terminalId] = currentSystemPorts()
    }

    func unregisterTerminal(_ terminalId: Int) {
        processInfoCache.removeValue(forKey: terminalId)
        portBaseline.removeValue(forKey: terminalId)
    }

    // MARK: - Polling

    private func poll() {
        guard let pool = pool else { return }

        // 获取当前所有监听端口（进程名 → 端口集合）
        let portMap = detectAllListeningPorts()
        let allCurrentPorts = Set(portMap.values.flatMap { $0 })

        var updated = false
        for (terminalId, oldInfo) in processInfoCache {
            var newInfo = TerminalProcessInfo()

            // 进程名
            if let name = pool.getForegroundProcessName(terminalId) {
                newInfo.processName = name
            }

            // 运行状态
            newInfo.isRunning = pool.hasRunningProcess(terminalId)

            // 端口检测 - 只显示基线后新增且属于当前进程的端口
            if !newInfo.processName.isEmpty, let baseline = portBaseline[terminalId] {
                let key = newInfo.processName.lowercased()
                let processPorts = Set(portMap[key] ?? portMap[String(key.prefix(9))] ?? [])
                // 交集：属于此进程 AND 不在基线中
                let newPorts = processPorts.subtracting(baseline)
                newInfo.listeningPorts = newPorts.sorted()
            }

            if newInfo != oldInfo {
                processInfoCache[terminalId] = newInfo
                updated = true
            }
        }

        if updated {
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.onUpdate?(self.processInfoCache)
            }
        }
    }

    // MARK: - Port Detection

    /// 获取当前系统所有监听端口
    private func currentSystemPorts() -> Set<UInt16> {
        let portMap = detectAllListeningPorts()
        return Set(portMap.values.flatMap { $0 })
    }

    /// 检测所有监听端口，返回 [进程名(小写): [端口]]
    private func detectAllListeningPorts() -> [String: [UInt16]] {
        let output = runCommand("/usr/sbin/lsof", args: ["-i", "-P", "-n", "-sTCP:LISTEN"])
        guard !output.isEmpty else { return [:] }

        var result: [String: [UInt16]] = [:]

        for line in output.split(separator: "\n") {
            let fields = line.split(separator: " ", omittingEmptySubsequences: true)
            guard fields.count >= 10 else { continue }

            let command = String(fields[0]).lowercased()
            guard command != "command" else { continue }

            // 地址:端口 在倒数第 2 个字段
            let addrPort = String(fields[fields.count - 2])

            if let colonIndex = addrPort.lastIndex(of: ":") {
                let portStr = addrPort[addrPort.index(after: colonIndex)...]
                if let port = UInt16(portStr) {
                    result[command, default: []].append(port)
                }
            }
        }

        // 去重排序
        for key in result.keys {
            result[key] = Array(Set(result[key]!)).sorted()
        }

        return result
    }

    private func runCommand(_ path: String, args: [String]) -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = args

        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice

        do {
            try process.run()
            process.waitUntilExit()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            return String(data: data, encoding: .utf8) ?? ""
        } catch {
            return ""
        }
    }

    deinit {
        stop()
    }
}
