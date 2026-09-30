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

private let staleRing = NotchRing(agent: "claude", usedPercent: 10, stale: true, style: .weekly)

@Test func notchStaleRingLightsUpWhileItsCardIsOpen() {
  let fresh = NotchRing(agent: "codex", usedPercent: 36, stale: false, style: .weekly)
  #expect(staleRing.opacity(highlighted: false) == 0.45)
  #expect(staleRing.opacity(highlighted: true) == 1)
  #expect(fresh.opacity(highlighted: false) == 1)
  #expect(fresh.opacity(highlighted: true) == 1)
}

@Test @MainActor func notchRingKeepsItsSizeWhenHighlighted() throws {
  func size(highlighted: Bool) throws -> CGSize {
    let ring = UsageRing(ring: staleRing, highlighted: highlighted)
    return try #require(ImageRenderer(content: ring).nsImage?.size)
  }
  #expect(try size(highlighted: true) == size(highlighted: false))
  let cell = NotchModel.ringDiameter + NotchModel.labelGap + NotchModel.percentLine
  #expect(try size(highlighted: true).height == cell)
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
    agent: "codex", title: "Codex", plan: "Pro", forecast: forecast, samples: [], staleText: nil,
    spend: hasSpend ? NotchModel.spend(apiEquivalent: 5927, price: 200, multiple: 29.6) : nil,
    models: hasSpend
      ? [
        .init(name: "gpt-5.6-sol", cost: "$3,120"), .init(name: "gpt-5.5", cost: "$1,980"),
        .init(name: "gpt-5.4-mini", cost: "$212"),
      ] : [],
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
    let limit = try renderedHeight(rows.limit)
    #expect(abs(limit - NotchModel.limitHeight) < 1, "limit \(limit)")
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
  #expect(fresh.staleText == nil)

  store.errors["quotaCollector"] = "boom"
  let withError = try #require(NotchModel.card(store: store, agent: "claude", now: now))
  #expect(withError.staleText != nil)
  store.errors["quotaCollector"] = nil

  let aged = try #require(NotchModel.card(store: store, agent: "codex", now: now))
  #expect(aged.staleText != nil)
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

@Test @MainActor func notchDisplayDefaultsToAutomaticAndPersists() throws {
  let suite = "AgentBurn.notch.display.\(UUID().uuidString)"
  let defaults = try #require(UserDefaults(suiteName: suite))
  defer { defaults.removePersistentDomain(forName: suite) }
  let appearance = AppAppearance(defaults: defaults, applyPolicy: { _ in })
  #expect(appearance.notchDisplay == nil)
  appearance.notchDisplay = "37D8832A-2D66-02CA-B9F7-8F30A301B230"
  #expect(
    AppAppearance(defaults: defaults, applyPolicy: { _ in }).notchDisplay
      == "37D8832A-2D66-02CA-B9F7-8F30A301B230")
  appearance.notchDisplay = nil
  #expect(AppAppearance(defaults: defaults, applyPolicy: { _ in }).notchDisplay == nil)
}

private struct FakeScreen {
  let id: String?
  let frame: CGRect
}

private let menuBarScreen = FakeScreen(
  id: "BUILTIN", frame: CGRect(x: 0, y: 0, width: 1728, height: 1117))
private let externalScreen = FakeScreen(
  id: "EXTERNAL", frame: CGRect(x: 1728, y: 0, width: 1920, height: 1080))
private let leftScreen = FakeScreen(
  id: "LEFT", frame: CGRect(x: -1920, y: 0, width: 1920, height: 1080))

private func notchScreenID(_ screens: [FakeScreen], preference: String?) -> String? {
  NotchModel.screen(from: screens, preference: preference, id: \.id, frame: \.frame)?.id
}

@Test func notchHasNoScreenWithoutScreens() {
  #expect(notchScreenID([], preference: nil) == nil)
  #expect(notchScreenID([], preference: "EXTERNAL") == nil)
}

@Test func notchGoesToTheChosenDisplayWhenItIsConnected() {
  let screens = [menuBarScreen, externalScreen, leftScreen]
  #expect(notchScreenID(screens, preference: "BUILTIN") == "BUILTIN")
  #expect(notchScreenID(screens, preference: "LEFT") == "LEFT")
}

@Test func notchAutomaticPicksTheRightmostScreenNotTheMenuBarOne() {
  #expect(notchScreenID([menuBarScreen, externalScreen], preference: nil) == "EXTERNAL")
  #expect(notchScreenID([menuBarScreen, leftScreen], preference: nil) == "BUILTIN")
  let above = FakeScreen(id: "ABOVE", frame: CGRect(x: 0, y: 1117, width: 1728, height: 1117))
  #expect(notchScreenID([menuBarScreen, above], preference: nil) == "BUILTIN")
  let unnamed = FakeScreen(id: nil, frame: menuBarScreen.frame)
  #expect(notchScreenID([unnamed, externalScreen], preference: nil) == "EXTERNAL")
}

@Test func notchFallsBackToTheRightmostScreenWhenTheChosenOneIsGone() {
  let screens = [menuBarScreen, externalScreen, leftScreen]
  #expect(notchScreenID(screens, preference: "UNPLUGGED") == "EXTERNAL")
}

@Test @MainActor func notchPreparesOneCardPerRingInRingOrder() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  store.summary = SummaryReport(
    totals: Totals(totalCost: 0, totalTokens: 0),
    agents: [agentUsage("claude"), agentUsage("codex"), agentUsage("opencode")],
    models: [], daily: nil, subscription: nil)
  store.reports["claude"] = harnessReport(agent: "claude", usedPercent: 70)
  store.updated["claude"] = .now
  store.reports["codex"] = harnessReport(agent: "codex", usedPercent: 40)
  store.updated["codex"] = .now

  let cards = NotchModel.cards(store: store)
  #expect(cards.map(\.agent) == NotchModel.rings(store: store).map(\.agent))
  #expect(cards.map(\.agent) == ["codex", "claude"])
}

private final class CardBodies {
  private(set) var byAgent: [String: Int] = [:]
  func record(_ agent: String) { byAgent[agent, default: 0] += 1 }
}

private struct CountedNotchCard: View {
  let data: NotchCardData
  let bodies: CardBodies

  var body: some View {
    let _ = bodies.record(data.agent)
    NotchCard(data: data)
  }
}

private struct NotchCardsHost: View {
  let cards: [NotchCardData]
  let bodies: CardBodies

  var body: some View {
    ForEach(cards, id: \.agent) { CountedNotchCard(data: $0, bodies: bodies) }
  }
}

// quotaCheckDate ticks about every 5 s, even with the notch closed: only new data may rebuild a chart.
@Test @MainActor func notchTickWithUnchangedDataSkipsTheCards() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  store.summary = SummaryReport(
    totals: Totals(totalCost: 0, totalTokens: 0),
    agents: [agentUsage("codex"), agentUsage("claude")], models: [], daily: nil,
    subscription: nil)
  func report(_ agent: String, topCost: Double) -> HarnessReport {
    harnessReport(
      agent: agent, usedPercent: 40, apiEquivalentPerMonth: 500, pricePerMonth: 20,
      topModels: [
        HarnessModel(model: "top", cost: topCost, tokens: 0),
        HarnessModel(model: "second", cost: 20, tokens: 0),
      ])
  }
  let observedAt = Date.now.addingTimeInterval(-3 * 3600)
  for agent in ["codex", "claude"] {
    store.reports[agent] = report(agent, topCost: 300)
    store.updated[agent] = observedAt
  }

  let bodies = CardBodies()
  let host = NSHostingView(rootView: NotchCardsHost(cards: [], bodies: bodies))
  host.frame = CGRect(
    x: 0, y: 0, width: NotchModel.cardWidth, height: NotchModel.largestCardHeight)
  var now = Date.now
  func tick(_ seconds: TimeInterval = 5) {
    now.addTimeInterval(seconds)
    host.rootView = NotchCardsHost(cards: NotchModel.cards(store: store, now: now), bodies: bodies)
    host.layoutSubtreeIfNeeded()
  }

  let ticks = 12
  for _ in 0..<ticks { tick() }
  #expect(bodies.byAgent == ["codex": 1, "claude": 1], "card bodies over \(ticks) ticks")

  store.reports["codex"] = report("codex", topCost: 310)
  tick()
  #expect(bodies.byAgent == ["codex": 2, "claude": 1], "after a new codex model cost")

  tick(2 * 3600)
  #expect(bodies.byAgent == ["codex": 3, "claude": 2], "after the stale text ages")
}

@Test @MainActor func notchStaleTextFollowsTheStoreClock() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  let observedAt = Date.now.addingTimeInterval(-2 * 86_400)
  store.reports["codex"] = harnessReport(agent: "codex", usedPercent: 40)
  store.updated["codex"] = observedAt

  func staleText(after seconds: TimeInterval) throws -> String? {
    try #require(
      NotchModel.card(store: store, agent: "codex", now: observedAt.addingTimeInterval(seconds))
    ).staleText
  }
  let minutes = try #require(try staleText(after: 3 * 60))
  let hours = try #require(try staleText(after: 3 * 3600))
  #expect(minutes.hasPrefix("Saved reading · "))
  #expect(minutes != hours)
  #expect(try staleText(after: 3 * 60) == minutes)
}

@Test @MainActor func notchCardHeightFromTheStoreMatchesTheFullCard() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  var history = QuotaHistory()
  history.record(
    QuotaReading(
      agent: "claude", observedAt: Date.now.timeIntervalSince1970 * 1000,
      window: QuotaWindow(
        windowMinutes: 10080, usedPercent: 55, elapsedPercent: 20, apiEquivalentSpent: 0)),
    source: store.customPath + "|" + store.codexHomes)
  try QuotaHistoryFile(directory: directory).save(history)
  store.reloadQuotas()
  store.reports["codex"] = harnessReport(
    agent: "codex", usedPercent: 40, apiEquivalentPerMonth: 500,
    topModels: (1...5).map { HarnessModel(model: "m\($0)", cost: Double($0), tokens: 0) })
  store.updated["codex"] = .now
  store.reports["gemini"] = harnessReport(agent: "gemini", usedPercent: 10)
  store.updated["gemini"] = .now

  for agent in ["codex", "gemini", "claude", "opencode"] {
    let full = NotchModel.card(store: store, agent: agent).map {
      NotchModel.cardHeight(hasSpend: $0.spend != nil, modelCount: $0.models.count)
    }
    #expect(NotchModel.cardHeight(store: store, agent: agent) == full, "\(agent)")
    #expect((full == nil) == (agent == "opencode"), "\(agent)")
  }
}

@Test @MainActor func notchRingClickOpensTheDashboardOnThatAgent() throws {
  let (store, suite, directory) = notchStoreFixture()
  let controller = NotchController.shared
  var opened = 0
  controller.attach(store)
  controller.showDashboard = { opened += 1 }
  defer {
    controller.showDashboard = nil
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }

  controller.open("codex")
  controller.openDashboard("claude")
  #expect(store.selection == "claude")
  #expect(opened == 1)
  #expect(controller.openAgent == nil)
}

@Test func notchDiskFreePercentIsTheFreeShareOfTheDisk() {
  #expect(DiskSpace(free: 123_500_000_000, total: 494_000_000_000).freePercent == 25)
  #expect(DiskSpace(free: 0, total: 494_000_000_000).freePercent == 0)
}

@Test func notchDiskLabelCountsInDecimalUnitsLikeFinder() {
  let us = Locale(identifier: "en_US")
  #expect(DiskSpace.label(bytes: 15_170_000_000, locale: us) == "15 GB")
  #expect(DiskSpace.label(bytes: 8_400_000_000, locale: us) == "8.4 GB")
  #expect(DiskSpace.label(bytes: 1_200_000_000_000, locale: us) == "1.2 TB")
  #expect(DiskSpace.label(bytes: 9_970_000_000, locale: us) == "10 GB")
  #expect(DiskSpace.label(bytes: 999_700_000_000, locale: us) == "1.0 TB")
  #expect(
    DiskSpace.label(bytes: 15_170_000_000, locale: Locale(identifier: "fr_FR")).hasSuffix("Go"))
  let disk = DiskSpace(free: 15_170_000_000, total: 494_384_795_648)
  #expect(
    disk.voiceOverLabel(locale: us) == "Startup disk, 15 gigabytes free of 494 gigabytes")
}

@Test func notchDiskReadsTheStartupVolumeLikeFoundation() throws {
  let disk = try #require(DiskSpace.startupDisk())
  let volume = try URL(fileURLWithPath: "/").resourceValues(forKeys: [.volumeTotalCapacityKey])
  #expect(disk.total == Int64(try #require(volume.volumeTotalCapacity)))
  #expect(disk.free > 0 && disk.free < disk.total)
}

@Test @MainActor func notchDiskSwitchDefaultsOffAndPersists() throws {
  let suite = "AgentBurn.notch.disk.\(UUID().uuidString)"
  let defaults = try #require(UserDefaults(suiteName: suite))
  defer { defaults.removePersistentDomain(forName: suite) }
  let appearance = AppAppearance(defaults: defaults, applyPolicy: { _ in })
  var changes: [Bool] = []
  appearance.onNotchChange = { changes.append($0) }
  #expect(!appearance.showsDiskSpace)
  appearance.showsDiskSpace = true
  #expect(AppAppearance(defaults: defaults, applyPolicy: { _ in }).showsDiskSpace)
  #expect(changes == [false])
}

@MainActor private func notchAppearanceFixture(showsDiskSpace: Bool) -> (
  appearance: AppAppearance, suite: String
) {
  let suite = "AgentBurn.notch.appearance.\(UUID().uuidString)"
  let appearance = AppAppearance(defaults: UserDefaults(suiteName: suite)!, applyPolicy: { _ in })
  appearance.showsDiskSpace = showsDiskSpace
  return (appearance, suite)
}

private let fifteenGB = DiskSpace(free: 15_170_000_000, total: 494_384_795_648)

private final class DiskReader {
  var reading: DiskSpace? = fifteenGB
  private(set) var reads = 0
  func read() -> DiskSpace? {
    reads += 1
    return reading
  }
}

@Test @MainActor func notchRingCountHasTheDiskRingOnlyWhenItIsOnAndReadable() {
  let (appearance, suite) = notchAppearanceFixture(showsDiskSpace: true)
  defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
  let reader = DiskReader()
  let controller = NotchController(appearance: appearance, readDisk: reader.read)
  controller.ringsChanged(["codex", "claude"])
  controller.refreshDisk()
  #expect(controller.ringCount == 3)
  appearance.showsDiskSpace = false
  controller.refreshDisk()
  #expect(controller.ringCount == 2)
  appearance.showsDiskSpace = true
  reader.reading = nil
  controller.refreshDisk()
  #expect(controller.ringCount == 2)
  reader.reading = fifteenGB
  controller.ringsChanged([])
  controller.refreshDisk()
  #expect(controller.ringCount == 1)
}

// A controller without a store never builds its panel, so apply(true) is safe here.
@Test @MainActor func notchDiskToggleReachesTheNotchThroughItsHook() {
  let (appearance, suite) = notchAppearanceFixture(showsDiskSpace: false)
  defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
  let reader = DiskReader()
  let controller = NotchController(appearance: appearance, readDisk: reader.read)
  appearance.onNotchChange = { [weak controller] in controller?.apply($0) }
  appearance.showsNotch = true
  #expect(reader.reads == 0)
  appearance.showsDiskSpace = true
  #expect(reader.reads == 1)
  #expect(controller.disk == fifteenGB)
  appearance.showsDiskSpace = false
  #expect(reader.reads == 1)
  #expect(controller.disk == nil)
}

@Test @MainActor func notchStoreTickRereadsTheDisk() throws {
  let (store, suite, directory) = notchStoreFixture()
  let (appearance, appearanceSuite) = notchAppearanceFixture(showsDiskSpace: true)
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    UserDefaults(suiteName: appearanceSuite)?.removePersistentDomain(forName: appearanceSuite)
    try? FileManager.default.removeItem(at: directory)
  }
  let reader = DiskReader()
  let controller = NotchController(appearance: appearance, readDisk: reader.read)
  controller.apply(true)
  let host = NSHostingView(rootView: NotchRoot(controller: controller, store: store))
  host.frame = CGRect(
    x: 0, y: 0, width: NotchModel.cardWidth + NotchModel.cardGap + NotchModel.notchDepth,
    height: NotchModel.largestCardHeight)
  host.layoutSubtreeIfNeeded()
  #expect(reader.reads == 1)

  reader.reading = DiskSpace(free: 14_000_000_000, total: 494_384_795_648)
  store.quotaCheckDate.addTimeInterval(5)
  host.layoutSubtreeIfNeeded()
  #expect(reader.reads == 2)
  #expect(controller.disk == reader.reading)
}

@Test @MainActor func notchDiskDriftThatKeepsTheRingKeepsTheValue() {
  let (appearance, suite) = notchAppearanceFixture(showsDiskSpace: true)
  defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
  let reader = DiskReader()
  let controller = NotchController(appearance: appearance, readDisk: reader.read)
  controller.refreshDisk()
  reader.reading = DiskSpace(free: 15_130_000_000, total: 494_384_795_648)
  controller.refreshDisk()
  #expect(reader.reads == 2)
  #expect(controller.disk == fifteenGB)

  let terabytes = DiskSpace(free: 1_200_000_000_000, total: 2_000_000_000_000)
  reader.reading = terabytes
  controller.refreshDisk()
  reader.reading = DiskSpace(free: 1_230_000_000_000, total: 2_000_000_000_000)
  controller.refreshDisk()
  #expect(controller.disk == reader.reading, "same label, the arc moves from 60 % to 61 %")
}

@Test @MainActor func notchDiskClickOpensStorageSettings() {
  let (appearance, suite) = notchAppearanceFixture(showsDiskSpace: true)
  defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
  var opened: [URL] = []
  let controller = NotchController(appearance: appearance, openURL: { opened.append($0) })
  controller.openStorage()
  #expect(
    opened.map(\.absoluteString) == ["x-apple.systempreferences:com.apple.settings.Storage"])
}

@Test @MainActor func notchDiskWidestLabelFitsTheNotch() throws {
  let cell = NotchModel.ringDiameter + NotchModel.labelGap + NotchModel.percentLine
  for free: Int64 in [999_400_000_000, 9_940_000_000, 8_000_000_000_000] {
    let ring = DiskRing(disk: DiskSpace(free: free, total: 8_000_000_000_000), highlighted: false)
      .environment(\.locale, Locale(identifier: "en_US"))
    let size = try #require(ImageRenderer(content: ring.fixedSize()).nsImage?.size)
    #expect(size.width <= NotchModel.notchDepth, "\(free) bytes: \(size.width) pt wide")
    #expect(size.height == cell)
  }
}

@Test @MainActor func notchCardChartFollowsTheStoreClockNotTheWallClock() throws {
  let (store, suite, directory) = notchStoreFixture()
  defer {
    UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    try? FileManager.default.removeItem(at: directory)
  }
  store.quotaChartRange = .week
  let wall = Date.now
  var history = QuotaHistory()
  for (daysAgo, used) in [(9.0, 20.0), (0.001, 40.0)] {
    history.record(
      QuotaReading(
        agent: "codex",
        observedAt: wall.addingTimeInterval(-daysAgo * 86_400).timeIntervalSince1970 * 1000,
        window: QuotaWindow(
          windowMinutes: 10080, usedPercent: used, elapsedPercent: 20, apiEquivalentSpent: 0)),
      source: store.customPath + "|" + store.codexHomes)
  }
  try QuotaHistoryFile(directory: directory).save(history)
  store.reloadQuotas()

  let now = wall.addingTimeInterval(-3 * 86_400)
  let card = try #require(NotchModel.card(store: store, agent: "codex", now: now))
  #expect(
    store.samples(for: "codex", range: .week, now: now)
      != store.samples(for: "codex", range: .week, now: wall))
  #expect(card.samples == store.samples(for: "codex", range: card.range, now: now))
}
