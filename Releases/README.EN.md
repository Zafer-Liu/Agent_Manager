# Agent Manager v0.3.0

> **Release Date:** July 12, 2026
> **Type:** Major Feature Update — External Collaboration

---

## 📦 Downloads

| Platform | File | Notes |
|----------|------|-------|
| Windows (x64) | `智管-Agent Manager_0.3.0_x64-setup.exe` | NSIS installer |
| macOS | *(coming soon)* | Intel / Apple Silicon |

> **Windows security note:** If Windows shows an "Unknown Publisher" warning, select **More info → Run anyway**. The installer is not code-signed yet.

---

## ✨ What's New in v0.3.0

### Phase 4: External Collaboration (P0-P2 Complete)

Upgrades Agent Manager from a "manual operation" local tool to an "external systems push tasks, multi-Agent collaborative execution, results auto-callback" automation platform.

#### P0 — Communication Layer

- **Local Hook Server** (`127.0.0.1:9420`)
  - External systems push tasks via HTTP POST `/hook`, matched by `template_key`
  - HTTP API: `GET /runs/:id` status query, `POST /runs/:id/approve` accept, `POST /runs/:id/reject` reject, `POST /agent/submit` sub-Agent result submission
  - Port and auth_token configurable, persisted to `hook_server.json`
  - **Callback outbound notification**: Calls external URL on Run terminal state, exponential backoff retry (3 attempts)

- **agent_task Node**
  - Workflows can dispatch to local/remote sub-Agents (not limited to MCP Server)
  - oneshot channel suspends waiting for sub-Agent result (120s timeout)
  - `RunTrigger` enum: Manual / Hook / Rework / Schedule

#### P1 — Frontend Visualization + Parallel Execution

- **ExternalTriggers Settings Page**: Hook Server status, port/token config, curl example, restart
- **Run History Source Column**: Distinguish Manual / Hook / Rework / Schedule triggers with colored badges
- **Fan-out Parallel Execution**: static / by_field / llm_split split strategies, `join_all` parallel execution
- **Fan-out Sub-step Display**: Sub-task results as independent StepInstances, collapsible grouped rendering
- **Cloudflare Tunnel Entry**: ProxyManager dynamically passes Hook port into tunnel configuration

#### P2 — Advanced Scheduling + Transport Extension

- **DispatchStrategy**: Fixed / Failover / CapabilityMatch / Random, with candidate Agent list and capability matching
- **Scheduled Triggers (cron)**: Built-in 5-field cron parser (no external crate), background thread checks every minute
- **McpTransport Http Variant**: Remote MCP Server support via Streamable HTTP transport
- **SVG DAG Canvas**: Upgraded from linear list to SVG-drawn DAG canvas with drag positioning, toolbar, and config panel

### Workflow Engine Core Capabilities

- **Acceptance/Rework Closed Loop**: Acceptance node supports approve/reject, reject routes back to the problematic node, in-chat acceptance card
- **Sweeper Auto-Healing**: Background 30s interval detects Step timeout and marks as failed
- **FailureTrace Diagnostics**: MCP stderr ring buffer + failure kind classification (timeout/agent_exit/stderr/tool_error etc.) + retry history
- **Metrics Event Source**: Append-only `workflow_events.jsonl` + frontend four-card local aggregation (total runs / success rate / avg duration / active Agents)

---

## 🔄 Upgrading from v0.2.3

Install v0.3.0 over the previous version. Agent, LLM, proxy, MCP, GitHub Token, and workflow configuration data is preserved.

After upgrading, to use the Hook Server, go to **Settings → External Triggers** to configure the port and auth_token.

---

## 📜 Previous Releases

- **v0.2.3:** Visual workflows, MCP Agent nodes, Chinese/English i18n, CI pipeline
- **v0.2.2:** Workflow engine, MCP Agent nodes, internationalization, persistent MCP selections
- **v0.2.1:** GitHub Agent installation, recommended projects, proxy detection, Tauri permission fixes
- **v0.2.0:** Manager Agent, Dashboard, PTY terminal, Agent publishing, project auto-detection
