import Foundation
import Testing

@testable import AgentBurn

private func summary(projects: String? = nil) throws -> SummaryReport {
  let field = projects.map { ",\"projects\":" + $0 } ?? ""
  return try JSONDecoder().decode(
    SummaryReport.self,
    from: Data(
      """
      {"totals":{"totalCost":12.8,"totalTokens":4577},"agents":[],"models":[]\(field)}
      """.utf8))
}

private let projectsJSON = """
  [{"name":"agent-burn","path":"/Users/x/dev/perso/agent-burn","totalCost":12.3,"totalTokens":4567,
    "percentage":41.2,"agents":[{"agent":"claude","totalCost":10.1,"totalTokens":4000},
    {"agent":"codex","totalCost":2.2,"totalTokens":567}]},
   {"name":"Other","path":null,"totalCost":0.5,"totalTokens":10,"percentage":1.1,
    "agents":[{"agent":"cursor","totalCost":0.5,"totalTokens":10}]}]
  """

@Test func summaryDecodesProjects() throws {
  let projects = try #require(try summary(projects: projectsJSON).projects)
  #expect(projects.map(\.id) == ["/Users/x/dev/perso/agent-burn", "other"])
  #expect(projects[0].name == "agent-burn")
  #expect(projects[0].totalTokens == 4567)
  #expect(projects[0].percentage == 41.2)
  #expect(projects[0].agents.map(\.agent) == ["claude", "codex"])
  #expect(projects[1].path == nil)
  #expect(projects[1].agents == [ProjectAgentUsage(agent: "cursor", totalCost: 0.5, totalTokens: 10)])
}

@Test func summaryWithoutProjectsStillDecodes() throws {
  #expect(try summary().projects == nil)
}

@Test func cachedReportFromBeforeProjectsStillDecodes() throws {
  let cache = try JSONDecoder().decode(
    ReportCache.self,
    from: Data(
      """
      {"source":"s","summaries":{"all":{"date":0,"report":{"totals":{"totalCost":1,"totalTokens":2},
      "agents":[{"agent":"codex","totalCost":1,"totalTokens":2}],"models":[]}}},"harnesses":{}}
      """.utf8))
  #expect(cache.summaries["all"]?.report.projects == nil)
  #expect(cache.agentNames == ["codex"])
}

@Test func projectsSurviveACacheRoundTrip() throws {
  let report = try summary(projects: projectsJSON)
  let decoded = try JSONDecoder().decode(
    SummaryReport.self, from: JSONEncoder().encode(report))
  #expect(decoded.projects == report.projects)
}
