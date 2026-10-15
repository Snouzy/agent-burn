import Darwin
import Foundation

struct QuotaCollectorConfig: Codable, Sendable {
  let customPath: String
  let codexHomes: String
  var source: String { customPath + "|" + codexHomes }
  static var directory: URL {
    FileManager.default.homeDirectoryForCurrentUser
      .appendingPathComponent("Library/Application Support/Agent Burn")
  }

  func save(directory: URL) throws {
    try FileManager.default.createDirectory(
      at: directory, withIntermediateDirectories: true,
      attributes: [.posixPermissions: 0o700])
    try JSONEncoder().encode(self).write(
      to: directory.appendingPathComponent("quota-collector.json"), options: .atomic)
  }
}

private struct CollectedQuota: Codable, Sendable {
  let agent: String
  let observedAt: Double
  let window: QuotaWindow?
}

private struct CollectedHarnessQuota: Codable, Sendable {
  let agent: String
  let observedAt: Double
  let window: QuotaWindow
  var sessionWindow: QuotaWindow? = nil
}

/// Claude's and Kimi's 5-hour session meters are stored as their own series beside the
/// main one. These names key persisted history, so they must never change.
/// Agents with a `harness` subscription view and a live quota collected by `harness <agent>`.
let harnessAgents = ["codex", "claude", "kimi"]

let claudeSessionQuotaAgent = "claude-session"
let kimiSessionQuotaAgent = "kimi-session"

func sessionQuotaAgent(for agent: String) -> String? {
  switch agent {
  case "claude": claudeSessionQuotaAgent
  case "kimi": kimiSessionQuotaAgent
  default: nil
  }
}

enum QuotaCollector {
  /// launchd owns scheduling; this process performs one bounded collection and exits.
  static func collect(directory: URL = QuotaCollectorConfig.directory) async throws {
    let config = try JSONDecoder().decode(
      QuotaCollectorConfig.self,
      from: Data(contentsOf: directory.appendingPathComponent("quota-collector.json")))
    // Also protect manual invocations and app upgrades from overlapping writes.
    let descriptor = open(
      directory.appendingPathComponent("quota-collector.lock").path,
      O_CREAT | O_RDWR | O_CLOEXEC, 0o600)
    guard descriptor >= 0 else { throw CocoaError(.fileWriteUnknown) }
    defer { close(descriptor) }
    guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else { return }
    defer { flock(descriptor, LOCK_UN) }
    let file = QuotaHistoryFile(directory: directory)
    var history = try file.load() ?? QuotaHistory()
    await withTaskGroup(of: (String, [QuotaReading], String?).self) { group in
      for agent in harnessAgents {
        group.addTask {
          do {
            let executable = try CLIClient.executable(customPath: config.customPath)
            let collected = try await CLIClient.read(
              CollectedHarnessQuota.self,
              executable: executable, arguments: ["harness", agent], offline: false,
              environment: ["CODEX_HOME": config.codexHomes, "AGENT_BURN_QUOTA_ONLY": "1"],
              timeout: 45)
            let reading = QuotaReading(
              agent: agent, observedAt: collected.observedAt, window: collected.window)
            guard collected.agent == agent, reading.observedAt.isFinite, reading.window.isValid,
              abs(reading.date.timeIntervalSinceNow) <= 90
            else { throw CLIError.invalidOutput }
            var readings = [reading]
            if let sessionAgent = sessionQuotaAgent(for: agent),
              let session = collected.sessionWindow, session.isValid
            {
              readings.append(
                QuotaReading(
                  agent: sessionAgent, observedAt: collected.observedAt, window: session))
            }
            return (agent, readings, nil)
          } catch {
            return (agent, [], "Live quota could not be collected. The last reading is preserved.")
          }
        }
      }
      group.addTask {
        do {
          let executable = try CLIClient.executable(customPath: config.customPath)
          let collected = try await CLIClient.read(
            CollectedQuota.self,
            executable: executable, arguments: ["summary", "--value"], offline: false,
            environment: ["CODEX_HOME": config.codexHomes, "AGENT_BURN_QUOTA_ONLY": "1"],
            timeout: 45)
          guard collected.agent == "cursor", collected.observedAt.isFinite,
            abs(Date(timeIntervalSince1970: collected.observedAt / 1000).timeIntervalSinceNow)
              <= 90
          else { throw CLIError.invalidOutput }
          guard let window = collected.window, window.isValid else {
            return ("cursor", [], nil)
          }
          return (
            "cursor",
            [QuotaReading(agent: "cursor", observedAt: collected.observedAt, window: window)],
            nil
          )
        } catch {
          return (
            "cursor", [],
            "Live Cursor credits could not be collected. The last reading is preserved."
          )
        }
      }
      for await (agent, readings, error) in group {
        for reading in readings { history.record(reading, source: config.source) }
        if let error {
          history.fail(agent: agent, source: config.source, message: error)
        } else if readings.isEmpty {
          history.failures[config.source]?[agent] = nil
        }
        // Persist each provider immediately, even if the other hangs or this process crashes.
        do { try file.save(history) } catch {
          FileHandle.standardError.write(Data("Quota history could not be saved.\n".utf8))
        }
      }
    }
    try file.save(history)
  }
}
