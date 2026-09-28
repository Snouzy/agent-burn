import Foundation
import SwiftUI
import Testing

@testable import AgentBurn

@MainActor private func notchStoreFixture() -> (store: UsageStore, suite: String, directory: URL) {
  let suite = "AgentBurn.notch.adapters.\(UUID().uuidString)"
  let defaults = UserDefaults(suiteName: suite)!
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  return (UsageStore(defaults: defaults, storageDirectory: directory), suite, directory)
}

private func agentUsage(_ agent: String) -> AgentUsage {
  AgentUsage(
    agent: agent, totalCost: 0, totalTokens: 0, models: nil, daily: nil, tokenBreakdown: nil)
}

private func harnessReport(
  agent: String, usedPercent: Double, plan: String? = nil, apiEquivalentPerMonth: Double = 0,
  pricePerMonth: Double? = nil, economics: Economics? = nil, topModels: [HarnessModel] = []
) -> HarnessReport {
  HarnessReport(
    agent: agent, plan: plan, liveLimits: true,
    window: QuotaWindow(
      windowMinutes: 10080, usedPercent: usedPercent, elapsedPercent: 20, apiEquivalentSpent: 0),
    apiEquivalentPerMonth: apiEquivalentPerMonth, daily: [], topModels: topModels,
    pricePerMonth: pricePerMonth, economics: economics, estimate: nil, spendMix: nil,
    weeklyTrend: nil, imageGenerations: nil)
}

@Test func notchAgentsKeepTheKnownOrderAndDropThoseWithoutAForecast() {
  let agents = NotchModel.agents(
    known: ["opencode", "claude", "cursor", "codex", "gemini"],
    hasForecast: { $0 != "gemini" })
  #expect(agents == ["codex", "claude", "cursor", "opencode"])
}

@Test func notchSpendReadsLikeAnEstimateAgainstThePlan() {
  let us = Locale(identifier: "en_US")
  #expect(
    NotchModel.spend(apiEquivalent: 5927.22, price: 200, multiple: 29.6, locale: us)
      == .init(spend: "~$5,927", plan: "plan $200", multiple: "×30"))
  #expect(
    NotchModel.spend(apiEquivalent: 42.1, price: 20, multiple: 2.1, locale: us)
      == .init(spend: "~$42.10", plan: "plan $20", multiple: "×2.1"))
  #expect(
    NotchModel.spend(apiEquivalent: 12, price: nil, multiple: nil, locale: us)
      == .init(spend: "~$12.00", plan: nil, multiple: nil))
}

@Test func notchCardGrowsWithItsSpendBlock() {
  let plain = NotchModel.cardHeight(hasSpend: false, modelCount: 0)
  let spend = NotchModel.cardHeight(hasSpend: true, modelCount: 3)
  #expect(spend > plain)
  #expect(NotchModel.cardHeight(hasSpend: true, modelCount: 5) == spend)
}

@Test func notchHeightFitsItsRings() {
  #expect(NotchModel.notchHeight(ringCount: 3) > NotchModel.notchHeight(ringCount: 1))
}

@Test @MainActor func notchSwitchDefaultsOffAndPersists() throws {
  let suite = "AgentBurn.notch.\(UUID().uuidString)"
  let defaults = try #require(UserDefaults(suiteName: suite))
  defer { defaults.removePersistentDomain(forName: suite) }
  let appearance = AppAppearance(defaults: defaults, applyPolicy: { _ in })
  #expect(!appearance.showsNotch)
  appearance.showsNotch = true
  #expect(AppAppearance(defaults: defaults, applyPolicy: { _ in }).showsNotch)
}

@Test func notchShapeFillsItsRectAndMeetsTheBezelAtBothEnds() {
  let rect = CGRect(x: 0, y: 0, width: NotchModel.notchDepth, height: 300)
  let path = NotchShape().path(in: rect)
  #expect(abs(path.boundingRect.width - rect.width) < 0.5)
  #expect(abs(path.boundingRect.height - rect.height) < 0.5)
  #expect(path.contains(CGPoint(x: rect.maxX - 1, y: 20)))
  #expect(!path.contains(CGPoint(x: rect.maxX - 5, y: 1)))
  #expect(!path.contains(CGPoint(x: 1, y: 1)))
  #expect(path.contains(CGPoint(x: 5, y: rect.midY)))
}

private func codexCardData(hasSpend: Bool = true) -> NotchCardData {
  let window = QuotaWindow(
    windowMinutes: 10080, usedPercent: 36, elapsedPercent: 30, apiEquivalentSpent: 0)
  let forecast = Forecast(window: window, observedAt: .now, isLive: true)
  return NotchCardData(
    agent: "codex", title: "Codex", plan: "Pro", forecast: forecast, samples: [], stale: false,
    spend: hasSpend ? NotchModel.spend(apiEquivalent: 5927, price: 200, multiple: 29.6) : nil,
    models: hasSpend
      ? [("gpt-5.6-sol", "$3,120"), ("gpt-5.5", "$1,980"), ("gpt-5.4-mini", "$212")] : [],
    style: .weekly, range: .rte)
}

@MainActor private func renderedHeight(_ view: some View) throws -> CGFloat {
  let width = NotchModel.cardWidth - 2 * NotchModel.cardPadding
  let renderer = ImageRenderer(
    content: view.frame(width: width).fixedSize(horizontal: false, vertical: true))
  return try #require(renderer.nsImage?.size.height)
}

@Test @MainActor func notchCardRenders() throws {
  let renderer = ImageRenderer(content: NotchCard(data: codexCardData()))
  #expect(renderer.cgImage != nil)
}

@Test @MainActor func notchCardRowsMatchTheirDeclaredHeight() throws {
  for data in [codexCardData(), codexCardData(hasSpend: false)] {
    let rows = NotchCardRows(data: data)
    let chart = try renderedHeight(rows.chart)
    #expect(abs(chart - NotchModel.chartHeight) < 1, "chart \(chart)")
    let content = try renderedHeight(rows)
    let budget =
      NotchModel.cardHeight(hasSpend: data.spend != nil, modelCount: data.models.count)
      - 2 * NotchModel.cardPadding
    #expect(abs(content - budget) < 1, "rows \(content)")
  }
}

@Test @MainActor func notchCardLooksTheSameInLightAndDarkMode() throws {
  let data = codexCardData()
  func pixels(_ scheme: ColorScheme) throws -> Data {
    let renderer = ImageRenderer(
      content: NotchCard(data: data).environment(\.colorScheme, scheme))
    return try #require(renderer.cgImage?.dataProvider?.data as Data?)
  }
  #expect(try pixels(.light) == pixels(.dark))
}

@Test @MainActor func notchRingsSkipAgentsWithoutAForecastAndKeepTheKnownOrder() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  store.summary = SummaryReport(
    totals: Totals(totalCost: 0, totalTokens: 0),
    agents: [agentUsage("codex"), agentUsage("claude"), agentUsage("opencode")],
    models: [], daily: nil, subscription: nil)
  store.reports["codex"] = harnessReport(agent: "codex", usedPercent: 40)
  store.updated["codex"] = .now
  store.reports["claude"] = harnessReport(agent: "claude", usedPercent: 70)
  store.updated["claude"] = .now

  let rings = NotchModel.rings(store: store)
  #expect(rings.map(\.agent) == ["codex", "claude"])
  #expect(rings.map(\.usedPercent) == [40, 70])
  #expect(NotchModel.card(store: store, agent: "opencode") == nil)
}

@Test @MainActor func notchCardFallsBackToTheSubscriptionPlanAndFormatsTopModels() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  store.summary = SummaryReport(
    totals: Totals(totalCost: 0, totalTokens: 0), agents: [], models: [], daily: nil,
    subscription: SubscriptionReport(agents: [
      SubscriptionAgent(
        agent: "codex", plan: "Pro Fallback", pricePerMonth: 20, periodUsage: 0,
        liveLimits: true, shortWindow: nil)
    ]))
  let models = [
    HarnessModel(model: "gpt-5", cost: 120.4, tokens: 0),
    HarnessModel(model: "gpt-4o", cost: 900.9, tokens: 0),
    HarnessModel(model: "o1", cost: 10, tokens: 0),
    HarnessModel(model: "cheap", cost: 1, tokens: 0),
  ]
  store.reports["codex"] = harnessReport(
    agent: "codex", usedPercent: 40, plan: nil, apiEquivalentPerMonth: 500, pricePerMonth: 20,
    economics: Economics(
      pricePerMonth: 20, apiEquivalentPerMonth: 500, subsidyPerMonth: 480, valueMultiple: 25,
      discountPercent: 96), topModels: models)
  store.updated["codex"] = .now

  let card = try #require(NotchModel.card(store: store, agent: "codex"))
  #expect(card.plan == "Pro Fallback")
  #expect(card.models.map(\.name) == ["gpt-4o", "gpt-5", "o1"])
  let expectedCosts = [900.9, 120.4, 10.0].map {
    $0.formatted(.currency(code: "USD").precision(.fractionLength(0)))
  }
  #expect(card.models.map(\.cost) == expectedCosts)
  #expect(card.spend == NotchModel.spend(apiEquivalent: 500, price: 20, multiple: 25))
}

@Test @MainActor func notchCardHasNoSpendWithoutAHarnessReport() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  let source = store.customPath + "|" + store.codexHomes
  var history = QuotaHistory()
  history.record(
    QuotaReading(
      agent: "claude", observedAt: Date.now.timeIntervalSince1970 * 1000,
      window: QuotaWindow(
        windowMinutes: 10080, usedPercent: 55, elapsedPercent: 20, apiEquivalentSpent: 0)),
    source: source)
  try QuotaHistoryFile(directory: directory).save(history)
  store.reloadQuotas()

  let card = try #require(NotchModel.card(store: store, agent: "claude"))
  #expect(card.spend == nil)
  #expect(card.plan == nil)
}

// Freshness needs a live (quota-history) reading: the reports+updated path always
// yields isLive == false, so it can never be non-stale on its own.
@Test @MainActor func notchCardIsStaleWhenTheReadingAgesOrQuotaFails() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  let source = store.customPath + "|" + store.codexHomes
  let now = Date.now
  var history = QuotaHistory()
  history.record(
    QuotaReading(
      agent: "claude", observedAt: now.addingTimeInterval(-5).timeIntervalSince1970 * 1000,
      window: QuotaWindow(
        windowMinutes: 10080, usedPercent: 55, elapsedPercent: 20, apiEquivalentSpent: 0)),
    source: source)
  history.record(
    QuotaReading(
      agent: "codex", observedAt: now.addingTimeInterval(-1000).timeIntervalSince1970 * 1000,
      window: QuotaWindow(
        windowMinutes: 10080, usedPercent: 12, elapsedPercent: 10, apiEquivalentSpent: 0)),
    source: source)
  try QuotaHistoryFile(directory: directory).save(history)
  store.reloadQuotas()

  let fresh = try #require(NotchModel.card(store: store, agent: "claude", now: now))
  #expect(!fresh.stale)

  store.errors["quotaCollector"] = "boom"
  let withError = try #require(NotchModel.card(store: store, agent: "claude", now: now))
  #expect(withError.stale)
  store.errors["quotaCollector"] = nil

  let aged = try #require(NotchModel.card(store: store, agent: "codex", now: now))
  #expect(aged.stale)
}

@Test @MainActor func notchCursorCardNamesPromotionalCreditsLikeTheDashboard() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  store.summary = SummaryReport(
    totals: Totals(totalCost: 0, totalTokens: 0),
    agents: [agentUsage("codex"), agentUsage("cursor")], models: [], daily: nil,
    subscription: nil)
  store.quotaChartRange = .week
  let source = store.customPath + "|" + store.codexHomes
  let observedAt = Date.now.timeIntervalSince1970 * 1000
  var history = QuotaHistory()
  history.record(
    QuotaReading(
      agent: "codex", observedAt: observedAt,
      window: QuotaWindow(
        windowMinutes: 10080, usedPercent: 12, elapsedPercent: 20, apiEquivalentSpent: 0)),
    source: source)
  history.record(
    QuotaReading(
      agent: "cursor", observedAt: observedAt,
      window: QuotaWindow(
        windowMinutes: 200 * 1440, usedPercent: 30, elapsedPercent: 50, apiEquivalentSpent: 0)),
    source: source)
  try QuotaHistoryFile(directory: directory).save(history)
  store.reloadQuotas()

  let cursor = try #require(NotchModel.card(store: store, agent: "cursor"))
  #expect(cursor.style.title == "Promotional credits")
  #expect(cursor.resetText == "Expires " + quotaDateCompact(cursor.forecast.reset))
  #expect(cursor.samples == store.samples(for: "cursor", range: store.cursorQuotaChartRange))
  let cursorChart = NotchCardRows(data: cursor).chart
  #expect(cursorChart.range == store.cursorQuotaChartRange)
  #expect(cursorChart.resetLabel == "Expires")

  let codex = try #require(NotchModel.card(store: store, agent: "codex"))
  #expect(codex.style.title == "Weekly quota")
  #expect(codex.resetText == "Resets " + quotaDateCompact(codex.forecast.reset))
  #expect(codex.samples == store.samples(for: "codex", range: .week))
  let codexChart = NotchCardRows(data: codex).chart
  #expect(codexChart.range == .week)
  #expect(codexChart.resetLabel == "Reset")

  #expect(
    NotchModel.rings(store: store).map(\.voiceOverLabel) == [
      "Codex, Weekly quota, 12 percent used", "Cursor, Promotional credits, 30 percent used",
    ])
}

private let notchScreen = CGRect(x: 1512, y: 120, width: 1512, height: 982)

@Test func notchPanelIsTallEnoughForThreeRingsAndHugsTheRightEdge() {
  let frame = NotchModel.panelFrame(screen: notchScreen, ringCount: 3)
  #expect(frame.height >= NotchModel.notchHeight(ringCount: 3))
  #expect(frame.height >= NotchModel.largestCardHeight)
  #expect(frame.width == NotchModel.cardWidth + NotchModel.cardGap + NotchModel.notchDepth)
  #expect(frame.maxX == notchScreen.maxX)
  #expect(abs(frame.midY - notchScreen.midY) < 0.001)
}

@Test func notchRectSitsFlushWithTheScreenEdgeWhenNoCardIsOpen() throws {
  let panel = NotchModel.panelFrame(screen: notchScreen, ringCount: 2)
  let rects = NotchModel.interactiveRects(panel: panel, ringCount: 2, cardHeight: nil)
  #expect(rects.count == 1)
  let notch = try #require(rects.first)
  #expect(notch.maxX == notchScreen.maxX)
  #expect(notch.width == NotchModel.notchDepth)
  #expect(notch.height <= NotchModel.notchHeight(ringCount: 2))
  #expect(abs(notch.midY - notchScreen.midY) < 0.001)
}

@Test func notchOpenCardAddsARectAtThePanelsLeadingEdge() throws {
  let panel = NotchModel.panelFrame(screen: notchScreen, ringCount: 1)
  let height = NotchModel.cardHeight(hasSpend: false, modelCount: 0)
  let rects = NotchModel.interactiveRects(panel: panel, ringCount: 1, cardHeight: height)
  #expect(rects.count == 2)
  let card = try #require(rects.first { $0.minX == panel.minX })
  #expect(card.width == NotchModel.cardWidth)
  #expect(card.height == height)
  #expect(abs(card.midY - panel.midY) < 0.001)
  #expect(rects.allSatisfy { $0 == card || !$0.intersects(card) })
}

@Test func notchWithoutRingsPassesEveryEventThrough() {
  let panel = NotchModel.panelFrame(screen: notchScreen, ringCount: 0)
  #expect(NotchModel.interactiveRects(panel: panel, ringCount: 0, cardHeight: nil).isEmpty)
  #expect(
    NotchModel.interactiveRects(
      panel: panel, ringCount: 0, cardHeight: NotchModel.largestCardHeight
    )
    .isEmpty)
}
