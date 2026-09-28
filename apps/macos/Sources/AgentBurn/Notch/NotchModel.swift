import Foundation

enum NotchModel {
  /// Codex's 44pt ring and its proportions, so the notch reads as Codenotch's.
  static let ringDiameter: CGFloat = 44
  static let notchDepth: CGFloat = 70
  static let curl: CGFloat = 38.7
  static let corner: CGFloat = 29.6
  static let padTop: CGFloat = 26.1
  static let padBottom: CGFloat = 18.8
  static let cellSpacing: CGFloat = 31.4
  static let labelGap: CGFloat = 10.1
  static let percentLine: CGFloat = 18
  static let cardWidth: CGFloat = 300
  static let cardGap: CGFloat = 12
  static let maxModels = 3

  private static let order = ["codex", "claude", "cursor"]

  static func agents(known: [String], hasForecast: (String) -> Bool) -> [String] {
    known.filter(hasForecast).sorted { a, b in
      let ra = order.firstIndex(of: a) ?? order.count
      let rb = order.firstIndex(of: b) ?? order.count
      return ra != rb ? ra < rb : a < b
    }
  }

  struct Spend: Equatable, Sendable {
    let spend: String
    let plan: String?
    let multiple: String?
  }

  static func spend(
    apiEquivalent: Double, price: Double?, multiple: Double?, locale: Locale = .current
  )
    -> Spend
  {
    func dollars(_ value: Double) -> String {
      value.formatted(
        .currency(code: "USD").precision(.fractionLength(value >= 100 ? 0 : 2)).locale(locale))
    }
    let times = multiple.map { $0 >= 10 ? "×\(Int($0.rounded()))" : String(format: "×%.1f", $0) }
    return Spend(
      spend: "~" + dollars(apiEquivalent),
      plan: price.map {
        "plan " + $0.formatted(.currency(code: "USD").precision(.fractionLength(0)).locale(locale))
      },
      multiple: times)
  }

  static let cardPadding: CGFloat = 14
  static let headerHeight: CGFloat = 30
  static let limitHeight: CGFloat = 40
  static let chartPlotHeight: CGFloat = 96
  // Compact QuotaChart draws its 12 pt cursor label and a gap above the plot, outside `height:`.
  static let chartLabelHeight: CGFloat = 15
  static let chartLabelGap: CGFloat = 8
  static let chartHeight = chartLabelHeight + chartLabelGap + chartPlotHeight
  static let spendHeight: CGFloat = 36
  static let modelRowHeight: CGFloat = 18
  static let rowGap: CGFloat = 10

  static func cardHeight(hasSpend: Bool, modelCount: Int) -> CGFloat {
    let base = 2 * cardPadding + headerHeight + rowGap + limitHeight + rowGap + chartHeight
    guard hasSpend else { return base }
    return base + rowGap + spendHeight + CGFloat(min(modelCount, maxModels)) * modelRowHeight
  }

  static var largestCardHeight: CGFloat { cardHeight(hasSpend: true, modelCount: maxModels) }

  static func notchHeight(ringCount: Int) -> CGFloat {
    let cells = CGFloat(max(ringCount, 1))
    let cell = ringDiameter + labelGap + percentLine
    return 2 * curl + padTop + cells * cell + (cells - 1) * cellSpacing + padBottom
  }

  // Screen coordinates, the space of NSEvent.mouseLocation.
  static func panelFrame(screen: CGRect, ringCount: Int) -> CGRect {
    let width = cardWidth + cardGap + notchDepth
    let height = max(notchHeight(ringCount: ringCount), largestCardHeight)
    return CGRect(
      x: screen.maxX - width, y: screen.midY - height / 2, width: width, height: height)
  }

  static func cardRect(panel: CGRect, height: CGFloat) -> CGRect {
    CGRect(x: panel.minX, y: panel.midY - height / 2, width: cardWidth, height: height)
  }

  static func interactiveRects(panel: CGRect, ringCount: Int, cardHeight: CGFloat?) -> [CGRect] {
    guard ringCount > 0 else { return [] }
    let height = notchHeight(ringCount: ringCount)
    // The flares at both ends are mostly transparent: let clicks through there.
    let notch = CGRect(
      x: panel.maxX - notchDepth, y: panel.midY - height / 2, width: notchDepth, height: height
    )
    .insetBy(dx: 0, dy: curl)
    return [notch] + (cardHeight.map { [cardRect(panel: panel, height: $0)] } ?? [])
  }
}
