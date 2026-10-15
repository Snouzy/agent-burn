import AppKit
import SwiftUI

/// The running apps the memory card can ask to quit, keyed by bundle path.
@MainActor struct RunningApps {
  var canQuit: (String) -> Bool
  var quit: (String) -> Void

  static let workspace = RunningApps(
    canQuit: { !running($0).isEmpty },
    quit: { path in
      for app in running(path) { app.terminate() }
    })

  // Never offer to quit Agent Burn itself or the Finder, which macOS keeps running.
  private static func running(_ bundlePath: String) -> [NSRunningApplication] {
    NSWorkspace.shared.runningApplications.filter {
      $0.bundleURL?.path == bundlePath && $0.processIdentifier != getpid()
        && $0.bundleIdentifier != "com.apple.finder"
    }
  }
}

struct MemoryCard: View {
  let snapshot: MemorySnapshot?
  let quitting: Set<String>
  let canQuit: (MemoryConsumer) -> Bool
  let onQuit: (MemoryConsumer) -> Void

  var body: some View {
    MemoryCardRows(snapshot: snapshot, quitting: quitting, canQuit: canQuit, onQuit: onQuit)
      .padding(NotchModel.cardPadding)
      .frame(width: NotchModel.cardWidth, height: NotchModel.memoryCardHeight, alignment: .top)
      .background(Color.black)
      .clipShape(RoundedRectangle(cornerRadius: 20))
      .foregroundStyle(.white)
      .environment(\.colorScheme, .dark)
  }
}

// Split from MemoryCard so a render test can measure content height, unclipped.
struct MemoryCardRows: View {
  let snapshot: MemorySnapshot?
  let quitting: Set<String>
  let canQuit: (MemoryConsumer) -> Bool
  let onQuit: (MemoryConsumer) -> Void
  @Environment(\.locale) private var locale

  var body: some View {
    VStack(alignment: .leading, spacing: NotchModel.rowGap) {
      header
      if let snapshot {
        summary(snapshot)
        VStack(alignment: .leading, spacing: 0) {
          Text("Top apps by memory")
            .font(.system(size: 11)).foregroundStyle(Color.white.opacity(0.6))
            .frame(height: NotchModel.consumersTitleHeight, alignment: .top)
          ForEach(snapshot.consumers) { consumer in
            ConsumerRow(
              consumer: consumer, quitting: quitting.contains(consumer.id),
              canQuit: canQuit(consumer), onQuit: { onQuit(consumer) })
          }
        }
      } else {
        Text("Reading memory…").font(.system(size: 12)).foregroundStyle(Color.white.opacity(0.6))
      }
    }
  }

  private var header: some View {
    HStack(spacing: 8) {
      Image(systemName: "memorychip").font(.system(size: 15)).frame(width: 18, height: 18)
      Text("Memory").font(.system(size: 14, weight: .semibold))
      Spacer()
      if let snapshot {
        Text("\(bytes(snapshot.breakdown.available)) available")
          .font(.system(size: 12, weight: .medium)).monospacedDigit()
      }
    }
    .frame(height: NotchModel.headerHeight)
  }

  private func summary(_ snapshot: MemorySnapshot) -> some View {
    let breakdown = snapshot.breakdown
    let parts = MemoryCardRows.parts(breakdown)
    return VStack(alignment: .leading, spacing: 7) {
      HStack(spacing: 5) {
        Circle().fill(MemoryStats(available: 0, total: 1, pressure: snapshot.pressure).tone)
          .frame(width: 6, height: 6)
        Text("\(MemoryCardRows.pressureTitle(snapshot.pressure)) · \(bytes(breakdown.total)) total")
          .font(.system(size: 11)).foregroundStyle(Color.white.opacity(0.6))
      }
      GeometryReader { proxy in
        HStack(spacing: 1.5) {
          ForEach(parts, id: \.title) { part in
            Rectangle().fill(part.color)
              .frame(width: proxy.size.width * CGFloat(part.bytes) / CGFloat(breakdown.total))
          }
          Spacer(minLength: 0)
        }
      }
      .frame(height: 8)
      .background(Color.white.opacity(0.12))
      .clipShape(Capsule())
      Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 3) {
        GridRow {
          legend(parts[0])
          legend(parts[1])
        }
        GridRow {
          legend(parts[2])
          legend(parts[3])
        }
      }
    }
    .frame(height: NotchModel.memorySummaryHeight, alignment: .top)
  }

  private func legend(_ part: Part) -> some View {
    HStack(spacing: 5) {
      Circle().fill(part.color).frame(width: 6, height: 6)
      Text(part.title).font(.system(size: 11)).foregroundStyle(Color.white.opacity(0.6))
      Text(bytes(part.bytes)).font(.system(size: 11, weight: .medium)).monospacedDigit()
    }
  }

  private func bytes(_ value: Int64) -> String { MemoryStats.label(bytes: value, locale: locale) }

  struct Part {
    let title: String
    let bytes: Int64
    let color: Color
  }

  /// Activity Monitor's order: what apps hold, what the system pins, then what macOS can reclaim.
  static func parts(_ breakdown: MemoryBreakdown) -> [Part] {
    [
      Part(title: "Apps", bytes: breakdown.app, color: Color(red: 0.35, green: 0.62, blue: 1)),
      Part(title: "Wired", bytes: breakdown.wired, color: .orange),
      Part(title: "Compressed", bytes: breakdown.compressed, color: .purple),
      Part(title: "Cached", bytes: breakdown.cached, color: Color.white.opacity(0.4)),
    ]
  }

  static func pressureTitle(_ pressure: MemoryStats.Pressure) -> String {
    switch pressure {
    case .normal: "Normal pressure"
    case .warning: "Memory pressure high"
    case .critical: "Memory pressure critical"
    }
  }
}

private struct ConsumerRow: View {
  let consumer: MemoryConsumer
  let quitting: Bool
  let canQuit: Bool
  let onQuit: () -> Void
  @State private var hovered = false
  @Environment(\.locale) private var locale

  var body: some View {
    HStack(spacing: 8) {
      AppIcon(path: consumer.bundlePath).frame(width: 20, height: 20)
      VStack(alignment: .leading, spacing: 1) {
        Text(consumer.name).font(.system(size: 12)).lineLimit(1).truncationMode(.middle)
        // An app runs helpers too, such as one per browser tab: its row adds them all up.
        if quitting || consumer.processes > 1 {
          Text(quitting ? "Quitting…" : "\(consumer.processes) processes")
            .font(.system(size: 10)).foregroundStyle(Color.white.opacity(0.5))
        }
      }
      Spacer(minLength: 6)
      if hovered && canQuit && !quitting {
        Button("Quit", action: onQuit)
          .buttonStyle(QuitButtonStyle())
          .help("Quit \(consumer.name)")
      }
      Text(MemoryStats.label(bytes: consumer.footprint, locale: locale))
        .font(.system(size: 12, weight: .medium)).monospacedDigit()
        .frame(minWidth: 52, alignment: .trailing)
    }
    .frame(height: NotchModel.consumerRowHeight)
    .contentShape(Rectangle())
    .opacity(quitting ? 0.45 : 1)
    .onHover { hovered = $0 }
    .accessibilityElement(children: .combine)
    .accessibilityActions {
      if canQuit && !quitting { Button("Quit \(consumer.name)", action: onQuit) }
    }
  }
}

private struct QuitButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .font(.system(size: 11, weight: .medium))
      .padding(.horizontal, 8)
      .frame(height: 18)
      .background(
        Capsule().fill(Color.white.opacity(configuration.isPressed ? 0.35 : 0.2)))
  }
}

private struct AppIcon: View {
  let path: String?

  var body: some View {
    if let path {
      Image(nsImage: AppIcons.icon(for: path)).resizable().interpolation(.high)
    } else {
      Image(systemName: "terminal").font(.system(size: 12))
        .foregroundStyle(Color.white.opacity(0.7))
    }
  }
}

@MainActor private enum AppIcons {
  private static var cache: [String: NSImage] = [:]

  static func icon(for path: String) -> NSImage {
    if let icon = cache[path] { return icon }
    let icon = NSWorkspace.shared.icon(forFile: path)
    cache[path] = icon
    return icon
  }
}
