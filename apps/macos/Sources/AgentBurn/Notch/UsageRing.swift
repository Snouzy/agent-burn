// Adapted from Codenotch (github.com/vinzdg/codenotch), MIT License, Copyright (c) 2026 Vinz.
import SwiftUI

struct NotchRing: Equatable, Identifiable, Sendable {
  let agent: String
  let usedPercent: Double
  let stale: Bool
  let style: QuotaMeterStyle
  /// The 5-hour session window, drawn as an inner ring when present.
  var sessionUsedPercent: Double? = nil
  var id: String { agent }
  /// Whichever window runs out first is the one worth reading at a glance.
  var label: String { "\(percent(max(usedPercent, sessionUsedPercent ?? usedPercent)))%" }
  var voiceOverLabel: String {
    guard let sessionUsedPercent else {
      return "\(harnessName(agent)), \(style.title), \(percent(usedPercent)) percent used"
    }
    return "\(harnessName(agent)), \(style.windowName) \(percent(usedPercent)) percent used, "
      + "5-hour \(percent(sessionUsedPercent)) percent used"
  }
  private func percent(_ value: Double) -> Int { Int(value.rounded()) }
  func opacity(highlighted: Bool) -> Double { stale && !highlighted ? 0.45 : 1 }
}

struct UsageRing: View {
  let ring: NotchRing
  let highlighted: Bool
  var metrics = NotchMetrics(scale: 1)
  var onRemove: (() -> Void)? = nil
  var body: some View {
    RingCell(
      progress: ring.usedPercent / 100, color: BurnTheme.color(for: ring.agent),
      label: ring.label, highlighted: highlighted,
      opacity: ring.opacity(highlighted: highlighted), voiceOverLabel: ring.voiceOverLabel,
      metrics: metrics, innerProgress: ring.sessionUsedPercent.map { $0 / 100 },
      onRemove: onRemove
    ) {
      if let image = BrandImages.images[ring.agent] {
        let size = ring.sessionUsedPercent == nil ? metrics.iconSize : metrics.doubleRingIconSize
        Image(nsImage: image).resizable().interpolation(.high)
          .frame(width: size, height: size)
      }
    }
  }
}

struct DiskRing: View {
  let disk: DiskSpace
  let highlighted: Bool
  var metrics = NotchMetrics(scale: 1)
  var onRemove: (() -> Void)? = nil
  @Environment(\.locale) private var locale

  var body: some View {
    RingCell(
      progress: 1 - disk.freePercent / 100, color: BurnTheme.remainingTone(disk.freePercent),
      label: DiskSpace.label(bytes: disk.free, locale: locale), highlighted: highlighted,
      opacity: 1, voiceOverLabel: disk.voiceOverLabel(locale: locale), metrics: metrics,
      onRemove: onRemove
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
  var onRemove: (() -> Void)? = nil
  @Environment(\.locale) private var locale

  var body: some View {
    RingCell(
      progress: 1 - memory.availablePercent / 100, color: memory.tone,
      label: MemoryStats.label(bytes: memory.available, locale: locale), highlighted: highlighted,
      opacity: 1, voiceOverLabel: memory.voiceOverLabel(locale: locale), metrics: metrics,
      onRemove: onRemove
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
  var innerProgress: Double? = nil
  var onRemove: (() -> Void)? = nil
  @ViewBuilder let center: Center

  var body: some View {
    let scale = metrics.scale
    VStack(spacing: metrics.labelGap) {
      ZStack {
        Circle().stroke(color.opacity(highlighted ? 0.35 : 0), lineWidth: 12 * scale)
        Circle().stroke(
          Color.white.opacity(highlighted ? 0.26 : 0.18), lineWidth: metrics.ringTrackWidth)
        Circle()
          .trim(from: 0, to: min(max(progress, 0), 1))
          .stroke(color, style: StrokeStyle(lineWidth: 3 * scale, lineCap: .round))
          .rotationEffect(.degrees(-90))
        if let innerProgress {
          ZStack {
            Circle().stroke(
              Color.white.opacity(highlighted ? 0.2 : 0.12), lineWidth: metrics.innerRingTrackWidth)
            Circle()
              .trim(from: 0, to: min(max(innerProgress, 0), 1))
              .stroke(
                color.opacity(0.75), style: StrokeStyle(lineWidth: 2 * scale, lineCap: .round)
              )
              .rotationEffect(.degrees(-90))
          }
          .frame(width: metrics.innerRingDiameter, height: metrics.innerRingDiameter)
        }
        center
      }
      .frame(width: metrics.ringDiameter, height: metrics.ringDiameter)
      // An overlay never resizes the cell. The badge stays inside it, or hovering it would end the hover.
      .overlay(alignment: .topTrailing) {
        if let onRemove {
          RemoveBadge(metrics: metrics, action: onRemove)
            .offset(x: 4 * scale)
            .opacity(highlighted ? 1 : 0)
            .allowsHitTesting(highlighted)
        }
      }
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
    .accessibilityActions {
      if let onRemove { Button("Hide from notch", action: onRemove) }
    }
  }
}

/// A small Launchpad-style delete badge, shown while its ring is hovered.
private struct RemoveBadge: View {
  let metrics: NotchMetrics
  let action: () -> Void
  @State private var hovered = false

  var body: some View {
    let size = 14 * metrics.scale
    Button(action: action) {
      Image(systemName: "xmark")
        .font(.system(size: 6.5 * metrics.scale, weight: .bold))
        .foregroundStyle(.white.opacity(hovered ? 1 : 0.8))
        .frame(width: size, height: size)
        .background(Circle().fill(Color(white: hovered ? 0.4 : 0.28)))
        .overlay(Circle().stroke(.black.opacity(0.6), lineWidth: 1))
    }
    .buttonStyle(.plain)
    .contentShape(Circle())
    .onHover { hovered = $0 }
    .help("Hide from notch")
  }
}
