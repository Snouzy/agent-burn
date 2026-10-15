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
  var metrics = NotchMetrics(scale: 1)
  var body: some View {
    RingCell(
      progress: ring.usedPercent / 100, color: BurnTheme.color(for: ring.agent),
      label: "\(Int(ring.usedPercent.rounded()))%", highlighted: highlighted,
      opacity: ring.opacity(highlighted: highlighted), voiceOverLabel: ring.voiceOverLabel,
      metrics: metrics
    ) {
      if let image = BrandImages.images[ring.agent] {
        Image(nsImage: image).resizable().interpolation(.high)
          .frame(width: metrics.iconSize, height: metrics.iconSize)
      }
    }
  }
}

struct DiskRing: View {
  let disk: DiskSpace
  let highlighted: Bool
  var metrics = NotchMetrics(scale: 1)
  @Environment(\.locale) private var locale

  var body: some View {
    RingCell(
      progress: 1 - disk.freePercent / 100, color: BurnTheme.remainingTone(disk.freePercent),
      label: DiskSpace.label(bytes: disk.free, locale: locale), highlighted: highlighted,
      opacity: 1, voiceOverLabel: disk.voiceOverLabel(locale: locale), metrics: metrics
    ) {
      Image(systemName: "internaldrive").font(.system(size: metrics.labelSize))
        .foregroundStyle(.white)
    }
  }
}

struct MemoryRing: View {
  let memory: MemoryStats
  let highlighted: Bool
  var metrics = NotchMetrics(scale: 1)
  @Environment(\.locale) private var locale

  var body: some View {
    RingCell(
      progress: 1 - memory.availablePercent / 100, color: memory.tone,
      label: DiskSpace.label(bytes: memory.available, locale: locale), highlighted: highlighted,
      opacity: 1, voiceOverLabel: memory.voiceOverLabel(locale: locale), metrics: metrics
    ) {
      Image(systemName: "memorychip").font(.system(size: metrics.labelSize))
        .foregroundStyle(.white)
    }
  }
}

private struct RingCell<Center: View>: View {
  let progress: Double
  let color: Color
  let label: String
  let highlighted: Bool
  let opacity: Double
  let voiceOverLabel: String
  let metrics: NotchMetrics
  @ViewBuilder let center: Center

  var body: some View {
    let scale = metrics.scale
    VStack(spacing: metrics.labelGap) {
      ZStack {
        Circle().stroke(color.opacity(highlighted ? 0.35 : 0), lineWidth: 12 * scale)
        Circle().stroke(Color.white.opacity(highlighted ? 0.26 : 0.18), lineWidth: 5.8 * scale)
        Circle()
          .trim(from: 0, to: min(max(progress, 0), 1))
          .stroke(color, style: StrokeStyle(lineWidth: 3 * scale, lineCap: .round))
          .rotationEffect(.degrees(-90))
        center
      }
      .frame(width: metrics.ringDiameter, height: metrics.ringDiameter)
      Text(label)
        .font(.system(size: metrics.labelSize, weight: .semibold))
        .monospacedDigit()
        .foregroundStyle(.white)
        .frame(height: metrics.percentLine)
    }
    // A .background never resizes its view, so the -7 padding only grows the highlight.
    .background(
      RoundedRectangle(cornerRadius: 14 * scale)
        .fill(Color.white.opacity(highlighted ? 0.08 : 0))
        .padding(-7 * scale)
    )
    .opacity(opacity)
    .animation(.easeOut(duration: 0.15), value: highlighted)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(voiceOverLabel)
  }
}
