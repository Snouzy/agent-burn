import Darwin
import SwiftUI

struct MemoryStats: Equatable, Sendable {
  enum Pressure: Equatable, Sendable {
    case normal, warning, critical
  }

  let available: Int64
  let total: Int64
  let pressure: Pressure

  var availablePercent: Double { Double(available) / Double(total) * 100 }

  // Free memory drifts on every read. Two readings that draw the same ring share this key.
  var drawn: [String] {
    [
      Self.label(bytes: available, locale: .current), "\(Int(availablePercent))",
      "\(pressure)",
    ]
  }

  // macOS fills idle RAM with cache, so the ring takes the kernel's pressure level for its color.
  var tone: Color {
    switch pressure {
    case .normal: .green
    case .warning: .orange
    case .critical: BurnTheme.behind
    }
  }

  // Available is total minus Activity Monitor's "Memory Used": app memory, wired and compressed.
  static func system() -> MemoryStats? {
    var stats = vm_statistics64()
    var count = mach_msg_type_number_t(
      MemoryLayout<vm_statistics64>.size / MemoryLayout<integer_t>.size)
    let result = withUnsafeMutablePointer(to: &stats) {
      $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
        host_statistics64(mach_host_self(), HOST_VM_INFO64, $0, &count)
      }
    }
    var total: UInt64 = 0
    var size = MemoryLayout<UInt64>.size
    var page: vm_size_t = 0
    guard result == KERN_SUCCESS, sysctlbyname("hw.memsize", &total, &size, nil, 0) == 0,
      total > 0, host_page_size(mach_host_self(), &page) == KERN_SUCCESS
    else { return nil }
    let anonymous = UInt64(stats.internal_page_count)
    let app = anonymous - min(UInt64(stats.purgeable_count), anonymous)
    let wired = UInt64(stats.wire_count)
    let used = (app + wired + UInt64(stats.compressor_page_count)) * UInt64(page)
    return MemoryStats(
      available: Int64(total - min(used, total)), total: Int64(total), pressure: pressureLevel())
  }

  static func pressure(level: Int32) -> Pressure {
    level >= 4 ? .critical : level >= 2 ? .warning : .normal
  }

  private static func pressureLevel() -> Pressure {
    var level: Int32 = 0
    var size = MemoryLayout<Int32>.size
    guard sysctlbyname("kern.memorystatus_vm_pressure_level", &level, &size, nil, 0) == 0 else {
      return .normal
    }
    return pressure(level: level)
  }

  /// Memory counts in binary units, as Activity Monitor does: 128 GB of RAM, not 137 GB.
  /// Below 1 GB it shows whole megabytes, like a background helper's 560 MB.
  static func label(
    bytes: Int64, locale: Locale,
    width: Measurement<UnitInformationStorage>.FormatStyle.UnitWidth = .abbreviated
  ) -> String {
    let gibibytes = Double(bytes) / 1_073_741_824
    // 1,000 MB and up reads as 1.0 GB, never "1,001 MB".
    guard (gibibytes * 1024).rounded() < 1000 else {
      return DiskSpace.label(bytes: Int64(gibibytes * 1e9), locale: locale, width: width)
    }
    return Measurement(value: (gibibytes * 1024).rounded(), unit: UnitInformationStorage.megabytes)
      .formatted(
        .measurement(
          width: width, usage: .asProvided, numberFormatStyle: .number.precision(.fractionLength(0))
        )
        .locale(locale))
  }

  func voiceOverLabel(locale: Locale) -> String {
    let available = Self.label(bytes: available, locale: locale, width: .wide)
    let total = Self.label(bytes: total, locale: locale, width: .wide)
    return "Memory, \(available) available of \(total), \(pressure) pressure"
  }
}
