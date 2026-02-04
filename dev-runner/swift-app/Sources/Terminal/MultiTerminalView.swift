//
//  MultiTerminalView.swift
//  DevRunner
//
//  支持多 Tab 的终端视图
//

import SwiftUI
import AppKit
import Metal

// MARK: - SwiftUI Wrapper

struct MultiTerminalView: NSViewRepresentable {

    /// 工作目录
    let workingDirectory: String

    /// Tab 管理器
    @ObservedObject var tabManager: TerminalTabManager

    /// 就绪回调
    var onReady: ((MultiTerminalController) -> Void)?

    func makeNSView(context: Context) -> MultiTerminalMetalView {
        let view = MultiTerminalMetalView()
        view.workingDirectory = workingDirectory
        view.tabManager = tabManager
        view.onReady = onReady
        return view
    }

    func updateNSView(_ nsView: MultiTerminalMetalView, context: Context) {
        // 当选中的 tab 改变时，更新显示的终端
        nsView.updateSelectedTerminal()
    }
}

// MARK: - Controller

class MultiTerminalController {
    weak var metalView: MultiTerminalMetalView?
    weak var tabManager: TerminalTabManager?

    /// 获取终端池（用于向特定终端发送命令）
    var terminalPool: SimpleTerminalPoolWrapper? {
        metalView?.terminalPool
    }

    /// 向当前选中的终端发送命令
    func sendCommand(_ command: String) {
        metalView?.sendCommand(command)
    }

    /// 向指定终端发送命令
    func sendCommand(_ command: String, to terminalId: Int) {
        metalView?.terminalPool?.sendCommand(command, to: terminalId)
    }

    /// 向当前选中的终端发送中断
    func sendInterrupt() {
        metalView?.sendInterrupt()
    }

    /// 向指定终端发送中断
    func sendInterrupt(to terminalId: Int) {
        metalView?.terminalPool?.sendInterrupt(to: terminalId)
    }

    /// 清屏
    func clear() {
        sendCommand("clear")
    }

    /// 创建新 Tab
    @discardableResult
    func createTab(title: String? = nil) -> TerminalTab? {
        guard let metalView = metalView else { return nil }
        return tabManager?.createTab(cwd: metalView.workingDirectory, title: title)
    }
}

// MARK: - Metal View

class MultiTerminalMetalView: NSView {

    var workingDirectory: String = ""
    var tabManager: TerminalTabManager?
    var onReady: ((MultiTerminalController) -> Void)?

    private(set) var terminalPool: SimpleTerminalPoolWrapper?
    private var renderScheduler: SimpleRenderScheduler?
    private let controller = MultiTerminalController()
    private var isInitialized = false
    private var lastLayoutHash: Int = 0

    // MARK: - Selection State

    /// 是否正在拖拽选中
    private var isDraggingSelection = false

    /// mouseDown 时快照的 terminalId（避免 tab 切换导致坐标错位）
    private var selectionTerminalId: Int?

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
            NotificationCenter.default.addObserver(
                self,
                selector: #selector(windowDidChangeScreen),
                name: NSWindow.didChangeScreenNotification,
                object: window
            )

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
        initializePool()
    }

    private func initializePool() {
        guard let window = window else { return }

        let viewPointer = Unmanaged.passUnretained(self).toOpaque()
        let scale = window.screen?.backingScaleFactor ?? window.backingScaleFactor

        // 创建终端池
        terminalPool = SimpleTerminalPoolWrapper(
            windowHandle: viewPointer,
            width: Float(bounds.width),
            height: Float(bounds.height),
            scale: Float(scale),
            fontSize: 13.0
        )

        guard let pool = terminalPool else { return }

        // 设置 TabManager 的 pool 引用
        tabManager?.setPool(pool)
        controller.tabManager = tabManager

        // 创建渲染调度器
        renderScheduler = SimpleRenderScheduler()
        renderScheduler?.bind(to: pool)
        renderScheduler?.start()

        // 创建第一个 Tab
        let cwd = workingDirectory.isEmpty ? FileManager.default.currentDirectoryPath : workingDirectory
        tabManager?.createTab(cwd: cwd, title: "zsh")

        // 同步布局
        syncLayout()

        // 缓存字体度量
        updateFontMetrics()

        // 通知就绪
        onReady?(controller)
    }

    override func layout() {
        super.layout()

        guard isInitialized, let pool = terminalPool else { return }

        if bounds.width > 0 && bounds.height > 0 {
            pool.resizeSugarloaf(width: Float(bounds.width), height: Float(bounds.height))
            syncLayout()
        }
    }

    /// 当选中的终端改变时调用
    func updateSelectedTerminal() {
        lastLayoutHash = 0  // 强制刷新布局
        syncLayout()
    }

    private func syncLayout() {
        guard isInitialized,
              let pool = terminalPool,
              let selectedTerminalId = tabManager?.selectedTerminalId,
              selectedTerminalId >= 0 else {
            return
        }

        // 计算布局 hash（包含选中的终端 ID）
        let currentHash = Int(bounds.width * 100) ^ Int(bounds.height * 100) ^ selectedTerminalId
        if currentHash == lastLayoutHash {
            return
        }
        lastLayoutHash = currentHash

        // 设置布局 - 只显示选中的终端
        pool.setRenderLayout(
            terminalId: selectedTerminalId,
            x: 0,
            y: 0,
            width: Float(bounds.width),
            height: Float(bounds.height),
            containerHeight: Float(bounds.height)
        )

        renderScheduler?.requestRender()
    }

    // MARK: - Public API

    func sendCommand(_ command: String) {
        guard let pool = terminalPool,
              let terminalId = tabManager?.selectedTerminalId,
              terminalId >= 0 else { return }
        pool.sendCommand(command, to: terminalId)
    }

    func sendInterrupt() {
        guard let pool = terminalPool,
              let terminalId = tabManager?.selectedTerminalId,
              terminalId >= 0 else { return }
        pool.sendInterrupt(to: terminalId)
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

    /// 屏幕坐标转网格坐标
    private func screenToGrid(location: CGPoint) -> (col: Int, row: Int) {
        guard cachedCellWidth > 0, cachedLineHeight > 0 else {
            return (0, 0)
        }

        let yFromTop = bounds.height - location.y
        let col = max(0, Int(location.x / cachedCellWidth))
        let row = max(0, Int(yFromTop / cachedLineHeight))
        return (col, row)
    }

    // MARK: - Mouse Events

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)

        guard let pool = terminalPool,
              let terminalId = tabManager?.selectedTerminalId,
              terminalId >= 0 else {
            super.mouseDown(with: event)
            return
        }

        let location = convert(event.locationInWindow, from: nil)
        let grid = screenToGrid(location: location)

        // 鼠标追踪模式（vim, tmux 等）
        if pool.hasMouseTrackingMode(terminalId: terminalId) {
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
                pressed: true
            )
            return
        }

        // 单击时先清除旧选区
        pool.clearSelection(terminalId: terminalId)

        guard let (absoluteRow, col) = pool.screenToAbsolute(
            terminalId: terminalId,
            screenRow: grid.row,
            screenCol: grid.col
        ) else {
            super.mouseDown(with: event)
            return
        }

        // 快照 terminalId（避免 tab 切换导致坐标错位）
        selectionTerminalId = terminalId
        selectionStartRow = absoluteRow
        selectionStartCol = col

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
        // 使用 mouseDown 时快照的 terminalId
        guard isDraggingSelection,
              let terminalId = selectionTerminalId,
              let pool = terminalPool else {
            super.mouseDragged(with: event)
            return
        }

        let location = convert(event.locationInWindow, from: nil)
        let grid = screenToGrid(location: location)

        guard let (absoluteRow, col) = pool.screenToAbsolute(
            terminalId: terminalId,
            screenRow: grid.row,
            screenCol: grid.col
        ) else {
            return
        }

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
        if let pool = terminalPool,
           let terminalId = tabManager?.selectedTerminalId,
           terminalId >= 0,
           pool.hasMouseTrackingMode(terminalId: terminalId) {
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

        guard isDraggingSelection,
              let terminalId = selectionTerminalId,
              let pool = terminalPool else {
            super.mouseUp(with: event)
            return
        }

        _ = pool.finalizeSelection(terminalId: terminalId)
        renderScheduler?.requestRender()
        isDraggingSelection = false
        // 不清除 selectionTerminalId，保留给 Cmd+C
    }

    // MARK: - Keyboard Events

    override func keyDown(with event: NSEvent) {
        let flags = event.modifierFlags

        // Cmd+C 复制
        if flags.contains(.command), event.charactersIgnoringModifiers == "c" {
            // 优先使用快照的 terminalId（选区所在终端），否则用当前选中
            let terminalId = selectionTerminalId ?? tabManager?.selectedTerminalId
            if let terminalId = terminalId,
               let pool = terminalPool,
               let text = pool.getSelectionText(terminalId: terminalId) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
                return
            }
            super.keyDown(with: event)
            return
        }

        // Cmd+V 粘贴
        if flags.contains(.command), event.charactersIgnoringModifiers == "v" {
            if let text = NSPasteboard.general.string(forType: .string),
               let terminalId = tabManager?.selectedTerminalId,
               let pool = terminalPool {
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

        let terminalId = selectionTerminalId ?? tabManager?.selectedTerminalId
        guard let terminalId = terminalId,
              let pool = terminalPool else {
            menu.addItem(withTitle: "粘贴", action: #selector(pasteFromClipboard(_:)), keyEquivalent: "v")
            return menu
        }

        if let text = pool.getSelectionText(terminalId: terminalId),
           !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            menu.addItem(withTitle: "复制", action: #selector(copySelection(_:)), keyEquivalent: "c")
            menu.addItem(NSMenuItem.separator())
        }

        menu.addItem(withTitle: "粘贴", action: #selector(pasteFromClipboard(_:)), keyEquivalent: "v")
        return menu
    }

    @objc private func copySelection(_ sender: Any?) {
        let terminalId = selectionTerminalId ?? tabManager?.selectedTerminalId
        guard let terminalId = terminalId,
              let pool = terminalPool,
              let text = pool.getSelectionText(terminalId: terminalId) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc private func pasteFromClipboard(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string),
              let terminalId = tabManager?.selectedTerminalId,
              let pool = terminalPool else { return }
        if pool.isBracketedPasteEnabled(terminalId: terminalId) {
            let wrapped = "\u{1B}[200~" + text + "\u{1B}[201~"
            pool.writeInput(terminalId: terminalId, data: wrapped)
        } else {
            pool.writeInput(terminalId: terminalId, data: text)
        }
    }

    // MARK: - Scrolling

    override func scrollWheel(with event: NSEvent) {
        guard let pool = terminalPool,
              let terminalId = tabManager?.selectedTerminalId,
              terminalId >= 0 else {
            super.scrollWheel(with: event)
            return
        }

        let delta = event.scrollingDeltaY
        if abs(delta) > 0.1 {
            let lines = Int32(delta / 3.0)
            if lines != 0 {
                pool.scroll(terminalId: terminalId, delta: lines)
                renderScheduler?.requestRender()
            }
        }
    }

    deinit {
        renderScheduler?.stop()
        NotificationCenter.default.removeObserver(self)
    }
}
