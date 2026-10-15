import SwiftUI

struct KimiAccount: Codable, Sendable {
  let sessionUsedPercent: Double?
  let sessionResetsAtMs: Double?
  let weeklyUsedPercent: Double?
  let weeklyResetsAtMs: Double?
  let monthlyUsedPercent: Double?
  let monthlyResetsAtMs: Double?
  let monthlyTotalUsedPercent: Double?
  let monthlyTotalResetsAtMs: Double?
  let boosterRemaining: Double?
  let boosterTotal: Double?
  let boosterCurrency: String?

  init(
    sessionUsedPercent: Double? = nil, sessionResetsAtMs: Double? = nil,
    weeklyUsedPercent: Double? = nil, weeklyResetsAtMs: Double? = nil,
    monthlyUsedPercent: Double? = nil, monthlyResetsAtMs: Double? = nil,
    monthlyTotalUsedPercent: Double? = nil, monthlyTotalResetsAtMs: Double? = nil,
    boosterRemaining: Double? = nil, boosterTotal: Double? = nil, boosterCurrency: String? = nil
  ) {
    self.sessionUsedPercent = sessionUsedPercent
    self.sessionResetsAtMs = sessionResetsAtMs
    self.weeklyUsedPercent = weeklyUsedPercent
    self.weeklyResetsAtMs = weeklyResetsAtMs
    self.monthlyUsedPercent = monthlyUsedPercent
    self.monthlyResetsAtMs = monthlyResetsAtMs
    self.monthlyTotalUsedPercent = monthlyTotalUsedPercent
    self.monthlyTotalResetsAtMs = monthlyTotalResetsAtMs
    self.boosterRemaining = boosterRemaining
    self.boosterTotal = boosterTotal
    self.boosterCurrency = boosterCurrency
  }

  // One malformed meter must not drop the whole summary report.
  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    func number(_ key: CodingKeys) -> Double? {
      (try? container.decodeIfPresent(Double.self, forKey: key)) ?? nil
    }
    sessionUsedPercent = number(.sessionUsedPercent)
    sessionResetsAtMs = number(.sessionResetsAtMs)
    weeklyUsedPercent = number(.weeklyUsedPercent)
    weeklyResetsAtMs = number(.weeklyResetsAtMs)
    monthlyUsedPercent = number(.monthlyUsedPercent)
    monthlyResetsAtMs = number(.monthlyResetsAtMs)
    monthlyTotalUsedPercent = number(.monthlyTotalUsedPercent)
    monthlyTotalResetsAtMs = number(.monthlyTotalResetsAtMs)
    boosterRemaining = number(.boosterRemaining)
    boosterTotal = number(.boosterTotal)
    boosterCurrency = (try? container.decodeIfPresent(String.self, forKey: .boosterCurrency)) ?? nil
  }

  /// The long-window meters present, main one first: legacy plans meter weekly, current
  /// plans meter Kimi Code monthly, with the account-wide monthly total as the fallback.
  var meters: [KimiMeter] {
    var meters = [
      KimiMeter(title: "Weekly", usedPercent: weeklyUsedPercent, resetsAtMs: weeklyResetsAtMs),
      KimiMeter(title: "Monthly", usedPercent: monthlyUsedPercent, resetsAtMs: monthlyResetsAtMs),
    ]
    // A total equal to the Kimi Code meter says nothing new.
    let duplicate =
      monthlyUsedPercent.flatMap { used in monthlyTotalUsedPercent.map { abs($0 - used) < 0.05 } }
      ?? false
    if !duplicate {
      meters.append(
        KimiMeter(
          title: "Monthly total", usedPercent: monthlyTotalUsedPercent,
          resetsAtMs: monthlyTotalResetsAtMs))
    }
    return meters.filter { $0.usedPercent != nil }
  }
  var mainUsedPercent: Double? { meters.first?.usedPercent }
}

struct KimiMeter: Identifiable {
  let title: String
  let usedPercent: Double?
  let resetsAtMs: Double?
  var id: String { title }
}

/// Kimi Code's meters: the main quota chart (weekly on legacy plans, else monthly), the
/// 5-hour session chart, then slim rows for every other meter and the booster balance.
struct KimiAccountView: View {
  @Environment(UsageStore.self) private var store
  let account: KimiAccount?
  let plan: SubscriptionAgent?
  private var now: Date { store.quotaCheckDate }
  private var tint: Color { BurnTheme.color(for: "kimi") }

  var body: some View {
    GroupBox {
      VStack(alignment: .leading, spacing: 16) {
        HStack {
          Label(
            "Kimi Code " + (plan?.plan ?? "account"),
            systemImage: "gauge.with.dots.needle.33percent"
          )
          .font(.headline)
          Spacer()
          if let price = plan?.pricePerMonth {
            Text(currency(price) + " / month").foregroundStyle(.secondary)
          }
        }
        main
        Divider()
        session
        if let account, account.meters.count > 1 || account.boosterTotal != nil {
          Divider()
          VStack(spacing: 12) {
            ForEach(account.meters.dropFirst()) { meter in
              ClaudeMeterRow(
                title: meter.title, used: meter.usedPercent,
                detail: "Resets " + claudeDate(meter.resetsAtMs))
            }
            if let total = account.boosterTotal {
              ClaudeMeterRow(
                title: "Booster", used: kimiBoosterUsedPercent(account),
                value: kimiBoosterValue(account),
                detail: "of " + kimiAmount(total, currency: account.boosterCurrency))
            }
          }
        }
        if account == nil {
          Text(
            "Live account limits are unavailable. Refresh with live data enabled to load Kimi Code’s meters."
          )
          .font(.caption).foregroundStyle(.secondary)
        }
      }.padding(12)
    }
  }

  @ViewBuilder private var main: some View {
    @Bindable var store = store
    if let forecast = store.forecast(for: "kimi") {
      let range = store.quotaChartRange
      let samples = store.samples(for: "kimi", range: range, now: now)
      HStack(alignment: .top, spacing: 28) {
        QuotaSummary(
          forecast: forecast, samples: samples, now: now,
          stale: !forecast.isFresh(at: now) || store.quotaError(for: "kimi") != nil,
          staleHelp: store.quotaError(for: "kimi")
            ?? "Showing the last known reading. Update pending.",
          rates: store.blendRates(for: "kimi"),
          style: quotaMeterStyle(for: "kimi", forecast: forecast)
        )
        .frame(width: 236, alignment: .leading)
        VStack(alignment: .trailing, spacing: 8) {
          QuotaChartRangePicker(range: $store.quotaChartRange)
          QuotaChart(forecast: forecast, samples: samples, color: tint, range: range, now: now)
            .id(range)
        }
      }
    } else {
      let meter = account?.meters.first
      ClaudeMeterRow(
        title: meter?.title ?? "Quota", used: meter?.usedPercent,
        detail: "Resets " + claudeDate(meter?.resetsAtMs))
    }
  }

  @ViewBuilder private var session: some View {
    if let forecast = store.forecast(for: kimiSessionQuotaAgent), forecast.reset > now {
      let samples = store.samples(for: kimiSessionQuotaAgent, range: .rte, now: now)
      HStack(alignment: .top, spacing: 28) {
        ClaudeSessionSummary(
          forecast: forecast, samples: samples, now: now, stale: !forecast.isFresh(at: now)
        )
        .frame(width: 236, alignment: .leading)
        QuotaChart(
          forecast: forecast, samples: samples, color: tint, compact: true, range: .rte,
          now: now, height: 120)
      }
    } else {
      ClaudeMeterRow(
        title: "Session · 5 hours", used: account?.sessionUsedPercent,
        detail: "Resets " + claudeDate(account?.sessionResetsAtMs, time: true))
    }
  }
}

func kimiBoosterUsedPercent(_ account: KimiAccount) -> Double? {
  guard let total = account.boosterTotal, total > 0, let remaining = account.boosterRemaining
  else { return nil }
  return max(0, min(100, (total - remaining) / total * 100))
}

func kimiBoosterValue(_ account: KimiAccount) -> String {
  account.boosterRemaining.map { kimiAmount($0, currency: account.boosterCurrency) + " left" }
    ?? "Unavailable"
}

func kimiAmount(_ value: Double, currency code: String?) -> String {
  guard let code, !code.isEmpty else { return value.formatted(.number.precision(.fractionLength(2))) }
  return value.formatted(.currency(code: code))
}
