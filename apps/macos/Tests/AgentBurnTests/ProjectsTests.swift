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
  #expect(
    projects[1].agents == [ProjectAgentUsage(agent: "cursor", totalCost: 0.5, totalTokens: 10)])
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

@Test func projectsPageReadsTheAllHarnessSummary() {
  let projects = SummaryQuery.for(period: .mtd, selection: "projects")
  #expect(projects == SummaryQuery.for(period: .mtd, selection: "summary"))
  #expect(!projects.arguments.contains("--agents"))
  #expect(SummaryQuery.for(period: .rtd, selection: "projects").period == .all)
  #expect(
    SummaryQuery.for(period: .week, selection: "codex").arguments.suffix(2) == [
      "--agents", "codex",
    ])
}

@Test @MainActor func projectsSelectionHasNoHarness() {
  let suite = "AgentBurn.projects.\(UUID().uuidString)"
  let defaults = UserDefaults(suiteName: suite)!
  defer { defaults.removePersistentDomain(forName: suite) }
  defaults.set("/usr/bin/false", forKey: "cliPath")
  let store = UsageStore(
    defaults: defaults,
    storageDirectory: FileManager.default.temporaryDirectory.appendingPathComponent(
      UUID().uuidString))
  store.selection = "projects"
  #expect(store.selectedHarness == nil)
  store.selection = "claude"
  #expect(store.selectedHarness == "claude")
}

private func project(
  _ name: String, path: String?, cost: Double, agents: [(String, Double)] = [],
  kind: String? = nil
) -> ProjectUsage {
  ProjectUsage(
    name: name, path: path, totalCost: cost, totalTokens: UInt64(cost * 100), percentage: 0,
    agents: agents.map { ProjectAgentUsage(agent: $0.0, totalCost: $0.1, totalTokens: 1) },
    kind: kind)
}

@Test func projectsModelSortsByCostWithOtherLast() {
  let model = ProjectsModel(projects: [
    project("Other", path: nil, cost: 50), project("small", path: "/a/small", cost: 1),
    project("big", path: "/a/big", cost: 9),
  ])
  #expect(model.projects.map(\.name) == ["big", "small", "Other"])
  #expect(model.maxCost == 50)
}

@Test func projectsModelFiltersByNameOrPath() {
  let projects = [
    project("agent-burn", path: "/Users/x/dev/perso/agent-burn", cost: 3),
    project("site", path: "/Users/x/work/site", cost: 2),
    project("Other", path: nil, cost: 1),
  ]
  #expect(ProjectsModel(projects: projects, query: "BURN").projects.map(\.name) == ["agent-burn"])
  #expect(ProjectsModel(projects: projects, query: " work ").projects.map(\.name) == ["site"])
  #expect(ProjectsModel(projects: projects, query: "  ").projects.count == 3)
  // Bars keep the same scale while searching.
  #expect(ProjectsModel(projects: projects, query: "site").maxCost == 3)
}

@Test func projectsModelOrdersSegmentsByCostAgainstTheTopProject() {
  let model = ProjectsModel(projects: [
    project("a", path: "/a", cost: 10, agents: [("codex", 2), ("claude", 8)]),
    project("b", path: "/b", cost: 5, agents: [("cursor", 0), ("codex", 5)]),
  ])
  let first = model.segments(for: model.projects[0])
  #expect(first.map(\.agent) == ["claude", "codex"])
  #expect(first.map(\.fraction) == [0.8, 0.2])
  #expect(model.segments(for: model.projects[1]).map(\.agent) == ["codex"])
  #expect(model.segments(for: model.projects[1]).map(\.fraction) == [0.5])
  #expect(ProjectsModel(projects: []).maxCost == 0)
  #expect(model.agents == ["claude", "codex"])
}

@Test func projectPathAbbreviatesTheHomeFolder() {
  #expect(abbreviatedProjectPath("/Users/x/dev/app", home: "/Users/x") == "~/dev/app")
  #expect(abbreviatedProjectPath("/Users/x", home: "/Users/x") == "~")
  #expect(abbreviatedProjectPath("/Users/xy/app", home: "/Users/x") == "/Users/xy/app")
  #expect(abbreviatedProjectPath("/opt/app", home: "/Users/x/") == "/opt/app")
}

@Test func projectKindDecodesWithFallbackForOldReports() throws {
  let projects = try #require(
    try summary(
      projects: """
        [{"name":"app","path":"/a/app","kind":"repository","totalCost":3,"totalTokens":3,"percentage":60,"agents":[]},
         {"name":"Scratch","path":null,"kind":"scratch","totalCost":1,"totalTokens":1,"percentage":20,"agents":[]},
         {"name":"Other","path":null,"kind":"other","totalCost":1,"totalTokens":1,"percentage":20,"agents":[]}]
        """
    ).projects)
  #expect(projects.map(\.category) == [.repository, .scratch, .other])
  #expect(Set(projects.map(\.id)).count == 3)
  let old = try #require(try summary(projects: projectsJSON).projects)
  #expect(old.map(\.category) == [.repository, .other])
  #expect(project("x", path: nil, cost: 1, kind: "future").category == .other)
}

@Test func projectsModelPutsScratchThenOtherAfterRepositories() {
  let model = ProjectsModel(projects: [
    project("Other", path: nil, cost: 50, kind: "other"),
    project("Scratch", path: nil, cost: 70, kind: "scratch"),
    project("small", path: "/a/small", cost: 1, kind: "repository"),
    project("big", path: "/a/big", cost: 9),
  ])
  #expect(model.projects.map(\.name) == ["big", "small", "Scratch", "Other"])
}

@Test func topProjectsListOnlyRepositories() {
  let repositories = ProjectsModel.repositories([
    project("Scratch", path: nil, cost: 70, kind: "scratch"),
    project("Other", path: nil, cost: 50),
    project("app", path: "/a/app", cost: 2),
  ])
  #expect(repositories.map(\.name) == ["app"])
}
