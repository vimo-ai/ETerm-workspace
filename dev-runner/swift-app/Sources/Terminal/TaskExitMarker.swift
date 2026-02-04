//
//  TaskExitMarker.swift
//  DevRunner
//
//  命令退出码标记：包装命令 + 从日志解析退出码
//

import Foundation

/// 任务退出码标记工具
///
/// 在发送命令时追加退出码标记，任务结束后从 LogBuffer 解析真实退出码。
/// 标记格式：`__DR_EXIT:<exitCode>__`
enum TaskExitMarker {
    static let prefix = "__DR_EXIT:"
    static let suffix = "__"

    /// 包装命令，追加退出码标记
    ///
    /// 输出：`<command>; printf '__DR_EXIT:%d__\n' $?`
    static func wrap(_ command: String) -> String {
        return "\(command); printf '\\n\(prefix)%d\(suffix)\\n' $?"
    }

    /// 从日志文本中解析退出码
    ///
    /// 扫描最后几行，找到 `__DR_EXIT:<N>__` 标记并提取 N
    /// - Returns: 退出码，未找到返回 nil
    static func parse(from logText: String) -> Int32? {
        // 从后往前扫描，标记通常在最后几行
        let lines = logText.split(separator: "\n", omittingEmptySubsequences: false)
        for line in lines.reversed() {
            if let exitCode = parseLine(String(line)) {
                return exitCode
            }
        }
        return nil
    }

    /// 解析单行
    private static func parseLine(_ line: String) -> Int32? {
        guard let prefixRange = line.range(of: prefix) else { return nil }
        let afterPrefix = line[prefixRange.upperBound...]
        guard let suffixRange = afterPrefix.range(of: suffix) else { return nil }
        let codeStr = afterPrefix[..<suffixRange.lowerBound]
        return Int32(codeStr)
    }
}
