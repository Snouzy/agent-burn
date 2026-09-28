import Foundation

extension NotchModel {
  @MainActor static func rings(store: UsageStore, now: Date = .now) -> [NotchRing] {
    agents(known: store.knownAgents, hasForecast: { store.forecast(for: $0) != nil }).map {
      agent in
      let forecast = store.forecast(for: agent)!
      return NotchRing(
        agent: agent, usedPercent: forecast.window.usedPercent,
        stale: !forecast.isFresh(at: now) || store.quotaError(for: agent) != nil,
        style: style(for: agent))
    }
  }

  @MainActor static func card(store: UsageStore, agent: String, now: Date = .now) -> NotchCardData?
  {
    guard let forecast = store.forecast(for: agent) else { return nil }
    let report = store.reports[agent]
    let plan =
      report?.plan
      ?? store.summary?.subscription?.agents.first { $0.agent == agent }?.plan
    let spend = report.map {
      NotchModel.spend(
        apiEquivalent: $0.apiEquivalentPerMonth, price: $0.pricePerMonth,
        multiple: $0.economics?.valueMultiple)
    }
    let models = (report?.topModels ?? [])
      .sorted { $0.cost > $1.cost }
      .prefix(maxModels)
      .map {
        (
          name: $0.model,
          cost: $0.cost.formatted(.currency(code: "USD").precision(.fractionLength(0)))
        )
      }
    let range = store.chartRange(for: agent)
    return NotchCardData(
      agent: agent, title: harnessName(agent), plan: plan, forecast: forecast,
      samples: store.samples(for: agent, range: range),
      stale: !forecast.isFresh(at: now)
        || store.quotaError(for: agent) != nil,
      spend: spend, models: models, style: style(for: agent), range: range)
  }

  // Cursor has a forecast only while promotional credits burn: see cursorQuotaReading.
  private static func style(for agent: String) -> QuotaMeterStyle {
    agent == "cursor" ? .promotionalCredits : .weekly
  }
}
