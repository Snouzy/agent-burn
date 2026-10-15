use serde_json::{Value, json};

use super::{AllRow, percentage};
use crate::{
    fast::FxHashMap,
    json_float,
    project_root::{Project, ProjectResolver},
};

const OTHER_PROJECT_NAME: &str = "Other";

struct ProjectTotal {
    name: String,
    path: Option<String>,
    cost: f64,
    tokens: u64,
    agents: Vec<AgentShare>,
}

struct AgentShare {
    agent: &'static str,
    cost: f64,
    tokens: u64,
}

/// Cost per project, grouping session working directories by git repository.
/// Usage without a working directory is reported as a trailing "Other" entry.
pub(super) fn projects_json(rows: &[AllRow]) -> Value {
    projects_json_with(rows, ProjectResolver::new(crate::home::home_dir()))
}

fn projects_json_with(rows: &[AllRow], mut resolver: ProjectResolver) -> Value {
    let agent_rows = rows
        .iter()
        .flat_map(|row| match row.agent_breakdowns.as_ref() {
            Some(breakdowns) => breakdowns.iter().collect::<Vec<_>>(),
            None if row.agent != "all" => vec![row],
            None => Vec::new(),
        })
        .collect::<Vec<_>>();
    for row in &agent_rows {
        for project in &row.project_breakdowns {
            if !project.cwd.is_empty() {
                resolver.add(&project.cwd);
            }
        }
    }
    let resolved = resolver.finish();

    let mut projects = Vec::<ProjectTotal>::new();
    let mut indexes = FxHashMap::<Option<&str>, usize>::default();
    for row in &agent_rows {
        if row.project_breakdowns.is_empty() {
            let share = (row.agent, row.total_cost, row.total_tokens);
            add_share(&mut projects, &mut indexes, None, share);
        }
        for project in &row.project_breakdowns {
            let resolved = resolved.get(project.cwd.as_ref());
            let share = (row.agent, project.cost, project.total_tokens);
            add_share(&mut projects, &mut indexes, resolved, share);
        }
    }

    projects.retain(|project| project.tokens > 0 || project.cost > 0.0);
    for project in &mut projects {
        project.agents.sort_by(|a, b| {
            b.cost
                .total_cmp(&a.cost)
                .then_with(|| b.tokens.cmp(&a.tokens))
        });
    }
    projects.sort_by(|a, b| {
        a.path
            .is_none()
            .cmp(&b.path.is_none())
            .then_with(|| b.cost.total_cmp(&a.cost))
            .then_with(|| b.tokens.cmp(&a.tokens))
    });

    let total_cost = projects.iter().map(|project| project.cost).sum::<f64>();
    let total_tokens = projects.iter().map(|project| project.tokens).sum::<u64>();
    let share = |project: &ProjectTotal| {
        if total_cost > 0.0 {
            percentage(project.cost, total_cost)
        } else {
            percentage(project.tokens as f64, total_tokens as f64)
        }
    };
    json!(
        projects
            .iter()
            .map(|project| json!({
                "name": project.name,
                "path": project.path,
                "totalCost": json_float(project.cost),
                "totalTokens": project.tokens,
                "percentage": json_float(share(project)),
                "agents": project
                    .agents
                    .iter()
                    .map(|share| json!({
                        "agent": share.agent,
                        "totalCost": json_float(share.cost),
                        "totalTokens": share.tokens,
                    }))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>()
    )
}

fn add_share<'a>(
    projects: &mut Vec<ProjectTotal>,
    indexes: &mut FxHashMap<Option<&'a str>, usize>,
    project: Option<&'a Project>,
    (agent, cost, tokens): (&'static str, f64, u64),
) {
    let key = project.map(|project| project.path.as_str());
    let index = *indexes.entry(key).or_insert_with(|| {
        projects.push(ProjectTotal {
            name: project
                .map_or(OTHER_PROJECT_NAME, |project| &project.name)
                .to_string(),
            path: key.map(str::to_string),
            cost: 0.0,
            tokens: 0,
            agents: Vec::new(),
        });
        projects.len() - 1
    });
    let total = &mut projects[index];
    total.cost += cost;
    total.tokens += tokens;
    match total.agents.iter_mut().find(|share| share.agent == agent) {
        Some(share) => {
            share.cost += cost;
            share.tokens += tokens;
        }
        None => total.agents.push(AgentShare {
            agent,
            cost,
            tokens,
        }),
    }
}

#[cfg(test)]
mod tests {
    use agent_burn_test_support::Fixture;

    use super::*;
    use crate::ProjectBreakdown;

    fn project(cwd: &str, total_tokens: u64, cost: f64) -> ProjectBreakdown {
        ProjectBreakdown {
            cwd: cwd.into(),
            total_tokens,
            cost,
        }
    }

    fn agent_row(
        agent: &'static str,
        total_tokens: u64,
        total_cost: f64,
        projects: Vec<ProjectBreakdown>,
    ) -> AllRow {
        AllRow {
            agent,
            total_tokens,
            total_cost,
            project_breakdowns: projects,
            ..AllRow::default()
        }
    }

    fn day(agents: Vec<AllRow>) -> AllRow {
        AllRow {
            period: "2026-01-02".to_string(),
            agent: "all",
            total_tokens: agents.iter().map(|row| row.total_tokens).sum(),
            total_cost: agents.iter().map(|row| row.total_cost).sum(),
            agent_breakdowns: Some(agents),
            ..AllRow::default()
        }
    }

    fn projects_json_for(fixture: &Fixture, rows: &[AllRow]) -> Value {
        projects_json_with(rows, ProjectResolver::new(Some(fixture.path("home"))))
    }

    #[test]
    fn sorts_projects_by_cost_and_keeps_other_last() {
        let fixture = Fixture::new();
        let app = fixture.path("home/dev/app");
        let web = fixture.path("home/dev/web");
        let rows = vec![day(vec![
            agent_row(
                "claude",
                60,
                3.0,
                vec![
                    project(&app.to_string_lossy(), 10, 1.0),
                    project("", 40, 1.5),
                    project(&web.to_string_lossy(), 10, 0.5),
                ],
            ),
            agent_row("qwen", 5, 0.25, Vec::new()),
        ])];

        let projects = projects_json_for(&fixture, &rows);

        let names = projects
            .as_array()
            .unwrap()
            .iter()
            .map(|project| {
                (
                    project["name"].as_str().unwrap(),
                    project["totalCost"].as_f64().unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec![("app", 1.0), ("web", 0.5), ("Other", 1.75),]);
        assert_eq!(projects[2]["path"], Value::Null);
        assert_eq!(projects[0]["path"], json!(app.to_string_lossy()));
    }

    #[test]
    fn groups_worktree_usage_under_its_repository_and_splits_it_by_agent() {
        let fixture = Fixture::new();
        let _ = fixture.create_dir_all("home/dev/app/.git");
        let repo = fixture.path("home/dev/app");
        let worktree = repo.join(".claude/worktrees/feature");
        let rows = vec![
            day(vec![
                agent_row(
                    "claude",
                    30,
                    3.0,
                    vec![
                        project(&repo.to_string_lossy(), 10, 1.0),
                        project(&worktree.to_string_lossy(), 20, 2.0),
                    ],
                ),
                agent_row(
                    "codex",
                    5,
                    0.5,
                    vec![project(&repo.join("src").to_string_lossy(), 5, 0.5)],
                ),
            ]),
            day(vec![agent_row(
                "codex",
                7,
                0.25,
                vec![project(&repo.to_string_lossy(), 7, 0.25)],
            )]),
        ];

        let projects = projects_json_for(&fixture, &rows);

        assert_eq!(
            projects,
            json!([{
                "name": "app",
                "path": repo.to_string_lossy(),
                "totalCost": 3.75,
                "totalTokens": 42,
                "percentage": 100,
                "agents": [
                    { "agent": "claude", "totalCost": 3, "totalTokens": 30 },
                    { "agent": "codex", "totalCost": 0.75, "totalTokens": 12 },
                ],
            }])
        );
    }

    #[test]
    fn project_costs_add_up_to_the_summary_total() {
        let fixture = Fixture::new();
        let rows = vec![day(vec![
            agent_row(
                "claude",
                30,
                0.75,
                vec![project("/x/app", 10, 0.25), project("", 20, 0.5)],
            ),
            agent_row("kimi", 8, 0.125, vec![project("/x/web", 8, 0.125)]),
        ])];

        let projects = projects_json_for(&fixture, &rows);

        let projects = projects.as_array().unwrap();
        let cost = projects
            .iter()
            .map(|project| project["totalCost"].as_f64().unwrap())
            .sum::<f64>();
        let percentage = projects
            .iter()
            .map(|project| project["percentage"].as_f64().unwrap())
            .sum::<f64>();
        assert!((cost - 0.875).abs() < 1e-12);
        assert!((percentage - 100.0).abs() < 1e-9);
    }

    #[test]
    fn omits_other_when_all_usage_is_attributed() {
        let fixture = Fixture::new();
        let rows = vec![day(vec![agent_row(
            "claude",
            10,
            1.0,
            vec![project("/x/app", 10, 1.0)],
        )])];

        let projects = projects_json_for(&fixture, &rows);

        assert_eq!(projects.as_array().unwrap().len(), 1);
        assert_eq!(projects[0]["name"], "app");
    }

    #[test]
    fn falls_back_to_token_share_without_cost() {
        let fixture = Fixture::new();
        let rows = vec![day(vec![agent_row(
            "claude",
            40,
            0.0,
            vec![project("/x/app", 30, 0.0), project("/x/web", 10, 0.0)],
        )])];

        let projects = projects_json_for(&fixture, &rows);

        assert_eq!(projects[0]["percentage"], 75);
        assert_eq!(projects[1]["percentage"], 25);
    }
}
