import SwiftUI

@main
struct DevRunnerApp: App {
    @StateObject private var runner = DevRunner.shared

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(runner)
        }
        .windowStyle(.automatic)
        .defaultSize(width: 1100, height: 650)
    }
}
