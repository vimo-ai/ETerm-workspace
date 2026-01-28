//
//  SimpleRenderScheduler.swift
//  DevRunner
//
//  简化版渲染调度器，封装 Rust CVDisplayLink
//

import Foundation

/// 简化版渲染调度器
///
/// Rust 侧完成整个渲染循环：
/// - bind(to:) 绑定到 TerminalPool
/// - start() 启动 CVDisplayLink
/// - requestRender() 标记需要渲染
class SimpleRenderScheduler {

    /// Rust 侧的 handle
    private var handle: RenderSchedulerHandle?

    /// 是否已启动
    private(set) var isRunning: Bool = false

    // MARK: - Initialization

    init() {
        handle = render_scheduler_create()
    }

    deinit {
        stop()
        if let handle = handle {
            render_scheduler_destroy(handle)
        }
    }

    // MARK: - Configuration

    /// 绑定到 TerminalPool
    ///
    /// 绑定后 RenderScheduler 在 VSync 时自动渲染
    func bind(to pool: SimpleTerminalPoolWrapper) {
        guard let schedulerHandle = handle,
              let poolHandle = pool.poolHandle else {
            return
        }

        render_scheduler_bind_to_pool(schedulerHandle, poolHandle)
    }

    // MARK: - Control

    /// 启动渲染调度器
    @discardableResult
    func start() -> Bool {
        guard let handle = handle else {
            return false
        }

        if isRunning {
            return true
        }

        let success = render_scheduler_start(handle)
        if success {
            isRunning = true
        }

        return success
    }

    /// 停止渲染调度器
    func stop() {
        guard let handle = handle, isRunning else { return }

        render_scheduler_stop(handle)
        isRunning = false
    }

    /// 请求渲染
    func requestRender() {
        guard let handle = handle else { return }
        render_scheduler_request_render(handle)
    }
}
