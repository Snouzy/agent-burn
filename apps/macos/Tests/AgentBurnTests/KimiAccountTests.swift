import Foundation
import SwiftUI
import Testing

@testable import AgentBurn

private func decodeKimi(_ json: String) throws -> KimiAccount {
  try JSONDecoder().decode(KimiAccount.self, from: Data(json.utf8))
}

@Test func kimiAccountDecodesEveryMeter() throws {
  let account = try decodeKimi(
    """
    {"sessionUsedPercent":72,"sessionResetsAtMs":1790000000000,"monthlyUsedPercent":40,"monthlyResetsAtMs":1791000000000,"monthlyTotalUsedPercent":55,"monthlyTotalResetsAtMs":1791000000000,"boosterRemaining":12.5,"boosterTotal":50,"boosterCurrency":"USD"}
    """)
  #expect(account.sessionUsedPercent == 72)
  #expect(account.monthlyUsedPercent == 40)
  #expect(account.monthlyTotalUsedPercent == 55)
  #expect(account.boosterRemaining == 12.5)
  #expect(account.boosterTotal == 50)
  #expect(account.boosterCurrency == "USD")
  #expect(account.meters.map(\.title) == ["Monthly", "Monthly total"])
  #expect(account.mainUsedPercent == 40)
}

@Test func kimiAccountToleratesMissingAndMalformedKeys() throws {
  let account = try decodeKimi(#"{"sessionUsedPercent":"oops","weeklyUsedPercent":12}"#)
  #expect(account.sessionUsedPercent == nil)
  #expect(account.weeklyUsedPercent == 12)
  #expect(account.boosterCurrency == nil)
  let empty = try decodeKimi("{}")
  #expect(empty.mainUsedPercent == nil)
}

@Test func kimiLegacyPlanUsesTheWeeklyMeter() throws {
  let account = try decodeKimi(#"{"weeklyUsedPercent":12,"weeklyResetsAtMs":1790000000000}"#)
  #expect(account.meters.map(\.title) == ["Weekly"])
  #expect(account.mainUsedPercent == 12)
  #expect(
    remainingQuota(for: .kimi, forecast: nil, cursorAccount: nil, kimiAccount: account) == 88)
}

@Test func kimiMainMeterPrefersWeeklyThenMonthlyThenMonthlyTotal() throws {
  let both = try decodeKimi(
    #"{"weeklyUsedPercent":12,"monthlyUsedPercent":40,"monthlyTotalUsedPercent":40}"#)
  #expect(both.mainUsedPercent == 12)
  // A monthly total equal to the Kimi Code meter adds nothing.
  #expect(both.meters.map(\.title) == ["Weekly", "Monthly"])
  let totalOnly = try decodeKimi(#"{"monthlyTotalUsedPercent":55}"#)
  #expect(totalOnly.mainUsedPercent == 55)
  #expect(totalOnly.meters.map(\.title) == ["Monthly total"])
}

@Test func kimiAccountSurvivesArchivalProjection() throws {
  let report = try JSONDecoder().decode(
    SummaryReport.self,
    from: Data(
      """
      {"totals":{"totalCost":0,"totalTokens":0},"agents":[],"models":[],"kimiAccount":{"monthlyUsedPercent":40,"boosterTotal":50}}
      """.utf8))
  let projected = MetricsArchive().report(period: .all, live: report)
  #expect(projected.kimiAccount?.monthlyUsedPercent == 40)
  #expect(projected.kimiAccount?.boosterTotal == 50)
}

// The collector's `harness kimi` is the single source of the main "kimi" series.
@Test func kimiAccountRecordsOnlyItsSessionReading() throws {
  let now = Date(timeIntervalSince1970: 1_790_000_000)
  let account = KimiAccount(
    sessionUsedPercent: 72, sessionResetsAtMs: (now.timeIntervalSince1970 + 3600) * 1000,
    weeklyUsedPercent: 12, weeklyResetsAtMs: (now.timeIntervalSince1970 + 86_400) * 1000,
    monthlyUsedPercent: 40, monthlyResetsAtMs: (now.timeIntervalSince1970 + 10 * 86_400) * 1000)
  var report = SummaryReport(
    totals: Totals(totalCost: 0, totalTokens: 0), agents: [], models: [], daily: nil,
    subscription: nil)
  report.kimiAccount = account
  let readings = accountQuotaReadings(report, now: now)
  #expect(readings.map(\.agent) == [kimiSessionQuotaAgent])

  let session = try #require(readings.first)
  #expect(session.window.windowMinutes == 300)
  #expect(session.window.usedPercent == 72)
  #expect(abs(session.window.elapsedPercent - 80) < 0.001)
}

@Test func expiredKimiSessionIsNotRecorded() {
  let now = Date(timeIntervalSince1970: 1_790_000_000)
  let account = KimiAccount(
    sessionUsedPercent: 72, sessionResetsAtMs: (now.timeIntervalSince1970 - 60) * 1000)
  #expect(kimiSessionReading(account, now: now) == nil)
}

@Test func sessionSeriesKeepTheirPersistedNames() {
  #expect(sessionQuotaAgent(for: "claude") == "claude-session")
  #expect(claudeSessionQuotaAgent == "claude-session")
  #expect(sessionQuotaAgent(for: "kimi") == "kimi-session")
  #expect(sessionQuotaAgent(for: "codex") == nil)
}

@Test func kimiIsAQuotaSourceNamedKimiCode() {
  #expect(QuotaSource.kimi.label == "Kimi Code")
  #expect(harnessName("kimi") == "Kimi Code")
}

@Test func monthlyWindowsUseTheMonthlyMeterStyle() {
  func forecast(_ minutes: Double) -> Forecast {
    Forecast(
      window: QuotaWindow(
        windowMinutes: minutes, usedPercent: 10, elapsedPercent: 10, apiEquivalentSpent: 0),
      observedAt: .now)
  }
  #expect(quotaMeterStyle(for: "kimi", forecast: forecast(43_200)) == .monthly)
  #expect(quotaMeterStyle(for: "kimi", forecast: forecast(10_080)) == .weekly)
  #expect(quotaMeterStyle(for: "codex", forecast: forecast(10_080)) == .weekly)
  #expect(quotaMeterStyle(for: "cursor", forecast: forecast(525_600)) == .promotionalCredits)
  #expect(QuotaMeterStyle.monthly.title == "Monthly quota")
}

@Test func kimiQuotaColorDiffersFromTheDefault() {
  let light = NSAppearance(named: .aqua)!
  let dark = NSAppearance(named: .darkAqua)!
  func rgb(_ color: Color, _ appearance: NSAppearance) -> NSColor {
    var resolved = NSColor.clear
    appearance.performAsCurrentDrawingAppearance {
      resolved = NSColor(color).usingColorSpace(.sRGB) ?? .clear
    }
    return resolved
  }
  for appearance in [light, dark] {
    let kimi = rgb(BurnTheme.quotaColor(for: "kimi"), appearance)
    let codex = rgb(BurnTheme.quotaColor(for: "codex"), appearance)
    #expect(kimi != codex)
    // Cyan family: blue and green both outweigh red.
    #expect(kimi.blueComponent > kimi.redComponent && kimi.greenComponent > kimi.redComponent)
  }
}
