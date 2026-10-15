import Darwin
import Foundation

/// One app, or one bare executable, and the memory all its processes use.
struct MemoryConsumer: Equatable, Identifiable, Sendable {
  /// The outermost `.app` bundle path, or the executable path.
  let id: String
  let name: String
  let footprint: Int64
  let processes: Int
  var bundlePath: String? { id.hasSuffix(".app") ? id : nil }
}

/// What the memory hover card shows: the breakdown and the apps using the most memory.
struct MemorySnapshot: Equatable, Sendable {
  let breakdown: MemoryBreakdown
  let pressure: MemoryStats.Pressure
  let consumers: [MemoryConsumer]

  static let consumerLimit = 5

  /// About 20 ms for a thousand processes: call it off the main actor.
  static func read() -> MemorySnapshot? {
    guard let breakdown = MemoryBreakdown.system() else { return nil }
    return MemorySnapshot(
      breakdown: breakdown, pressure: MemoryStats.pressureLevel(),
      consumers: MemoryConsumers.top(MemoryConsumers.processes(), limit: consumerLimit))
  }
}

enum MemoryConsumers {
  struct Process: Equatable, Sendable {
    let path: String
    let footprint: UInt64
  }

  /// Helpers live inside their app's bundle, so the outermost `.app` groups them, the way
  /// Activity Monitor and the Dock name them. A bare executable stands on its own.
  static func top(_ processes: [Process], limit: Int) -> [MemoryConsumer] {
    var groups: [String: (footprint: UInt64, processes: Int)] = [:]
    for process in processes where process.footprint > 0 {
      let key = groupPath(process.path)
      groups[key, default: (0, 0)].footprint += process.footprint
      groups[key, default: (0, 0)].processes += 1
    }
    return groups.map { path, group in
      MemoryConsumer(
        id: path, name: displayName(path), footprint: Int64(clamping: group.footprint),
        processes: group.processes)
    }
    .sorted { $0.footprint != $1.footprint ? $0.footprint > $1.footprint : $0.name < $1.name }
    .prefix(limit).map { $0 }
  }

  static func groupPath(_ path: String) -> String {
    guard let range = path.range(of: ".app/") else { return path }
    return String(path[..<range.lowerBound]) + ".app"
  }

  static func displayName(_ path: String) -> String {
    let name = (path as NSString).lastPathComponent
    return name.hasSuffix(".app") ? String(name.dropLast(4)) : name
  }

  /// Every process this user may inspect. Other users' processes, root's included, are skipped:
  /// reading them needs privileges, and the apps worth quitting run as this user anyway.
  static func processes() -> [Process] {
    let capacity = Int(proc_listallpids(nil, 0)) + 64
    guard capacity > 64 else { return [] }
    var pids = [pid_t](repeating: 0, count: capacity)
    let count = Int(proc_listallpids(&pids, Int32(capacity * MemoryLayout<pid_t>.size)))
    var path = [CChar](repeating: 0, count: Int(MAXPATHLEN) * 4)
    return pids.prefix(max(count, 0)).compactMap { pid in
      guard pid > 0, let footprint = footprint(pid) else { return nil }
      let length = proc_pidpath(pid, &path, UInt32(path.count))
      guard length > 0 else { return nil }
      return Process(path: String(cString: path), footprint: footprint)
    }
  }

  /// Activity Monitor's Memory column.
  private static func footprint(_ pid: pid_t) -> UInt64? {
    var info = rusage_info_v4()
    let result = withUnsafeMutablePointer(to: &info) {
      $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) {
        proc_pid_rusage(pid, RUSAGE_INFO_V4, $0)
      }
    }
    return result == 0 ? info.ri_phys_footprint : nil
  }
}
