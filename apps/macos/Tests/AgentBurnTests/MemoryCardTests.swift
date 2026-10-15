import Foundation
import SwiftUI
import Testing

@testable import AgentBurn

@Test func memoryConsumersGroupHelpersUnderTheirOutermostApp() {
  let chrome = "/Applications/Google Chrome.app"
  let processes = [
    MemoryConsumers.Process(path: "\(chrome)/Contents/MacOS/Google Chrome", footprint: 900),
    MemoryConsumers.Process(
      path:
        "\(chrome)/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)",
      footprint: 2_100),
    MemoryConsumers.Process(path: "/opt/homebrew/bin/node", footprint: 1_500),
    MemoryConsumers.Process(path: "/usr/local/bin/node", footprint: 400),
    MemoryConsumers.Process(path: "/Applications/Xcode.app/Contents/MacOS/Xcode", footprint: 0),
  ]

  let top = MemoryConsumers.top(processes, limit: 5)

  #expect(top.map(\.name) == ["Google Chrome", "node", "node"])
  #expect(
    top[0] == MemoryConsumer(id: chrome, name: "Google Chrome", footprint: 3_000, processes: 2))
  #expect(top[0].bundlePath == chrome)
  #expect(top[1].bundlePath == nil, "A bare executable has no app to quit")
  #expect(top[1].id == "/opt/homebrew/bin/node", "Same-named executables stay apart")
}

@Test func memoryConsumersKeepTheLargestFirstUpToTheLimit() {
  let processes = (1...8).map {
    MemoryConsumers.Process(
      path: "/Applications/App\($0).app/Contents/MacOS/App", footprint: UInt64($0))
  }
  #expect(
    MemoryConsumers.top(processes, limit: 5).map(\.name) == [
      "App8", "App7", "App6", "App5", "App4",
    ])
}

@Test func memoryBreakdownMatchesActivityMonitor() {
  let page: UInt64 = 16_384
  let breakdown = MemoryBreakdown(
    total: 32 * 1_073_741_824, pageSize: page, anonymous: 1_000_000, purgeable: 50_000,
    wired: 200_000, compressor: 100_000, external: 300_000)

  #expect(breakdown.app == Int64(950_000 * page), "Purgeable memory is cache, not app memory")
  #expect(breakdown.wired == Int64(200_000 * page))
  #expect(breakdown.compressed == Int64(100_000 * page))
  #expect(breakdown.cached == Int64(350_000 * page))
  #expect(breakdown.used == breakdown.app + breakdown.wired + breakdown.compressed)
  #expect(breakdown.available == breakdown.total - breakdown.used)
  #expect(breakdown.free == breakdown.available - breakdown.cached)
}

@Test func memoryBreakdownNeverGoesNegative() {
  let breakdown = MemoryBreakdown(total: 100, app: 80, wired: 30, compressed: 10, cached: 50)
  #expect(breakdown.available == 0)
  #expect(breakdown.free == 0)
}

private let chromePath = "/Applications/Google Chrome.app"
private let snapshot = MemorySnapshot(
  breakdown: MemoryBreakdown(
    total: 34_359_738_368, app: 15_000_000_000, wired: 3_400_000_000,
    compressed: 1_900_000_000, cached: 4_000_000_000),
  pressure: .normal,
  consumers: [
    MemoryConsumer(id: chromePath, name: "Google Chrome", footprint: 7_000_000_000, processes: 47),
    MemoryConsumer(
      id: "/opt/homebrew/bin/node", name: "node", footprint: 560_000_000, processes: 3),
  ])

@MainActor private final class FakeApps {
  var running: Set<String> = [chromePath]
  private(set) var quit: [String] = []
  var apps: RunningApps {
    RunningApps(canQuit: { self.running.contains($0) }, quit: { self.quit.append($0) })
  }
}

@MainActor private func memoryController(
  apps: FakeApps, reads: @escaping @Sendable () async -> MemorySnapshot? = { snapshot }
) -> (NotchController, String) {
  let suite = "AgentBurn.memory.card.\(UUID().uuidString)"
  let appearance = AppAppearance(defaults: UserDefaults(suiteName: suite)!, applyPolicy: { _ in })
  appearance.showsMemory = true
  let controller = NotchController(
    appearance: appearance,
    readMemory: { MemoryStats(available: 9_000_000_000, total: 34_359_738_368, pressure: .normal) },
    readSnapshot: reads, apps: apps.apps)
  controller.refreshMemory()
  return (controller, suite)
}

@MainActor private func waitFor(_ condition: () -> Bool) async {
  for _ in 0..<100 where !condition() { try? await Task.sleep(for: .milliseconds(10)) }
}

@Test @MainActor func memoryCardReadsOnlyWhileOpenAndSurvivesRingChanges() async {
  let apps = FakeApps()
  let (controller, suite) = memoryController(apps: apps)
  defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
  #expect(controller.memorySnapshot == nil, "No process walk until the card opens")

  controller.open(NotchModel.memoryCard)
  await waitFor { controller.memorySnapshot != nil }
  #expect(controller.memorySnapshot == snapshot)

  controller.ringsChanged(["codex"])
  #expect(controller.openAgent == NotchModel.memoryCard, "The memory card is not an agent card")

  controller.hideMemory()
  #expect(controller.openAgent == nil, "Hiding the memory ring closes its card")
}

@Test @MainActor func memoryCardQuitsOnlyRunningApps() {
  let apps = FakeApps()
  let (controller, suite) = memoryController(apps: apps)
  defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
  let chrome = snapshot.consumers[0]
  let node = snapshot.consumers[1]

  #expect(controller.canQuit(chrome))
  #expect(!controller.canQuit(node), "A bare executable has no app to quit")
  controller.quit(node)
  controller.quit(chrome)
  #expect(apps.quit == [chromePath])
  #expect(controller.quitting == [chromePath])

  apps.running = []
  #expect(!controller.canQuit(chrome))
}

@Test @MainActor func memoryCardForgetsQuitAppsOnceTheyAreGone() async {
  let apps = FakeApps()
  let after = MemorySnapshot(
    breakdown: snapshot.breakdown, pressure: .normal, consumers: [snapshot.consumers[1]])
  let (controller, suite) = memoryController(apps: apps, reads: { after })
  defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
  controller.quit(snapshot.consumers[0])
  controller.open(NotchModel.memoryCard)
  await waitFor { controller.memorySnapshot != nil }
  #expect(controller.quitting.isEmpty)
}

@Test @MainActor func memoryCardContentFitsItsHeightAndThePanel() throws {
  let width = NotchModel.cardWidth - 2 * NotchModel.cardPadding
  let rows = MemoryCardRows(
    snapshot: MemorySnapshot(
      breakdown: snapshot.breakdown, pressure: .warning,
      consumers: Array(repeating: snapshot.consumers, count: 3).flatMap { $0 }.prefix(5)
        .enumerated().map {
          MemoryConsumer(
            id: "\($0.element.id)\($0.offset)", name: $0.element.name,
            footprint: $0.element.footprint, processes: $0.element.processes)
        }),
    quitting: [], canQuit: { _ in true }, onQuit: { _ in })
  let renderer = ImageRenderer(
    content: rows.frame(width: width).fixedSize(horizontal: false, vertical: true))
  let height = try #require(renderer.nsImage?.size.height)
  let room = NotchModel.memoryCardHeight - 2 * NotchModel.cardPadding
  #expect(height <= room + 0.5, "content \(height) pt, room \(room) pt")
  #expect(height >= room - 4, "content \(height) pt leaves the card mostly empty")
  #expect(NotchModel.memoryCardHeight <= NotchModel.largestCardHeight)
}

@Test func memoryLabelsCountInBinaryUnitsLikeActivityMonitor() {
  let us = Locale(identifier: "en_US")
  let gib: Int64 = 1_073_741_824
  #expect(MemoryStats.label(bytes: 128 * gib, locale: us) == "128 GB")
  #expect(MemoryStats.label(bytes: 9_210_000_000, locale: us) == "8.6 GB")
  #expect(MemoryStats.label(bytes: 560 * 1_048_576, locale: us) == "560 MB")
  #expect(MemoryStats.label(bytes: gib - 1_048_576, locale: us) == "1.0 GB")
}

@Test func memoryLabelsSwitchToGigabytesBeforeAThousandMegabytes() {
  let us = Locale(identifier: "en_US")
  #expect(MemoryStats.label(bytes: 999 * 1_048_576, locale: us) == "999 MB")
  #expect(MemoryStats.label(bytes: 1_001 * 1_048_576, locale: us) == "1.0 GB")
}
