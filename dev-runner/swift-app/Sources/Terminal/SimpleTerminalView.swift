//
//  SimpleTerminalView.swift
//  DevRunner
//
//  简化版终端 Metal 渲染视图
//  只读显示，不处理键盘输入
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
