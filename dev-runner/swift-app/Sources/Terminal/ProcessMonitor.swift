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
    var childCount: Int = 0
}

/// 进程监控器
///
/// 定期检测每个终端的前台进程信息：
/// - 进程名（通过 TerminalPool FFI）
/// - 监听端口（通过 lsof）
/// - 子进程数量（通过 pgrep）
class ProcessMonitor {

    private weak var pool: SimpleTerminalPoolWrapper?
    private var timer: Timer?
    private var processInfoCache: [Int: TerminalProcessInfo] = [:]  // terminalId -> ProcessInfo

    /// 监控信息更新回调
    var onUpdate: (([Int: TerminalProcessInfo]) -> Void)?

    init() {}

    func setPool(_ pool: SimpleTerminalPoolWrapper) {
        self.pool = pool
    }

    /// 开始监控
    func start(interval: TimeInterval = 2.0) {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            self?.poll()
        }
    }

    /// 停止监控
    func stop() {
        timer?.invalidate()
        timer = nil
    }

    /// 查询指定终端的进程信息
    func info(for terminalId: Int) -> TerminalProcessInfo {
        processInfoCache[terminalId] ?? TerminalProcessInfo()
    }

    // MARK: - Polling

    private func poll() {
        guard let pool = pool else { return }

        // 获取所有监听端口的快照（一次 lsof 调用）
        let portMap = detectAllListeningPorts()

        var updated = false
        for (terminalId, oldInfo) in processInfoCache {
            var newInfo = TerminalProcessInfo()

            // 进程名
            if let name = pool.getForegroundProcessName(terminalId) {
                newInfo.processName = name
            }

            // 是否在运行
            newInfo.isRunning = pool.hasRunningProcess(terminalId)

            // 端口 - 根据进程名匹配（小写比较，处理 lsof COMMAND 截断）
            if !newInfo.processName.isEmpty {
                let key = newInfo.processName.lowercased()
                // 精确匹配 or 前缀匹配（lsof 截断到 9 字符）
                if let ports = portMap[key] {
                    newInfo.listeningPorts = ports
                } else {
                    // lsof COMMAND 可能被截断，用前缀匹配
                    let truncated = String(key.prefix(9))
                    newInfo.listeningPorts = portMap[truncated] ?? []
                }
            }

            if newInfo != oldInfo {
                processInfoCache[terminalId] = newInfo
                updated = true
            }
        }

        if updated {
            onUpdate?(processInfoCache)
        }
    }

    /// 注册终端 ID
    func registerTerminal(_ terminalId: Int) {
        processInfoCache[terminalId] = TerminalProcessInfo()
    }

    /// 注销终端 ID
    func unregisterTerminal(_ terminalId: Int) {
        processInfoCache.removeValue(forKey: terminalId)
    }

    // MARK: - Port Detection

    /// 检测所有监听端口，返回 [进程名: [端口]] 映射
    private func detectAllListeningPorts() -> [String: [UInt16]] {
        let output = runCommand("/usr/sbin/lsof", args: ["-i", "-P", "-n", "-sTCP:LISTEN"])
        guard !output.isEmpty else { return [:] }

        var result: [String: [UInt16]] = [:]

        // 解析 lsof 输出
        // COMMAND  PID  USER  FD  TYPE DEVICE SIZE/OFF NODE NAME
        // node    1234  user  22u IPv4 ...          TCP *:3000 (LISTEN)
        for line in output.split(separator: "\n") {
            let fields = line.split(separator: " ", omittingEmptySubsequences: true)
            guard fields.count >= 9 else { continue }

            let command = String(fields[0]).lowercased()
            let name = String(fields.last ?? "")

            // 解析端口号：*:3000 或 127.0.0.1:8080
            if let colonIndex = name.lastIndex(of: ":") {
                let portStr = name[name.index(after: colonIndex)...]
                    .replacingOccurrences(of: " (LISTEN)", with: "")
                if let port = UInt16(portStr) {
                    result[command, default: []].append(port)
                }
            }
        }

        // 去重
        for key in result.keys {
            result[key] = Array(Set(result[key] ?? []).sorted())
        }

        return result
    }

    // MARK: - Shell Execution

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
