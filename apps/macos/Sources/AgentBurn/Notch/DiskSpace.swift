import Foundation

struct DiskSpace: Equatable, Sendable {
  let free: Int64
  let total: Int64

  var freePercent: Double { Double(free) / Double(total) * 100 }

  // Free bytes drift on every read. Two readings that draw the same ring share this key.
  // The percent rounds down, so the 15 and 35 cutoffs of remainingTone never fall inside one step.
  var drawn: [String] {
    [
      Self.label(bytes: free, locale: .current), Self.label(bytes: total, locale: .current),
      "\(Int(freePercent))",
    ]
  }

  // statfs took 8 µs here. volumeAvailableCapacityForImportantUsageKey took 12–23 ms a call.
  static func startupDisk() -> DiskSpace? {
    var stats = statfs()
    guard statfs("/", &stats) == 0, stats.f_blocks > 0 else { return nil }
    let block = Int64(stats.f_bsize)
    return DiskSpace(free: Int64(stats.f_bavail) * block, total: Int64(stats.f_blocks) * block)
  }

  func voiceOverLabel(locale: Locale) -> String {
    let free = Self.label(bytes: free, locale: locale, width: .wide)
    return "Startup disk, \(free) free of \(Self.label(bytes: total, locale: locale, width: .wide))"
  }

  static func label(
    bytes: Int64, locale: Locale,
    width: Measurement<UnitInformationStorage>.FormatStyle.UnitWidth = .abbreviated
  ) -> String {
    let gigabytes = Double(bytes) / 1e9
    // The cutoffs sit where the rounded value changes, so 9.97 GB shows "10 GB", not "10.0 GB".
    let (value, unit, digits): (Double, UnitInformationStorage, Int) =
      gigabytes >= 999.5
      ? (gigabytes / 1000, .terabytes, 1)
      : (gigabytes, .gigabytes, gigabytes >= 9.95 ? 0 : 1)
    return Measurement(value: value, unit: unit).formatted(
      .measurement(
        width: width, usage: .asProvided,
        numberFormatStyle: .number.precision(.fractionLength(digits))
      )
      .locale(locale))
  }
}
