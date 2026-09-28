import SwiftUI

struct NotchCardData {
  let agent: String
  let title: String
  let plan: String?
  let forecast: Forecast
  let samples: [QuotaSample]
  let stale: Bool
  let spend: NotchModel.Spend?
  let models: [(name: String, cost: String)]
  let style: QuotaMeterStyle
  let range: QuotaChartRange

  var resetText: String { "\(style.resetDateTitle) \(quotaDateCompact(forecast.reset))" }
}

struct NotchCard: View {
  let data: NotchCardData

  var body: some View {
    NotchCardRows(data: data)
      .padding(NotchModel.cardPadding)
      .frame(
        width: NotchModel.cardWidth,
        height: NotchModel.cardHeight(hasSpend: data.spend != nil, modelCount: data.models.count),
        alignment: .top
      )
      .background(Color.black)
      .clipShape(RoundedRectangle(cornerRadius: 20))
      .foregroundStyle(.white)
      .environment(\.colorScheme, .dark)
  }
}

// Split from NotchCard so a render test can measure content height, unclipped.
struct NotchCardRows: View {
  let data: NotchCardData
  private var color: Color { BurnTheme.quotaColor(for: data.agent) }

  var body: some View {
    VStack(alignment: .leading, spacing: NotchModel.rowGap) {
      header
      limit
      chart
      if let spend = data.spend {
        VStack(alignment: .leading, spacing: 0) {
          spendRow(spend)
          ForEach(data.models.prefix(NotchModel.maxModels), id: \.name) { model in
            modelRow(model)
          }
        }
      }
    }
  }

  var chart: QuotaChart {
    QuotaChart(
      forecast: data.forecast, samples: data.samples, color: color, compact: true,
      range: data.range, resetLabel: data.style.chartResetLabel,
      height: NotchModel.chartPlotHeight)
  }

  private var header: some View {
    HStack(spacing: 8) {
      if let image = BrandImages.images[data.agent] {
        Image(nsImage: image).resizable().interpolation(.high).frame(width: 18, height: 18)
      }
      Text(data.title).font(.system(size: 14, weight: .semibold))
      if let plan = data.plan {
        Text(plan).font(.system(size: 12)).foregroundStyle(Color.white.opacity(0.6))
      }
      Spacer()
    }
    .frame(height: NotchModel.headerHeight)
  }

  private var limit: some View {
    let used = data.forecast.window.usedPercent
    return VStack(alignment: .leading, spacing: 4) {
      HStack {
        Text(data.style.title).font(.system(size: 12))
        Spacer()
        Text(data.resetText)
          .font(.system(size: 11)).foregroundStyle(Color.white.opacity(0.6))
      }
      GeometryReader { proxy in
        ZStack(alignment: .leading) {
          Capsule().fill(Color.white.opacity(0.18))
          Capsule().fill(color)
            .frame(width: proxy.size.width * min(max(used / 100, 0), 1))
        }
      }
      .frame(height: 4)
      Text(
        data.stale
          ? "Saved reading · " + data.forecast.observedAt.formatted(.relative(presentation: .named))
          : "\(Int(used.rounded()))% used"
      )
      .font(.system(size: 11)).foregroundStyle(Color.white.opacity(0.6))
    }
    .frame(height: NotchModel.limitHeight)
  }

  private func spendRow(_ spend: NotchModel.Spend) -> some View {
    VStack(alignment: .leading, spacing: 2) {
      Text("30 days · API-equivalent").font(.system(size: 11))
        .foregroundStyle(Color.white.opacity(0.6))
      Text([spend.spend, spend.plan, spend.multiple].compactMap { $0 }.joined(separator: " · "))
        .font(.system(size: 13, weight: .medium))
    }
    .frame(height: NotchModel.spendHeight, alignment: .top)
  }

  private func modelRow(_ model: (name: String, cost: String)) -> some View {
    HStack {
      Text(model.name).font(.system(size: 11)).lineLimit(1)
      Spacer()
      Text(model.cost).font(.system(size: 11)).foregroundStyle(Color.white.opacity(0.6))
    }
    .frame(height: NotchModel.modelRowHeight)
  }
}
