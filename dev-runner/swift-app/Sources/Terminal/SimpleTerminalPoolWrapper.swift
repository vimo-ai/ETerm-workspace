//
//  SimpleTerminalPoolWrapper.swift
//  DevRunner
//
//  简化版终端池封装，专为 DevRunner 设计
//  只需要基本功能：创建终端、发送命令、渲染输出
//

import Foundation
import AppKit

/// 简化版终端池 Wrapper
///
/// DevRunner 专用，只需要：
/// - 创建终端（指定工作目录）
/// - 发送命令
/// - 基本渲染
/// - 自动滚动到底部
class SimpleTerminalPoolWrapper {

    // MARK: - Properties

    private var handle: TerminalPoolHandle?

    /// 暴露 handle 用于 RenderScheduler 绑定
    var poolHandle: TerminalPoolHandle? { handle }

    /// 当前终端 ID（DevRunner 只需要一个终端）
    private(set) var terminalId: Int = -1

    // MARK: - Initialization

    /// 创建终端池
    ///
    /// - Parameters:
    ///   - windowHandle: NSView 的原始指针
    ///   - width: 窗口宽度（逻辑像素）
    ///   - height: 窗口高度（逻辑像素）
    ///   - scale: DPI 缩放因子
    ///   - fontSize: 字体大小
    init?(windowHandle: UnsafeMutableRawPointer,
          width: Float,
          height: Float,
          scale: Float,
          fontSize: Float = 13.0) {

        print("[TerminalPool] init: width=\(width), height=\(height), scale=\(scale)")

        let config = TerminalPoolConfig(
            cols: 80,
            rows: 24,
            font_size: fontSize,
            line_height: 1.0,
            scale: scale,
            window_handle: windowHandle,
            display_handle: windowHandle,
            window_width: width,
            window_height: height,
            history_size: 10000,
            log_buffer_size: 10000  // dev-runner 启用日志捕获
        )

        handle = terminal_pool_create(config)
        print("[TerminalPool] init: handle=\(handle != nil ? "ok" : "nil")")

        guard handle != nil else {
            return nil
        }

        // 注册事件回调（必须！否则 PTY 事件不会触发渲染）
        // 回调本身不需要做任何事，但注册动作会启用内部事件队列
        terminal_pool_set_event_callback(handle, { _, event in
            // 简单日志，用于调试
            print("[TerminalPool] event: type=\(event.event_type.rawValue)")
        }, nil)
        print("[TerminalPool] init: event callback registered")
    }

    deinit {
        if let handle = handle {
            terminal_pool_destroy(handle)
        }
    }

    // MARK: - Terminal Management

    /// 创建终端（使用指定的工作目录）
    ///
    /// - Parameter cwd: 工作目录
    /// - Returns: 终端 ID，失败返回 -1
    @discardableResult
    func createTerminal(cwd: String) -> Int {
        guard let handle = handle else { return -1 }
        let id = terminal_pool_create_terminal_with_cwd(handle, 80, 24, cwd)
        terminalId = Int(id)
        return terminalId
    }

    /// 关闭终端
    func closeTerminal() {
        guard let handle = handle, terminalId >= 0 else { return }
        terminal_pool_close_terminal(handle, terminalId)
        terminalId = -1
    }

    /// 销毁指定终端
    func destroyTerminal(_ terminalId: Int) {
        guard let handle = handle else { return }
        terminal_pool_close_terminal(handle, terminalId)
    }

    /// 获取前台进程名称
    func getForegroundProcessName(_ terminalId: Int) -> String? {
        guard let handle = handle else { return nil }
        guard let cStr = terminal_pool_get_foreground_process_name(handle, terminalId) else {
            return nil
        }
        let name = String(cString: cStr)
        rio_free_string(cStr)
        return name.isEmpty ? nil : name
    }

    /// 检查是否有运行中的子进程
    func hasRunningProcess(_ terminalId: Int) -> Bool {
        guard let handle = handle else { return false }
        return terminal_pool_has_running_process(handle, terminalId)
    }

    /// 向指定终端发送命令
    func sendCommand(_ command: String, to terminalId: Int) {
        guard let handle = handle, terminalId >= 0 else { return }

        let input = command + "\n"
        guard let data = input.data(using: .utf8) else { return }

        data.withUnsafeBytes { ptr in
            guard let baseAddress = ptr.baseAddress else { return }
            _ = terminal_pool_input(handle, terminalId, baseAddress.assumingMemoryBound(to: UInt8.self), data.count)
        }
    }

    /// 向指定终端发送中断
    func sendInterrupt(to terminalId: Int) {
        guard let handle = handle, terminalId >= 0 else { return }
        var byte: UInt8 = 0x03
        _ = terminal_pool_input(handle, terminalId, &byte, 1)
    }

    // MARK: - Input

    /// 渲染请求回调（由外部设置）
    var onNeedsRender: (() -> Void)?

    /// 发送命令到终端
    ///
    /// - Parameter command: 命令字符串（不包含换行符）
    func sendCommand(_ command: String) {
        guard let handle = handle, terminalId >= 0 else { return }

        // 添加换行符执行命令
        let input = command + "\n"
        guard let data = input.data(using: .utf8) else { return }

        data.withUnsafeBytes { ptr in
            guard let baseAddress = ptr.baseAddress else { return }
            _ = terminal_pool_input(handle, terminalId, baseAddress.assumingMemoryBound(to: UInt8.self), data.count)
        }
    }

    /// 发送 Ctrl+C 中断
    func sendInterrupt() {
        guard let handle = handle, terminalId >= 0 else { return }

        // Ctrl+C = 0x03
        var byte: UInt8 = 0x03
        _ = terminal_pool_input(handle, terminalId, &byte, 1)
    }

    // MARK: - Rendering

    /// 开始新的一帧
    func beginFrame() {
        guard let handle = handle else { return }
        terminal_pool_begin_frame(handle)
    }

    /// 渲染终端到指定位置
    ///
    /// - Parameters:
    ///   - x, y: 渲染位置（逻辑坐标）
    ///   - width, height: 终端区域大小
    func renderTerminal(x: Float, y: Float, width: Float, height: Float) {
        guard let handle = handle, terminalId >= 0 else { return }
        _ = terminal_pool_render_terminal(handle, terminalId, x, y, width, height)
    }

    /// 结束帧
    func endFrame() {
        guard let handle = handle else { return }
        terminal_pool_end_frame(handle)
    }

    /// 调整渲染表面大小
    func resizeSugarloaf(width: Float, height: Float) {
        guard let handle = handle else { return }
        terminal_pool_resize_sugarloaf(handle, width, height)
    }

    /// 设置 DPI 缩放
    func setScale(_ scale: Float) {
        guard let handle = handle else { return }
        terminal_pool_set_scale(handle, scale)
    }

    /// 设置渲染布局（简化版，只有一个终端）
    func setRenderLayout(x: Float, y: Float, width: Float, height: Float, containerHeight: Float) {
        setRenderLayout(terminalId: terminalId, x: x, y: y, width: width, height: height, containerHeight: containerHeight)
    }

    /// 设置指定终端的渲染布局
    func setRenderLayout(terminalId: Int, x: Float, y: Float, width: Float, height: Float, containerHeight: Float) {
        guard let handle = handle, terminalId >= 0 else {
            print("[TerminalPool] setRenderLayout: no handle or termId")
            return
        }

        print("[TerminalPool] setRenderLayout: termId=\(terminalId) x=\(x) y=\(y) w=\(width) h=\(height) container=\(containerHeight)")

        var layouts = [TerminalRenderLayout(
            terminal_id: terminalId,
            x: x,
            y: y,
            width: width,
            height: height
        )]

        layouts.withUnsafeMutableBufferPointer { buffer in
            terminal_pool_set_render_layout(handle, buffer.baseAddress, buffer.count, containerHeight)
        }
    }

    /// 触发渲染
    func renderAll() {
        guard let handle = handle else { return }
        terminal_pool_render_all(handle)
    }

    // MARK: - Scrolling

    /// 滚动终端
    ///
    /// - Parameter delta: 滚动行数（正数向上，负数向下）
    func scroll(delta: Int32) {
        guard let handle = handle, terminalId >= 0 else { return }
        _ = terminal_pool_scroll(handle, terminalId, delta)
    }

    /// 滚动指定终端
    func scroll(terminalId: Int, delta: Int32) {
        guard let handle = handle, terminalId >= 0 else { return }
        _ = terminal_pool_scroll(handle, terminalId, delta)
    }

    // MARK: - Output Capture (for MCP API)

    /// 获取终端可见行（用于日志 API）
    ///
    /// - Parameter terminalId: 终端 ID
    /// - Returns: 可见行文本数组，失败返回空数组
    func getVisibleLines(_ terminalId: Int) -> [String] {
        guard let handle = handle, terminalId >= 0 else { return [] }

        // FFI: const char*** -> Swift 需要 UnsafeMutablePointer<UnsafePointer<CChar>?>
        var linesPtr: UnsafeMutablePointer<UnsafePointer<CChar>?>? = nil
        var count: Int = 0

        let success = terminal_pool_get_visible_lines(handle, Int64(terminalId), &linesPtr, &count)

        guard success, let lines = linesPtr, count > 0 else { return [] }

        defer {
            terminal_pool_free_string_array(lines, count)
        }

        var result: [String] = []
        result.reserveCapacity(count)

        for i in 0..<count {
            if let cStr = lines[i] {
                result.append(String(cString: cStr))
            }
        }

        return result
    }

    /// 获取终端滚动历史行数
    func getScrollbackLines(_ terminalId: Int) -> Int {
        guard let handle = handle, terminalId >= 0 else { return 0 }
        let lines = terminal_pool_get_scrollback_lines(handle, Int64(terminalId))
        return max(0, Int(lines))
    }

    // MARK: - LogBuffer API (Rust 层日志捕获)

    /// 查询终端日志（分页 + 搜索）
    ///
    /// - Parameters:
    ///   - terminalId: 终端 ID
    ///   - since: 返回 seq > since 的日志（0 = 全部）
    ///   - limit: 最多返回行数
    ///   - search: 可选的搜索过滤
    /// - Returns: JSON 字符串，nil 表示 LogBuffer 未启用
    func queryLog(_ terminalId: Int, since: UInt64 = 0, limit: Int = 200, search: String? = nil) -> String? {
        guard let handle = handle, terminalId >= 0 else { return nil }

        let result: UnsafeMutablePointer<CChar>?
        if let search = search {
            result = terminal_pool_query_log(handle, terminalId, since, limit, search)
        } else {
            result = terminal_pool_query_log(handle, terminalId, since, limit, nil)
        }

        guard let cStr = result else { return nil }
        let json = String(cString: cStr)
        rio_free_string(cStr)
        return json
    }

    /// 获取终端最后 N 行日志
    ///
    /// - Parameters:
    ///   - terminalId: 终端 ID
    ///   - count: 行数
    /// - Returns: JSON 数组字符串，nil 表示 LogBuffer 未启用
    func tailLog(_ terminalId: Int, count: Int = 100) -> String? {
        guard let handle = handle, terminalId >= 0 else { return nil }

        guard let cStr = terminal_pool_tail_log(handle, terminalId, count) else { return nil }
        let json = String(cString: cStr)
        rio_free_string(cStr)
        return json
    }

    /// 清空终端日志缓冲
    func clearLog(_ terminalId: Int) -> Bool {
        guard let handle = handle, terminalId >= 0 else { return false }
        return terminal_pool_clear_log(handle, terminalId)
    }

    // MARK: - Raw Input (for paste)

    /// 写入原始数据到终端（不添加换行符）
    func writeInput(terminalId: Int, data: String) {
        guard let handle = handle, terminalId >= 0 else { return }
        guard let utf8Data = data.data(using: .utf8) else { return }

        utf8Data.withUnsafeBytes { ptr in
            guard let baseAddress = ptr.baseAddress else { return }
            _ = terminal_pool_input(handle, terminalId, baseAddress.assumingMemoryBound(to: UInt8.self), utf8Data.count)
        }
    }

    // MARK: - Selection

    /// 屏幕坐标转绝对坐标
    ///
    /// - Returns: (absoluteRow, col)，失败返回 nil
    func screenToAbsolute(terminalId: Int, screenRow: Int, screenCol: Int) -> (absoluteRow: Int64, col: Int)? {
        guard let handle = handle else { return nil }
        let result = terminal_pool_screen_to_absolute(handle, terminalId, screenRow, screenCol)
        guard result.success else { return nil }
        return (result.absolute_row, result.col)
    }

    /// 设置选区
    @discardableResult
    func setSelection(terminalId: Int, startAbsoluteRow: Int64, startCol: Int, endAbsoluteRow: Int64, endCol: Int) -> Bool {
        guard let handle = handle else { return false }
        return terminal_pool_set_selection(handle, terminalId, startAbsoluteRow, startCol, endAbsoluteRow, endCol)
    }

    /// 清除选区
    @discardableResult
    func clearSelection(terminalId: Int) -> Bool {
        guard let handle = handle else { return false }
        return terminal_pool_clear_selection(handle, terminalId)
    }

    /// 完成选区（mouseUp 时调用）
    ///
    /// 如果选区全是空白，Rust 会自动清除选区并返回 nil
    func finalizeSelection(terminalId: Int) -> String? {
        guard let handle = handle else { return nil }
        let result = terminal_pool_finalize_selection(handle, terminalId)
        guard result.has_selection, let textPtr = result.text else { return nil }
        let text = String(cString: textPtr)
        terminal_pool_free_string(textPtr)
        return text
    }

    /// 获取选中文本（不清除选区）
    func getSelectionText(terminalId: Int) -> String? {
        guard let handle = handle else { return nil }
        let result = terminal_pool_get_selection_text(handle, terminalId)
        guard result.success, let textPtr = result.text else { return nil }
        let text = String(cString: textPtr)
        terminal_pool_free_string(textPtr)
        return text
    }

    // MARK: - Font Metrics

    /// 获取字体度量（物理像素）
    ///
    /// - Returns: (cellWidth, cellHeight, lineHeight)，失败返回 nil
    func getFontMetrics() -> (cellWidth: Float, cellHeight: Float, lineHeight: Float)? {
        guard let handle = handle else { return nil }
        var metrics = SugarloafFontMetrics(cell_width: 0, cell_height: 0, line_height: 0)
        let success = terminal_pool_get_font_metrics(handle, &metrics)
        guard success else { return nil }
        return (metrics.cell_width, metrics.cell_height, metrics.line_height)
    }

    // MARK: - Mouse Tracking

    /// 检查终端是否启用了鼠标追踪模式（SGR 1006）
    func hasMouseTrackingMode(terminalId: Int) -> Bool {
        guard let handle = handle else { return false }
        return terminal_pool_has_mouse_tracking_mode(handle, terminalId)
    }

    /// 发送 SGR 格式的鼠标报告
    ///
    /// - Parameters:
    ///   - button: 0=左键, 1=中键, 2=右键, 64=滚动上, 65=滚动下
    ///   - col: 列（1-based）
    ///   - row: 行（1-based）
    ///   - pressed: true=按下, false=释放
    @discardableResult
    func sendMouseSGR(terminalId: Int, button: UInt8, col: UInt16, row: UInt16, pressed: Bool) -> Bool {
        guard let handle = handle else { return false }
        return terminal_pool_send_mouse_sgr(handle, terminalId, button, col, row, pressed)
    }

    // MARK: - Paste Mode

    /// 检查终端是否启用了 Bracketed Paste Mode
    func isBracketedPasteEnabled(terminalId: Int) -> Bool {
        guard let handle = handle else { return false }
        return terminal_pool_is_bracketed_paste_enabled(handle, terminalId)
    }
}
