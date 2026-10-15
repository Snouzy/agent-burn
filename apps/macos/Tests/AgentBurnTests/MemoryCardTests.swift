import Foundation
import Testing

@testable import AgentBurn

@Test func memoryLabelsCountInBinaryUnitsLikeActivityMonitor() {
  let us = Locale(identifier: "en_US")
  let gib: Int64 = 1_073_741_824
  #expect(MemoryStats.label(bytes: 128 * gib, locale: us) == "128 GB")
  #expect(MemoryStats.label(bytes: 9_210_000_000, locale: us) == "8.6 GB")
  #expect(MemoryStats.label(bytes: 560 * 1_048_576, locale: us) == "560 MB")
  #expect(MemoryStats.label(bytes: gib - 1_048_576, locale: us) == "1.0 GB")
}

@Test func memoryLabelsSwitchToGigabytesBeforeAThousandMegabytes() {
  let us = Locale(identifier: "en_US")
  #expect(MemoryStats.label(bytes: 999 * 1_048_576, locale: us) == "999 MB")
  #expect(MemoryStats.label(bytes: 1_001 * 1_048_576, locale: us) == "1.0 GB")
}
