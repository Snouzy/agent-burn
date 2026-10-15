# Kimi Source

agent-burn reads two Kimi CLIs under the single `kimi` agent:

- the legacy Python **kimi-cli** (`~/.kimi`)
- the current TypeScript **Kimi Code** (`~/.kimi-code`)

Both write `wire.jsonl` files under a `sessions/` directory; the layout is
told apart by depth.

## Data directories

| Variable | Effect |
| --- | --- |
| `KIMI_DATA_DIR` | Comma-separated data roots that replace both defaults. Each root may hold either layout (this is what tests use). |
| `KIMI_CODE_HOME` | Kimi Code's own home override, read when `KIMI_DATA_DIR` is unset. Defaults to `~/.kimi-code`. |

Without `KIMI_DATA_DIR`, agent-burn scans `~/.kimi` and the Kimi Code home.

## Legacy kimi-cli (`~/.kimi`)

```text
sessions/{group}/{session}/wire.jsonl
```

Only `StatusUpdate` messages with `payload.token_usage` count:

| Wire field | agent-burn field |
| --- | --- |
| `input_other` | input |
| `output` | output |
| `input_cache_read` | cache read |
| `input_cache_creation` | cache creation |
| `total` | fallback when the parts are missing |

The model comes from `config.json` `model` (default `kimi-for-coding`).
Sub-agent `subagents/{id}/wire.jsonl` files are not read; kimi-cli reports
sub-agent events inside the parent wire as `SubagentEvent`.

## Kimi Code (`~/.kimi-code`)

```text
sessions/{workDirKey}/{session}/agents/{agentId}/wire.jsonl
```

`agents/main` is the main agent. Every other `agents/{agentId}` is a sub-agent,
and its usage rolls up into the parent `{session}`.

Usage lines (upstream `packages/agent-core-v2/src/agent/usage/usageOps.ts`):

```json
{"type":"usage.record","agentId":"main","model":"kimi-code/kimi-for-coding","usage":{"inputOther":100,"output":50,"inputCacheRead":1000,"inputCacheCreation":0},"usageScope":"turn","time":1779256800000}
```

- Kimi Code writes one `usage.record` per LLM request. Both `turn` and
  `session` scopes are deltas, so every record is summed.
- `time` is epoch milliseconds. When it is missing, the file's modification
  time is used.
- `step.end` usage inside `context.append_loop_event` repeats the same
  request and is ignored.
- Records with zero tokens are skipped.

### Forks

Forks and child sessions
(`sessionLifecycleService.ts` `fork` / `copyAgentWire` / `appendForkedMarker`)
copy the source agent's wire records into the new session, then append a
`{"type":"forked","agentId":…}` marker. In each file, agent-burn counts only
the `usage.record` lines after the last `forked` marker. A fork of a fork
therefore counts only its own usage.

### Migrated sessions

Kimi Code imports legacy sessions into `sessions/{bucket}/ses_{uuid}/` and
tags them in `state.json` with `custom.imported_from_kimi_cli: true`. The
upstream translator (`packages/migration-legacy/src/sessions/translator.ts`,
`wire-writer.ts`) drops kimi-cli's `_usage` rows and writes no `usage.record`.
It only writes a `token_counting.measured` context-size anchor. The original
`~/.kimi` tree is left in place.

As a result:

- the legacy tree is the only source of the usage it recorded
- the imported wire adds only the usage from after the import

No cross-tree dedupe is needed, and agent-burn does not read `state.json`.

### Workspaces

`workspaces.json` in the data root maps each `{workDirKey}` to the folder the
sessions ran in:

```json
{"version":1,"workspaces":{"wd_agent-burn_148663d55be9":{"root":"/Users/me/dev/agent-burn","name":"agent-burn"}}}
```

Kimi Code entries use that `root` as their project path, which feeds the
`projects` breakdown in `summary --json`. An unknown key, a missing file, and
the legacy tree fall back to the `Kimi` label and stay unattributed.

### Model aliases

`model` is a Kimi Code alias. agent-burn resolves it through `config.toml`
`[models."<alias>"].model`. When no entry exists, it strips the managed
`kimi-code/` prefix. A minimal line reader handles the `config.toml` keys
agent-burn needs, so no TOML dependency is added.

## Pricing

Kimi's coding-plan model ids are not API model names, so LiteLLM has no
`kimi-for-coding` key. Costs are API-equivalent estimates:

| Model | Priced as |
| --- | --- |
| `kimi-for-coding` (legacy tree) | `moonshot/kimi-k2.5` before 2026-04-20T15:28:10.072Z, then `moonshot/kimi-k2.6` |
| `kimi-for-coding` (Kimi Code tree) | `moonshot/kimi-k2.7-code` |
| `kimi-for-coding-highspeed` (either tree) | `moonshot/kimi-k2.7-code-highspeed` |
| `k3`, `k3-256k` | `moonshot/kimi-k3` |
| anything else | `moonshot/<model>`, `kimi/<model>`, `<model>` |

`moonshot/kimi-k2.7-code`, `moonshot/kimi-k2.7-code-highspeed`, and
`moonshot/kimi-k3` are built-in overrides in `pricing.rs`, because the embedded
LiteLLM snapshot only keeps Anthropic/OpenAI/Z.ai keys. Moonshot publishes no
highspeed list price, so the highspeed override uses LiteLLM's
`tencent/kimi-k2.7-code-highspeed` rates (2× K2.7 Code). Cache writes are priced
at 1.25× input, the same convention used for K2.5 and K2.6.

## Live quota

`agent-burn harness kimi` and `summary --value --json` (`kimiAccount`) call
`GET {baseUrl}/usages` with `Authorization: Bearer <access_token>`. This
mirrors upstream `packages/oauth/src/managed-usage.ts`.

How the base URL and token are found (`packages/oauth/src/managed-kimi-code.ts`,
`region.ts`, `storage.ts`):

- The base URL is the first of:
  1. `KIMI_CODE_BASE_URL`
  2. `[providers."managed:kimi-code"].base_url` in `config.toml`
  3. `https://api.kimi.com/coding/v1` (mainland China)

  The global region writes `https://api.kimi.ai/coding/v1` to `config.toml`.
- The credential slot comes from `[providers."managed:kimi-code".oauth].key`,
  either `oauth/kimi-code` or a region-scoped `oauth/kimi-code-env-<hash>`. Its
  token is stored in `$KIMI_CODE_HOME/credentials/<key without oauth/>.json`.
  Without a key, the slot is `credentials/kimi-code.json`.
- agent-burn reads only `access_token` (snake_case).
- If `expires_at` (unix seconds) has passed, there is no network call.
  `expires_at == 0` means "no known expiry", as in upstream.
- **agent-burn never calls the OAuth refresh endpoint.** Refreshing rotates
  the refresh token and could sign the user's CLI out. An expired token simply
  means the quota is unavailable until Kimi Code refreshes it.
- Requests time out after 5 s, with a 1 MB body cap. Network, HTTP, and parse
  errors yield no quota and are never fatal. `--offline` skips the request.

Response windows (`usages.*`), each optional:

- `used_ratio` is 0–1 and may be a string.
- `reset_time` is RFC 3339 and may carry nanoseconds.

| Key | Meaning | `kimiAccount` keys |
| --- | --- | --- |
| `limit_5h` | 5-hour session | `sessionUsedPercent`, `sessionResetsAtMs` |
| `limit_7d` | weekly (legacy plans only) | `weeklyUsedPercent`, `weeklyResetsAtMs` |
| `limit_month_code` | monthly coding quota | `monthlyUsedPercent`, `monthlyResetsAtMs` |
| `limit_month_total` | monthly total quota | `monthlyTotalUsedPercent`, `monthlyTotalResetsAtMs` |

Percents are 0–100 and reset times are epoch milliseconds. Unknown keys are
omitted, not null. The older payload parsed by kimi-cli `ui/shell/usage.py` is
also accepted:

- a `usage` object with `limit` and `used`/`remaining` maps to the weekly window
- a `limits[]` row whose window is 300 minutes maps to the 5-hour window

The booster wallet (`boosterWallet.balance` with `type: "BOOSTER"`) uses
fixed-point `amount` / `amountLeft` values, at 1,000,000 units per cent. A
positive sub-cent amount rounds up to one cent, as in upstream. These map to:

- `boosterTotal` / `boosterRemaining`, in major units (`1250000000` → `12.5`)
- `boosterCurrency`, from `monthlyChargeLimit.currency` or
  `monthlyUsed.currency`, omitted when neither is present

`AGENT_BURN_QUOTA_ONLY=1 agent-burn harness kimi --json` returns the shared
quota snapshot:

- `window` is `limit_7d` (10080 minutes) when present. Otherwise it is
  `limit_month_code`, falling back to `limit_month_total`. A monthly window
  spans the calendar month (UTC) before `reset_time`.
- `sessionWindow` is `limit_5h` (300 minutes).

The full harness report has no plan name, price, or `--kimi-plan` flag. Its
terminal section reads "this month" for a monthly window and "this week" for a
weekly one.
