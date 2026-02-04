//
//  HTTPServer.swift
//  DevRunner
//
//  轻量 HTTP Server，使用 Network.framework
//

import Foundation
import Network

/// HTTP 请求
struct HTTPRequest {
    let method: String
    let path: String
    let query: [String: String]
    let headers: [String: String]
    let body: Data?

    /// 解析 query 参数
    func queryParam(_ name: String) -> String? {
        query[name]
    }

    /// 解析 JSON body
    func jsonBody<T: Decodable>(_ type: T.Type) -> T? {
        guard let body = body else { return nil }
        return try? JSONDecoder().decode(type, from: body)
    }
}

/// HTTP 响应
struct HTTPResponse {
    var status: Int
    var statusText: String
    var headers: [String: String]
    var body: Data?

    init(status: Int = 200, statusText: String = "OK", headers: [String: String] = [:], body: Data? = nil) {
        self.status = status
        self.statusText = statusText
        self.headers = headers
        self.body = body
    }

    /// JSON 响应
    static func json<T: Encodable>(_ value: T, status: Int = 200) -> HTTPResponse {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        guard let data = try? encoder.encode(value) else {
            return .error(500, "JSON encoding failed")
        }
        return HTTPResponse(
            status: status,
            statusText: statusText(for: status),
            headers: ["Content-Type": "application/json"],
            body: data
        )
    }

    /// 原始 JSON 响应（直接透传 JSON 字符串，不再编码）
    static func rawJSON(_ json: String, status: Int = 200) -> HTTPResponse {
        HTTPResponse(
            status: status,
            statusText: statusText(for: status),
            headers: ["Content-Type": "application/json"],
            body: json.data(using: .utf8)
        )
    }

    /// 错误响应
    static func error(_ status: Int, _ message: String) -> HTTPResponse {
        struct ErrorBody: Codable {
            let error: String
        }
        return .json(ErrorBody(error: message), status: status)
    }

    /// 成功响应（无 body）
    static func ok() -> HTTPResponse {
        HTTPResponse(status: 200, statusText: "OK")
    }

    /// 状态文本
    private static func statusText(for code: Int) -> String {
        switch code {
        case 200: return "OK"
        case 201: return "Created"
        case 204: return "No Content"
        case 400: return "Bad Request"
        case 404: return "Not Found"
        case 405: return "Method Not Allowed"
        case 500: return "Internal Server Error"
        default: return "Unknown"
        }
    }

    /// 序列化为 HTTP 报文
    func serialize() -> Data {
        var response = "HTTP/1.1 \(status) \(statusText)\r\n"

        var headers = self.headers
        headers["Connection"] = "close"
        if let body = body {
            headers["Content-Length"] = String(body.count)
        }

        for (key, value) in headers {
            response += "\(key): \(value)\r\n"
        }

        response += "\r\n"

        var data = response.data(using: .utf8) ?? Data()
        if let body = body {
            data.append(body)
        }

        return data
    }
}

/// 路由处理器
typealias RouteHandler = (HTTPRequest) async -> HTTPResponse

/// HTTP Server
final class HTTPServer {

    private let port: UInt16
    private var listener: NWListener?
    private var routes: [(method: String, pattern: String, handler: RouteHandler)] = []
    private let queue = DispatchQueue(label: "ai.vimo.DevRunner.HTTPServer", qos: .userInitiated)

    init(port: UInt16 = 9274) {
        self.port = port
    }

    // MARK: - Routing

    /// 注册路由
    func route(_ method: String, _ pattern: String, handler: @escaping RouteHandler) {
        routes.append((method: method, pattern: pattern, handler: handler))
    }

    /// GET 路由
    func get(_ pattern: String, handler: @escaping RouteHandler) {
        route("GET", pattern, handler: handler)
    }

    /// POST 路由
    func post(_ pattern: String, handler: @escaping RouteHandler) {
        route("POST", pattern, handler: handler)
    }

    /// DELETE 路由
    func delete(_ pattern: String, handler: @escaping RouteHandler) {
        route("DELETE", pattern, handler: handler)
    }

    // MARK: - Lifecycle

    /// 启动服务器
    func start() throws {
        let params = NWParameters.tcp
        params.allowLocalEndpointReuse = true

        listener = try NWListener(using: params, on: NWEndpoint.Port(rawValue: port)!)

        listener?.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                print("[HTTPServer] Listening on port \(self?.port ?? 0)")
            case .failed(let error):
                print("[HTTPServer] Failed: \(error)")
            case .cancelled:
                print("[HTTPServer] Cancelled")
            default:
                break
            }
        }

        listener?.newConnectionHandler = { [weak self] connection in
            self?.handleConnection(connection)
        }

        listener?.start(queue: queue)
    }

    /// 停止服务器
    func stop() {
        listener?.cancel()
        listener = nil
    }

    // MARK: - Connection Handling

    private func handleConnection(_ connection: NWConnection) {
        connection.stateUpdateHandler = { state in
            switch state {
            case .ready:
                self.receiveRequest(connection)
            case .failed(let error):
                print("[HTTPServer] Connection failed: \(error)")
                connection.cancel()
            default:
                break
            }
        }

        connection.start(queue: queue)
    }

    private func receiveRequest(_ connection: NWConnection) {
        // 接收数据（最多 64KB）
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self = self else {
                connection.cancel()
                return
            }

            if let error = error {
                print("[HTTPServer] Receive error: \(error)")
                connection.cancel()
                return
            }

            guard let data = data, !data.isEmpty else {
                if isComplete {
                    connection.cancel()
                }
                return
            }

            // 解析请求
            guard let request = self.parseRequest(data) else {
                self.sendResponse(connection, .error(400, "Invalid request"))
                return
            }

            // 路由匹配
            Task {
                let response = await self.routeRequest(request)
                self.sendResponse(connection, response)
            }
        }
    }

    private func sendResponse(_ connection: NWConnection, _ response: HTTPResponse) {
        let data = response.serialize()
        connection.send(content: data, completion: .contentProcessed { error in
            if let error = error {
                print("[HTTPServer] Send error: \(error)")
            }
            connection.cancel()
        })
    }

    // MARK: - Request Parsing

    private func parseRequest(_ data: Data) -> HTTPRequest? {
        guard let text = String(data: data, encoding: .utf8) else { return nil }

        // 分离 header 和 body
        let parts = text.components(separatedBy: "\r\n\r\n")
        guard !parts.isEmpty else { return nil }

        let headerSection = parts[0]
        let bodyText = parts.count > 1 ? parts[1] : nil

        // 解析请求行
        let lines = headerSection.components(separatedBy: "\r\n")
        guard let requestLine = lines.first else { return nil }

        let requestParts = requestLine.split(separator: " ", maxSplits: 2)
        guard requestParts.count >= 2 else { return nil }

        let method = String(requestParts[0])
        let fullPath = String(requestParts[1])

        // 解析 path 和 query
        let (path, query) = parsePathAndQuery(fullPath)

        // 解析 headers
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            if let colonIndex = line.firstIndex(of: ":") {
                let key = String(line[..<colonIndex]).trimmingCharacters(in: .whitespaces)
                let value = String(line[line.index(after: colonIndex)...]).trimmingCharacters(in: .whitespaces)
                headers[key.lowercased()] = value
            }
        }

        // Body
        let body = bodyText?.data(using: .utf8)

        return HTTPRequest(
            method: method,
            path: path,
            query: query,
            headers: headers,
            body: body
        )
    }

    private func parsePathAndQuery(_ fullPath: String) -> (path: String, query: [String: String]) {
        guard let questionIndex = fullPath.firstIndex(of: "?") else {
            return (fullPath, [:])
        }

        let path = String(fullPath[..<questionIndex])
        let queryString = String(fullPath[fullPath.index(after: questionIndex)...])

        var query: [String: String] = [:]
        for pair in queryString.split(separator: "&") {
            let parts = pair.split(separator: "=", maxSplits: 1)
            if parts.count == 2 {
                let key = String(parts[0]).removingPercentEncoding ?? String(parts[0])
                let value = String(parts[1]).removingPercentEncoding ?? String(parts[1])
                query[key] = value
            } else if parts.count == 1 {
                let key = String(parts[0]).removingPercentEncoding ?? String(parts[0])
                query[key] = ""
            }
        }

        return (path, query)
    }

    // MARK: - Routing

    private func routeRequest(_ request: HTTPRequest) async -> HTTPResponse {
        // CORS 预检
        if request.method == "OPTIONS" {
            var response = HTTPResponse.ok()
            response.headers["Access-Control-Allow-Origin"] = "*"
            response.headers["Access-Control-Allow-Methods"] = "GET, POST, DELETE, OPTIONS"
            response.headers["Access-Control-Allow-Headers"] = "Content-Type"
            return response
        }

        // 匹配路由
        for route in routes {
            if route.method == request.method && matchPattern(route.pattern, request.path) {
                var response = await route.handler(request)
                // 添加 CORS 头
                response.headers["Access-Control-Allow-Origin"] = "*"
                return response
            }
        }

        return .error(404, "Not Found")
    }

    /// 简单的路由匹配（支持前缀匹配）
    private func matchPattern(_ pattern: String, _ path: String) -> Bool {
        // 精确匹配
        if pattern == path {
            return true
        }

        // 前缀匹配（pattern 以 * 结尾）
        if pattern.hasSuffix("*") {
            let prefix = String(pattern.dropLast())
            return path.hasPrefix(prefix)
        }

        return false
    }
}
