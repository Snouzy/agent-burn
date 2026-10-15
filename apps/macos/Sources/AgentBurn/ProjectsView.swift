import SwiftUI

struct ProjectBarSegment: Identifiable, Equatable {
  let agent: String
  let cost: Double
  /// Width relative to the most expensive project, so rows share one scale.
  let fraction: Double
  var id: String { agent }
}

struct ProjectsModel {
  let projects: [ProjectUsage]
  let maxCost: Double
  /// Agents with spend in any project, most expensive first, for the legend.
  let agents: [String]

  init(projects all: [ProjectUsage], query: String = "") {
    let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
    // "Other" has no directory: keep it last whatever it costs.
    projects = all.filter {
      needle.isEmpty || $0.name.localizedCaseInsensitiveContains(needle)
        || ($0.path?.localizedCaseInsensitiveContains(needle) ?? false)
    }
    .sorted {
      ($0.path == nil ? 1 : 0, -$0.totalCost, $0.name)
        < ($1.path == nil ? 1 : 0, -$1.totalCost, $1.name)
    }
    maxCost = all.map(\.totalCost).max() ?? 0
    let spend = Dictionary(
      all.flatMap(\.agents).map { ($0.agent, $0.totalCost) }, uniquingKeysWith: +)
    agents = spend.filter { $0.value > 0 }
      .sorted { ($0.value, $1.key) > ($1.value, $0.key) }.map(\.key)
  }

  func segments(for project: ProjectUsage) -> [ProjectBarSegment] {
    guard maxCost > 0 else { return [] }
    return project.agents.filter { $0.totalCost > 0 }
      .sorted { $0.totalCost > $1.totalCost }
      .map {
        ProjectBarSegment(agent: $0.agent, cost: $0.totalCost, fraction: $0.totalCost / maxCost)
      }
  }
}

func abbreviatedProjectPath(
  _ path: String, home: String = FileManager.default.homeDirectoryForCurrentUser.path
) -> String {
  let home = home.hasSuffix("/") ? String(home.dropLast()) : home
  if path == home { return "~" }
  return path.hasPrefix(home + "/") ? "~" + path.dropFirst(home.count) : path
}

let otherProjectHelp =
  "Sessions whose agent did not record a working directory are grouped here."

struct ProjectsView: View {
  @Environment(UsageStore.self) private var store
  @State private var search = ""

  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack(alignment: .center, spacing: 12) {
        IconBadge(symbol: "folder.fill", size: 42)
        VStack(alignment: .leading, spacing: 3) {
          Text("Projects").font(.system(size: 24, weight: .bold, design: .rounded))
          Text("\(store.period.label) · spend by working directory across every harness")
            .font(.system(size: 12)).foregroundStyle(.secondary)
        }
      }
      if store.summary == nil, let error = store.errors["summary"] {
        ReportNotice(message: error)
      }
      PeriodBar()
      if let projects = store.summary?.projects {
        list(ProjectsModel(projects: projects, query: search), total: projects.count)
      } else {
        ContentUnavailableView {
          Label(
            store.isLoading ? "Loading projects" : "Projects are not loaded yet",
            systemImage: "folder")
        } description: {
          Text("Refresh to load spend for each project folder.")
        } actions: {
          Button("Refresh") { Task { await store.refresh() } }.disabled(store.isLoading)
        }
        .frame(maxWidth: .infinity, minHeight: 380)
      }
    }
  }

  private func list(_ model: ProjectsModel, total: Int) -> some View {
    VStack(alignment: .leading, spacing: 12) {
      CardHeader(title: "All projects", symbol: "folder.fill") {
        HStack(spacing: 10) {
          Text(total == 1 ? "1 project" : "\(total) projects")
          HStack(spacing: 5) {
            Image(systemName: "magnifyingglass").foregroundStyle(.tertiary)
            TextField("Filter projects", text: $search).textFieldStyle(.plain)
              .font(.system(size: 12))
          }
          .padding(.horizontal, 9).padding(.vertical, 5)
          .frame(width: 200)
          .background(BurnTheme.track, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        }
      }
      if model.agents.count > 1 { ProjectLegend(agents: model.agents) }
      if model.projects.isEmpty {
        Label(
          total == 0 ? "No project usage recorded in this period." : "No projects match “\(search)”.",
          systemImage: "folder"
        )
        .font(.system(size: 12)).foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, minHeight: 80)
      } else {
        VStack(spacing: 2) {
          ProjectColumns()
          ForEach(model.projects) { project in
            ProjectRow(project: project, segments: model.segments(for: project))
          }
        }
        .padding(.horizontal, -8)
      }
    }
    .burnCard()
  }
}

/// Top projects for the period, linking to the full Projects page.
struct TopProjectsCard: View {
  @Environment(UsageStore.self) private var store
  let projects: [ProjectUsage]
  var body: some View {
    let model = ProjectsModel(projects: projects)
    VStack(alignment: .leading, spacing: 12) {
      CardHeader(title: "Top projects", symbol: "folder.fill") {
        HStack(spacing: 10) {
          Text("\(projects.count) active")
          Button {
            store.selection = "projects"
          } label: {
            HStack(spacing: 3) {
              Text("See all")
              Image(systemName: "chevron.right").font(.system(size: 9, weight: .semibold))
            }
            .foregroundStyle(BurnTheme.flameLabel)
          }
          .buttonStyle(.plain)
          .help("Open Projects")
        }
      }
      VStack(spacing: 2) {
        ForEach(model.projects.prefix(5)) { project in
          ProjectRow(project: project, segments: model.segments(for: project), compact: true)
        }
      }
      .padding(.horizontal, -8)
    }
    .burnCard()
  }
}

private struct ProjectLegend: View {
  let agents: [String]
  var body: some View {
    HStack(spacing: 14) {
      ForEach(agents.prefix(8), id: \.self) { agent in
        HStack(spacing: 5) {
          Circle().fill(BurnTheme.color(for: agent)).frame(width: 7, height: 7)
          Text(harnessName(agent))
        }
        .fixedSize()
      }
      if agents.count > 8 {
        Text("+\(agents.count - 8)").fixedSize()
          .help(agents.dropFirst(8).map(harnessName).joined(separator: ", "))
      }
      Spacer(minLength: 0)
    }
    .font(.system(size: 11)).foregroundStyle(.secondary)
    .accessibilityElement(children: .combine)
  }
}

/// Column titles aligned with `ProjectRow`'s fixed widths.
private struct ProjectColumns: View {
  var body: some View {
    HStack(spacing: 12) {
      Text("Project").padding(.leading, 38).frame(width: 298, alignment: .leading)
      Text("By harness").frame(maxWidth: .infinity, alignment: .leading)
      Text("Spend").frame(width: 84, alignment: .trailing)
      Text("Tokens").frame(width: 64, alignment: .trailing)
      Text("Share").frame(width: 48, alignment: .trailing)
    }
    .font(.system(size: 10, weight: .medium)).foregroundStyle(.tertiary)
    .padding(.horizontal, 8).padding(.bottom, 2)
    .accessibilityHidden(true)
  }
}

private struct ProjectRow: View {
  let project: ProjectUsage
  let segments: [ProjectBarSegment]
  var compact = false
  private var isOther: Bool { project.path == nil }

  var body: some View {
    HStack(spacing: 12) {
      IconBadge(
        symbol: isOther ? "questionmark.folder.fill" : "folder.fill",
        tint: isOther ? .gray : BurnTheme.flame, size: 26)
      VStack(alignment: .leading, spacing: 2) {
        Text(project.name).font(.system(size: 12, weight: .medium)).lineLimit(1)
        Text(project.path.map { abbreviatedProjectPath($0) } ?? "No working directory")
          .font(.system(size: 10)).foregroundStyle(.secondary)
          .lineLimit(1).truncationMode(.middle)
      }
      .frame(width: compact ? 220 : 260, alignment: .leading)
      .help(project.path ?? otherProjectHelp)
      ProjectBar(segments: segments)
        .help(
          segments.map { "\(harnessName($0.agent)) \(currency($0.cost))" }
            .joined(separator: " · "))
      Text(currency(project.totalCost)).font(.system(size: 12, weight: .semibold))
        .frame(width: 84, alignment: .trailing)
      if !compact {
        Text(tokens(project.totalTokens)).font(.system(size: 11)).foregroundStyle(.secondary)
          .frame(width: 64, alignment: .trailing)
      }
      Text(
        (project.percentage / 100).formatted(.percent.precision(.fractionLength(1)))
      )
      .font(.system(size: 11)).foregroundStyle(.secondary)
      .frame(width: 48, alignment: .trailing)
    }
    .monospacedDigit()
    .hoverRow()
    .accessibilityElement(children: .combine)
  }
}

/// One capsule per project, split by harness, scaled to the most expensive project.
private struct ProjectBar: View {
  let segments: [ProjectBarSegment]
  var body: some View {
    GeometryReader { geometry in
      ZStack(alignment: .leading) {
        Capsule().fill(BurnTheme.track)
        HStack(spacing: 1) {
          ForEach(segments) { segment in
            Rectangle().fill(BurnTheme.color(for: segment.agent).gradient)
              .frame(width: max(2, geometry.size.width * segment.fraction - 1))
          }
        }
        .clipShape(Capsule())
      }
    }
    .frame(height: 6)
    .frame(maxWidth: .infinity)
    .accessibilityHidden(true)
  }
}
