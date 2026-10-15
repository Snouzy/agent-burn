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
  static let cardWidth: CGFloat = 320
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

  static let cardPadding: CGFloat = 18
  static let headerHeight: CGFloat = 30
  static let limitHeight: CGFloat = 45
  static let chartPlotHeight: CGFloat = 96
  // Compact QuotaChart draws its 12 pt cursor label and a gap above the plot, outside `height:`.
  static let chartLabelHeight: CGFloat = 15
  static let chartLabelGap: CGFloat = 8
  static let chartHeight = chartLabelHeight + chartLabelGap + chartPlotHeight
  static let spendHeight: CGFloat = 38
  static let modelRowHeight: CGFloat = 22
  static let rowGap: CGFloat = 16

  static func cardHeight(hasSpend: Bool, modelCount: Int) -> CGFloat {
    let base = 2 * cardPadding + headerHeight + rowGap + limitHeight + rowGap + chartHeight
    guard hasSpend else { return base }
    return base + rowGap + spendHeight + CGFloat(min(modelCount, maxModels)) * modelRowHeight
  }

  static var largestCardHeight: CGFloat { cardHeight(hasSpend: true, modelCount: maxModels) }

  static func notchHeight(ringCount: Int) -> CGFloat {
    NotchMetrics(scale: 1).notchHeight(ringCount: ringCount)
  }

  // Screen coordinates, the space of NSEvent.mouseLocation.
  static func panelFrame(screen: CGRect, ringCount: Int) -> CGRect {
    NotchMetrics(scale: 1).panelFrame(screen: screen, ringCount: ringCount)
  }

  static func screen<S>(
    from screens: [S], preference: String?, id: (S) -> String?, frame: (S) -> CGRect
  ) -> S? {
    if let preference, let chosen = screens.first(where: { id($0) == preference }) {
      return chosen
    }
    return screens.reduce(nil) { best, next in
      best.map { frame(next).maxX > frame($0).maxX ? next : $0 } ?? next
    }
  }

  static func cardRect(panel: CGRect, height: CGFloat) -> CGRect {
    CGRect(x: panel.minX, y: panel.midY - height / 2, width: cardWidth, height: height)
  }

  static func interactiveRects(panel: CGRect, ringCount: Int, cardHeight: CGFloat?) -> [CGRect] {
    NotchMetrics(scale: 1).interactiveRects(
      panel: panel, ringCount: ringCount, cardHeight: cardHeight)
  }
}

/// The notch geometry at the size the user picked, like the Dock's size slider.
/// The hover card keeps its own fixed size.
struct NotchMetrics: Equatable, Sendable {
  static let scales: ClosedRange<Double> = 0.7...1.4

  let scale: CGFloat

  init(scale: Double) {
    self.scale = CGFloat(min(max(scale, Self.scales.lowerBound), Self.scales.upperBound))
  }

  var ringDiameter: CGFloat { NotchModel.ringDiameter * scale }
  var notchDepth: CGFloat { NotchModel.notchDepth * scale }
  var curl: CGFloat { NotchModel.curl * scale }
  var corner: CGFloat { NotchModel.corner * scale }
  var padTop: CGFloat { NotchModel.padTop * scale }
  var padBottom: CGFloat { NotchModel.padBottom * scale }
  var cellSpacing: CGFloat { NotchModel.cellSpacing * scale }
  var labelGap: CGFloat { NotchModel.labelGap * scale }
  var percentLine: CGFloat { NotchModel.percentLine * scale }
  var labelSize: CGFloat { 14 * scale }
  var iconSize: CGFloat { 17 * scale }
  private var cell: CGFloat { ringDiameter + labelGap + percentLine }

  func notchHeight(ringCount: Int) -> CGFloat {
    let cells = CGFloat(max(ringCount, 1))
    return 2 * curl + padTop + cells * cell + (cells - 1) * cellSpacing + padBottom
  }

  func panelFrame(screen: CGRect, ringCount: Int) -> CGRect {
    let width = NotchModel.cardWidth + NotchModel.cardGap + notchDepth
    let height = max(notchHeight(ringCount: ringCount), NotchModel.largestCardHeight)
    return CGRect(
      x: screen.maxX - width, y: screen.midY - height / 2, width: width, height: height)
  }

  func interactiveRects(panel: CGRect, ringCount: Int, cardHeight: CGFloat?) -> [CGRect] {
    guard ringCount > 0 else { return [] }
    let height = notchHeight(ringCount: ringCount)
    // The flares at both ends are mostly transparent: let clicks through there.
    let notch = CGRect(
      x: panel.maxX - notchDepth, y: panel.midY - height / 2, width: notchDepth, height: height
    )
    .insetBy(dx: 0, dy: curl)
    return [notch] + (cardHeight.map { [NotchModel.cardRect(panel: panel, height: $0)] } ?? [])
  }

  /// The vertical center of a ring cell, from the top of the notch.
  func cellCenter(_ index: Int) -> CGFloat {
    curl + padTop + CGFloat(index) * (cell + cellSpacing) + cell / 2
  }

  /// Like the Dock: the ring under the cursor grows by `amount`, its neighbours by less.
  func magnification(cell index: Int, mouseY: CGFloat?, amount: Double) -> CGFloat {
    guard let mouseY else { return 1 }
    let reach = 1.5 * (cell + cellSpacing)
    let distance = min(abs(mouseY - cellCenter(index)) / reach, 1)
    return 1 + CGFloat(amount) * (1 + cos(distance * .pi)) / 2
  }

  /// Dragging the grabber away from the screen edge grows the notch, like the Dock separator.
  static func dragScale(start: Double, startX: CGFloat, mouseX: CGFloat) -> Double {
    NotchMetrics(scale: start + Double(startX - mouseX) / NotchModel.notchDepth).scale.native
  }
}
