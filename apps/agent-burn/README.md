# Agent Burn

Native macOS app and local CLI for coding-agent usage, limits, and subscription value.

Site: [agent-burn.melvynx.dev](https://agent-burn.melvynx.dev)

`summary` is the all-up local spend view. `harness <claude|codex|kimi>` is the subscription-limit view. The npm package also installs `burn` as a short alias.

## Install

**Mac app:** [Download](https://agent-burn.melvynx.dev/download), unzip, move to Applications. macOS 14+, Apple Silicon and Intel. The app bundles the CLI.

**CLI:**

```bash
npx agent-burn@latest summary --value
pnpm dlx agent-burn@latest harness claude --value
bunx agent-burn@latest harness codex --value
```

## Commands

```bash
agent-burn
agent-burn summary
agent-burn summary today
agent-burn summary week --value
agent-burn harness claude --value
agent-burn harness codex --value
agent-burn harness kimi
agent-burn summary --json
agent-burn summary --json --jq '.projects[:5]'
```

`summary --json` includes `projects`: cost per git repository for Claude Code, Codex, and Kimi Code sessions, with worktrees merged into their repository.

macOS app: menu-bar quota for Codex, Claude, Kimi Code, and Cursor, plus a dashboard across detected harnesses. Build locally with `just macos::run`. Details: [apps/macos](https://github.com/Melvynx/agent-burn/tree/main/apps/macos).

## Subscription Value

`--value` compares local API-equivalent usage with known or supplied monthly plan prices.

```bash
agent-burn summary --value
agent-burn summary --value --claude-plan max-20x --codex-plan pro
agent-burn harness claude --value --claude-plan 200
agent-burn harness codex --value --codex-plan plus
```

Plan overrides:

- Claude: `pro`, `max-5x`, `max-20x`, or a monthly price
- Codex: `plus`, `pro`, or a monthly price
- Cursor: `pro`, `pro+`, `ultra`, or a monthly price

Kimi Code has no plan flag. Its costs are API-equivalent estimates: LiteLLM has no `kimi-for-coding` price, so coding-plan models are priced as Kimi K2.7 Code or Kimi K3 (Kimi K2.5/K2.6 for legacy kimi-cli logs).

## Shared Options

```bash
--since <YYYYMMDD>       Start date
--until <YYYYMMDD>       End date
--json                   JSON output
--jq <filter>            Apply a jq filter to JSON output
--mode <auto|calculate|display>
--breakdown              Include model breakdowns
--offline                Use embedded pricing and skip live requests
--no-cost                Hide cost fields
--timezone <tz>          Date grouping timezone
--compact                Force compact table layout
--config <path>          Load a config file
```

## Data Sources

Reads local logs. Nothing is uploaded.

| Source | Default location |
| --- | --- |
| Claude Code | `~/.claude`, `~/.config/claude/projects` |
| Codex | `${CODEX_HOME:-~/.codex}` |
| Cursor | Cursor `state.vscdb` plus the signed-in dashboard usage API |
| Kimi Code | `${KIMI_CODE_HOME:-~/.kimi-code}` and legacy kimi-cli `~/.kimi` (`KIMI_DATA_DIR` replaces both) |

Live Kimi Code quotas use the access token the Kimi CLI stores. Agent Burn never refreshes it, so quotas pause after it expires until the Kimi CLI refreshes it.

## Acknowledgments

Agent Burn started from [ccusage](https://github.com/ccusage/ccusage) by [ryoppippi](https://github.com/ryoppippi). The original local log readers, cost aggregation, and CLI report patterns are the prior work this project builds on.

## License

MIT. Copyright (c) 2025 ryoppippi and 2026 Melvynx.
