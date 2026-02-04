//
//  SimpleTerminalView.swift
//  DevRunner
//
//  简化版终端 Metal 渲染视图
//  支持文本选中、复制粘贴
//

import SwiftUI
import AppKit
import Metal

// MARK: - SwiftUI Wrapper

struct SimpleTerminalView: NSViewRepresentable {

    /// 工作目录
    let workingDirectory: String

    /// 命令执行回调
    var onReady: ((SimpleTerminalController) -> Void)?

    func makeNSView(context: Context) -> SimpleTerminalMetalView {
        let view = SimpleTerminalMetalView()
        view.workingDirectory = workingDirectory
        view.onReady = onReady
        return view
    }

    func updateNSView(_ nsView: SimpleTerminalMetalView, context: Context) {
        // 触发布局更新
        nsView.needsLayout = true
    }
}

// MARK: - Terminal Controller

/// 终端控制器，用于外部发送命令
class SimpleTerminalController {
    weak var metalView: SimpleTerminalMetalView?

    /// 发送命令
    func sendCommand(_ command: String) {
        metalView?.sendCommand(command)
    }

    /// 发送中断 (Ctrl+C)
    func sendInterrupt() {
        metalView?.sendInterrupt()
    }

    /// 清屏
    func clear() {
        metalView?.sendCommand("clear")
    }
}

// MARK: - Metal View

class SimpleTerminalMetalView: NSView {

    /// 工作目录
    var workingDirectory: String = ""

    /// 就绪回调
    var onReady: ((SimpleTerminalController) -> Void)?

    /// 终端池
    private var terminalPool: SimpleTerminalPoolWrapper?

    /// 渲染调度器
    private var renderScheduler: SimpleRenderScheduler?

    /// 控制器
    private let controller = SimpleTerminalController()

    /// 是否已初始化
    private var isInitialized = false

    /// 布局缓存
    private var lastLayoutHash: Int = 0

    // MARK: - Selection State

    /// 是否正在拖拽选中
    private var isDraggingSelection = false

    /// 选中起点（绝对行号）
    private var selectionStartRow: Int64 = 0
    private var selectionStartCol: Int = 0

    /// 缓存的字体度量（逻辑像素）
    private var cachedCellWidth: CGFloat = 0
    private var cachedLineHeight: CGFloat = 0

    // MARK: - Initialization

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        commonInit()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        commonInit()
    }

    override func makeBackingLayer() -> CALayer {
        let metalLayer = CAMetalLayer()
        metalLayer.device = MTLCreateSystemDefaultDevice()
        metalLayer.pixelFormat = .bgra8Unorm
        metalLayer.framebufferOnly = false
        return metalLayer
    }

    private func commonInit() {
        wantsLayer = true
        layer?.contentsScale = NSScreen.main?.backingScaleFactor ?? 2.0
        layer?.isOpaque = false
        controller.metalView = self
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()

        if window != nil {
            // 监听屏幕切换
            NotificationCenter.default.addObserver(
                self,
                selector: #selector(windowDidChangeScreen),
                name: NSWindow.didChangeScreenNotification,
                object: window
            )

            // 延迟初始化确保布局完成
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
                self?.initialize()
            }
        } else {
            NotificationCenter.default.removeObserver(self)
        }
    }

    @objc private func windowDidChangeScreen() {
        guard let window = window else { return }

        let newScale = window.screen?.backingScaleFactor ?? window.backingScaleFactor
        let currentScale = layer?.contentsScale ?? 2.0

        if abs(newScale - currentScale) > 0.01 {
            layer?.contentsScale = newScale
            terminalPool?.setScale(Float(newScale))
            updateFontMetrics()
            needsLayout = true
        }
    }

    private func initialize() {
        guard !isInitialized else { return }
        guard window != nil else { return }
        guard bounds.width > 0 && bounds.height > 0 else { return }

        isInitialized = true
        initializeTerminal()
    }

    private func initializeTerminal() {
        guard let window = window else {
            print("[SimpleTerminal] initializeTerminal: no window")
            return
        }

        let viewPointer = Unmanaged.passUnretained(self).toOpaque()
        let scale = window.screen?.backingScaleFactor ?? window.backingScaleFactor

        print("[SimpleTerminal] initializeTerminal: bounds=\(bounds), scale=\(scale)")

        // 创建终端池
        terminalPool = SimpleTerminalPoolWrapper(
            windowHandle: viewPointer,
            width: Float(bounds.width),
            height: Float(bounds.height),
            scale: Float(scale),
            fontSize: 13.0
        )

        guard let pool = terminalPool else {
            print("[SimpleTerminal] initializeTerminal: failed to create pool")
            return
        }

        // 创建终端
        let cwd = workingDirectory.isEmpty ? FileManager.default.currentDirectoryPath : workingDirectory
        let termId = pool.createTerminal(cwd: cwd)
        print("[SimpleTerminal] initializeTerminal: created terminal \(termId) at \(cwd)")

        // 创建渲染调度器并绑定
        renderScheduler = SimpleRenderScheduler()
        renderScheduler?.bind(to: pool)
        let started = renderScheduler?.start() ?? false
        print("[SimpleTerminal] initializeTerminal: scheduler started=\(started)")

        // 初始布局同步
        syncLayout()

        // 缓存字体度量
        updateFontMetrics()

        // 通知就绪
        print("[SimpleTerminal] initializeTerminal: calling onReady")
        onReady?(controller)
    }

    override func layout() {
        super.layout()

        guard isInitialized, let pool = terminalPool else { return }

        let scale = window?.screen?.backingScaleFactor ?? window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2.0

        if bounds.width > 0 && bounds.height > 0 {
            pool.resizeSugarloaf(width: Float(bounds.width), height: Float(bounds.height))
            syncLayout()
        }
    }

    private func syncLayout() {
        print("[SimpleTerminal] syncLayout: isInit=\(isInitialized) pool=\(terminalPool != nil) termId=\(terminalPool?.terminalId ?? -1)")
        guard isInitialized,
              let pool = terminalPool,
              pool.terminalId >= 0 else {
            print("[SimpleTerminal] syncLayout: guard failed")
            return
        }

        // 计算布局 hash
        let currentHash = Int(bounds.width * 100) ^ Int(bounds.height * 100)
        print("[SimpleTerminal] syncLayout: hash=\(currentHash) lastHash=\(lastLayoutHash)")
        if currentHash == lastLayoutHash {
            print("[SimpleTerminal] syncLayout: hash unchanged, skip")
            return
        }
        lastLayoutHash = currentHash

        // 设置布局（整个区域）
        print("[SimpleTerminal] syncLayout: setting layout bounds=\(bounds)")
        pool.setRenderLayout(
            x: 0,
            y: 0,
            width: Float(bounds.width),
            height: Float(bounds.height),
            containerHeight: Float(bounds.height)
        )

        // 请求渲染
        renderScheduler?.requestRender()
        print("[SimpleTerminal] syncLayout: done")
    }

    // MARK: - Public API

    /// 发送命令
    func sendCommand(_ command: String) {
        print("[SimpleTerminal] sendCommand: '\(command)' pool=\(terminalPool != nil) termId=\(terminalPool?.terminalId ?? -1)")
        terminalPool?.sendCommand(command)
    }

    /// 发送中断
    func sendInterrupt() {
        print("[SimpleTerminal] sendInterrupt")
        terminalPool?.sendInterrupt()
    }

    // MARK: - First Responder

    override var acceptsFirstResponder: Bool { true }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    // MARK: - Coordinate Conversion

    /// 更新缓存的字体度量
    private func updateFontMetrics() {
        guard let pool = terminalPool,
              let metrics = pool.getFontMetrics() else { return }
        let scale = window?.screen?.backingScaleFactor ?? window?.backingScaleFactor ?? 2.0
        cachedCellWidth = CGFloat(metrics.cellWidth) / scale
        cachedLineHeight = CGFloat(metrics.lineHeight) / scale
    }

    /// 屏幕坐标转网格坐标（终端 row/col）
    private func screenToGrid(location: CGPoint) -> (col: Int, row: Int) {
        guard cachedCellWidth > 0, cachedLineHeight > 0 else {
            return (0, 0)
        }

        // NSView 坐标: 原点左下，Y 向上
        // 终端坐标: row=0 在顶部
        let yFromTop = bounds.height - location.y
        let col = max(0, Int(location.x / cachedCellWidth))
        let row = max(0, Int(yFromTop / cachedLineHeight))
        return (col, row)
    }

    // MARK: - Mouse Events

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)

        guard let pool = terminalPool, pool.terminalId >= 0 else {
            super.mouseDown(with: event)
            return
        }

        let terminalId = pool.terminalId
        let location = convert(event.locationInWindow, from: nil)
        let grid = screenToGrid(location: location)

        // 鼠标追踪模式（vim, tmux 等）
        if pool.hasMouseTrackingMode(terminalId: terminalId) {
            let button: UInt8
            switch event.buttonNumber {
            case 0: button = 0  // 左键
            case 1: button = 2  // macOS 右键 → SGR 右键
            case 2: button = 1  // macOS 中键 → SGR 中键
            default: return
            }
            _ = pool.sendMouseSGR(
                terminalId: terminalId,
                button: button,
                col: UInt16(grid.col + 1),
                row: UInt16(grid.row + 1),
                pressed: true
            )
            return
        }

        // 单击时先清除旧选区
        pool.clearSelection(terminalId: terminalId)

        // 坐标转换: screen → absolute
        guard let (absoluteRow, col) = pool.screenToAbsolute(
            terminalId: terminalId,
            screenRow: grid.row,
            screenCol: grid.col
        ) else {
            super.mouseDown(with: event)
            return
        }

        // 记录起点
        selectionStartRow = absoluteRow
        selectionStartCol = col

        // 设置初始选区（起点=终点）
        pool.setSelection(
            terminalId: terminalId,
            startAbsoluteRow: absoluteRow,
            startCol: col,
            endAbsoluteRow: absoluteRow,
            endCol: col
        )

        renderScheduler?.requestRender()
        isDraggingSelection = true
    }

    override func mouseDragged(with event: NSEvent) {
        guard isDraggingSelection,
              let pool = terminalPool, pool.terminalId >= 0 else {
            super.mouseDragged(with: event)
            return
        }

        let terminalId = pool.terminalId
        let location = convert(event.locationInWindow, from: nil)
        let grid = screenToGrid(location: location)

        guard let (absoluteRow, col) = pool.screenToAbsolute(
            terminalId: terminalId,
            screenRow: grid.row,
            screenCol: grid.col
        ) else {
            return
        }

        // 更新选区（起点不变，终点跟随鼠标）
        pool.setSelection(
            terminalId: terminalId,
            startAbsoluteRow: selectionStartRow,
            startCol: selectionStartCol,
            endAbsoluteRow: absoluteRow,
            endCol: col
        )

        renderScheduler?.requestRender()
    }

    override func mouseUp(with event: NSEvent) {
        // 鼠标追踪模式：发送 release
        if let pool = terminalPool, pool.terminalId >= 0 {
            let terminalId = pool.terminalId
            if pool.hasMouseTrackingMode(terminalId: terminalId) {
                let location = convert(event.locationInWindow, from: nil)
                let grid = screenToGrid(location: location)
                let button: UInt8
                switch event.buttonNumber {
                case 0: button = 0
                case 1: button = 2
                case 2: button = 1
                default: return
                }
                _ = pool.sendMouseSGR(
                    terminalId: terminalId,
                    button: button,
                    col: UInt16(grid.col + 1),
                    row: UInt16(grid.row + 1),
                    pressed: false
                )
                return
            }
        }

        guard isDraggingSelection,
              let pool = terminalPool, pool.terminalId >= 0 else {
            super.mouseUp(with: event)
            return
        }

        // 完成选区（全空白时 Rust 自动清除）
        _ = pool.finalizeSelection(terminalId: pool.terminalId)
        renderScheduler?.requestRender()
        isDraggingSelection = false
    }

    // MARK: - Keyboard Events

    override func keyDown(with event: NSEvent) {
        guard let pool = terminalPool, pool.terminalId >= 0 else {
            super.keyDown(with: event)
            return
        }

        let terminalId = pool.terminalId
        let flags = event.modifierFlags

        // Cmd+C 复制
        if flags.contains(.command), event.charactersIgnoringModifiers == "c" {
            if let text = pool.getSelectionText(terminalId: terminalId) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
                return
            }
            // 没有选区，不拦截
            super.keyDown(with: event)
            return
        }

        // Cmd+V 粘贴
        if flags.contains(.command), event.charactersIgnoringModifiers == "v" {
            if let text = NSPasteboard.general.string(forType: .string) {
                if pool.isBracketedPasteEnabled(terminalId: terminalId) {
                    let wrapped = "\u{1B}[200~" + text + "\u{1B}[201~"
                    pool.writeInput(terminalId: terminalId, data: wrapped)
                } else {
                    pool.writeInput(terminalId: terminalId, data: text)
                }
            }
            return
        }

        super.keyDown(with: event)
    }

    // MARK: - Context Menu

    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        menu.allowsContextMenuPlugIns = false

        guard let pool = terminalPool, pool.terminalId >= 0 else {
            menu.addItem(withTitle: "粘贴", action: #selector(pasteFromClipboard(_:)), keyEquivalent: "v")
            return menu
        }

        // 有选中文本时显示复制选项
        if let text = pool.getSelectionText(terminalId: pool.terminalId),
           !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            menu.addItem(withTitle: "复制", action: #selector(copySelection(_:)), keyEquivalent: "c")
            menu.addItem(NSMenuItem.separator())
        }

        menu.addItem(withTitle: "粘贴", action: #selector(pasteFromClipboard(_:)), keyEquivalent: "v")
        return menu
    }

    @objc private func copySelection(_ sender: Any?) {
        guard let pool = terminalPool, pool.terminalId >= 0,
              let text = pool.getSelectionText(terminalId: pool.terminalId) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc private func pasteFromClipboard(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string),
              let pool = terminalPool, pool.terminalId >= 0 else { return }
        if pool.isBracketedPasteEnabled(terminalId: pool.terminalId) {
            let wrapped = "\u{1B}[200~" + text + "\u{1B}[201~"
            pool.writeInput(terminalId: pool.terminalId, data: wrapped)
        } else {
            pool.writeInput(terminalId: pool.terminalId, data: text)
        }
    }

    // MARK: - Scrolling

    override func scrollWheel(with event: NSEvent) {
        guard let pool = terminalPool else {
            super.scrollWheel(with: event)
            return
        }

        // 触控板滚动
        let delta = event.scrollingDeltaY
        if abs(delta) > 0.1 {
            let lines = Int32(delta / 3.0)
            if lines != 0 {
                pool.scroll(delta: lines)
                renderScheduler?.requestRender()
            }
        }
    }

    deinit {
        renderScheduler?.stop()
        terminalPool?.closeTerminal()
        NotificationCenter.default.removeObserver(self)
    }
}

// MARK: - Preview

#Preview {
    SimpleTerminalView(workingDirectory: "/tmp") { controller in
        // 测试命令
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            controller.sendCommand("echo 'Hello from DevRunner!'")
        }
    }
    .frame(width: 800, height: 400)
}
