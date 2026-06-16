# Agent Manager v0.2.3

> **Release Date:** June 16, 2026
> **Type:** Feature and Stability Update

---

## 📦 Downloads

| Platform | File | Notes |
|----------|------|-------|
| Windows (x64) | `智管-Agent Manager_0.2.3_x64-setup.exe` | NSIS installer |
| macOS | *(coming soon)* | Intel / Apple Silicon |

> **Windows security note:** If Windows shows an “Unknown Publisher” warning, select **More info → Run anyway**. The installer is not code-signed yet.

---

## ✨ What's New in v0.2.3

### 🔀 Visual Workflows

- Compose MCP tools, LLM steps, and complete MCP Agent nodes with drag and drop
- Let an MCP Agent node use every tool exposed by its selected server
- Select and execute workflows directly from the chat page
- Follow streamed steps, expand long output, and inspect failures before the final summary

### 🧩 MCP Agent and Smart Configuration

- Enable multiple MCP servers in one conversation
- Inspect tool calls through collapsible execution steps
- Scan local MCP packages or configure stdio and SSE servers manually
- Parse JSON, commands, README excerpts, or URLs into MCP configuration with an LLM
- Persist the selected LLM, enabled servers, and active workflow

### 🌐 Chinese and English UI

- Added an in-app Chinese / English language switcher
- Main pages, forms, statuses, and errors now use the i18n system

### ✅ Continuous Integration

- Pushes and pull requests run ESLint, the TypeScript/Vite build, and Rust tests
- Releases reuse the same checks before cross-platform packaging

---

## 🐛 Bug Fixes

| Issue | Fix |
|-------|-----|
| **Parse with AI** appeared to do nothing | Errors are always visible and include the actual HTTP/API message |
| Chinese README parsing could fail | Text is now truncated safely on Unicode character boundaries |
| MCP Agent workflow requests could fail | Blocking MCP stdio and asynchronous LLM HTTP work are separated |
| Workflow context was passed incorrectly | Prompt/task semantics and output fallbacks were corrected |
| Expanded workflow output remained truncated | Line clamping is removed after expansion |
| MCP selections disappeared after restart | Relevant selections are now persisted in localStorage |

---

## 🔄 Upgrading from v0.2.2

Install v0.2.3 over the previous version. Agent, LLM, proxy, and MCP configuration data is preserved.

After upgrading, test the configured LLM again and confirm that the MCP servers you need are enabled.

---

## 📜 Previous Releases

- **v0.2.2:** Workflows, MCP Agent nodes, internationalization, and persistent MCP selections
- **v0.2.1:** GitHub Agent installation, recommended projects, proxy detection, and Tauri permission fixes
- **v0.2.0:** Manager Agent, Dashboard, PTY terminal, Agent publishing, and project auto-detection
