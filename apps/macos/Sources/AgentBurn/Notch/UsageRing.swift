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
}

struct UsageRing: View {
  let ring: NotchRing
  var body: some View {
    VStack(spacing: NotchModel.labelGap) {
      ZStack {
        Circle().stroke(Color.white.opacity(0.18), lineWidth: 5.8)
        Circle()
          .trim(from: 0, to: min(max(ring.usedPercent / 100, 0), 1))
          .stroke(
            BurnTheme.color(for: ring.agent), style: StrokeStyle(lineWidth: 3, lineCap: .round)
          )
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
    .opacity(ring.stale ? 0.45 : 1)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(ring.voiceOverLabel)
  }
}
