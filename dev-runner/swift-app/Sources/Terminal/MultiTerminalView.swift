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

    /// 向当前选中的终端发送命令
    func sendCommand(_ command: String) {
        metalView?.sendCommand(command)
    }

    /// 向当前选中的终端发送中断
    func sendInterrupt() {
        metalView?.sendInterrupt()
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

    private var terminalPool: SimpleTerminalPoolWrapper?
    private var renderScheduler: SimpleRenderScheduler?
    private let controller = MultiTerminalController()
    private var isInitialized = false
    private var lastLayoutHash: Int = 0

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
                pool.scroll(delta: lines)
                renderScheduler?.requestRender()
            }
        }
    }

    deinit {
        renderScheduler?.stop()
        NotificationCenter.default.removeObserver(self)
    }
}
