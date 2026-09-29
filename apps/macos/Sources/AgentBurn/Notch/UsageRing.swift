// Adapted from Codenotch (github.com/vinzdg/codenotch), MIT License, Copyright (c) 2026 Vinz.
import SwiftUI

struct NotchRing: Equatable, Identifiable, Sendable {
  let agent: String
  let usedPercent: Double
  let stale: Bool
  let style: QuotaMeterStyle
  var id: String { agent }
  var voiceOverLabel: String {
    "\(harnessName(agent)), \(style.title), \(Int(usedPercent.rounded())) percent used"
  }
  func opacity(highlighted: Bool) -> Double { stale && !highlighted ? 0.45 : 1 }
}

struct UsageRing: View {
  let ring: NotchRing
  let highlighted: Bool
  var body: some View {
    let color = BurnTheme.color(for: ring.agent)
    VStack(spacing: NotchModel.labelGap) {
      ZStack {
        Circle().stroke(color.opacity(highlighted ? 0.35 : 0), lineWidth: 12)
        Circle().stroke(Color.white.opacity(highlighted ? 0.26 : 0.18), lineWidth: 5.8)
        Circle()
          .trim(from: 0, to: min(max(ring.usedPercent / 100, 0), 1))
          .stroke(color, style: StrokeStyle(lineWidth: 3, lineCap: .round))
          .rotationEffect(.degrees(-90))
        if let image = BrandImages.images[ring.agent] {
          Image(nsImage: image).resizable().interpolation(.high).frame(width: 17, height: 17)
        }
      }
      .frame(width: NotchModel.ringDiameter, height: NotchModel.ringDiameter)
      Text("\(Int(ring.usedPercent.rounded()))%")
        .font(.system(size: 14, weight: .semibold))
        .monospacedDigit()
        .foregroundStyle(.white)
        .frame(height: NotchModel.percentLine)
    }
    // A .background never resizes its view, so the -7 padding only grows the highlight.
    .background(
      RoundedRectangle(cornerRadius: 14).fill(Color.white.opacity(highlighted ? 0.08 : 0))
        .padding(-7)
    )
    .opacity(ring.opacity(highlighted: highlighted))
    .animation(.easeOut(duration: 0.15), value: highlighted)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(ring.voiceOverLabel)
  }
}
