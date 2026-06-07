# Agent Manager v0.2.0

> **Release Date:** June 2026  
> **Type:** Major Feature Release

---

## 📦 Downloads

| Platform | File | Notes |
|----------|------|-------|
| Windows (x64) | `agent-manager_0.2.0.exe` | NSIS installer — double-click to install |
| macOS (Intel) | *(coming soon)* | `.dmg` format |

> **Windows security note:** If you see an "Unknown Publisher" warning, click **More info → Run anyway**. The installer is unsigned; this is expected.

---

## ✨ What's New in v0.2.0

### 🤖 Manager Agent — Natural-Language Commander

Control all your Agents with plain text. No clicking required:

```
Start the Business Analytics Agent and open its interface.
Stop Mindmap and tell me what it does.
Which Agents are currently running?
```

Supported actions: start/stop, open UI, open terminal, read Agent README, navigate to any page. Chat history persists when you switch pages and come back.

> Requires an LLM provider — DeepSeek, OpenAI, or any OpenAI-compatible API.

---

### 🏫 Dashboard — Classroom View

All Agents visualized in a classroom layout: Manager Agent at the podium, others in student seats. See every Agent's status at a glance. Hover to start/stop, view details, or open the UI — without leaving the page.

---

### 🌐 Agent Publishing — One-Click Public Access

No fixed IP, domain name, or server needed:

- **Cloudflare Tunnel (temporary):** generates `https://xxx.trycloudflare.com` in seconds. Close the link after your meeting and it expires immediately — ideal for demos.
- **Caddy Reverse Proxy (permanent):** bind a custom domain with automatic HTTPS, add username/password access control with bcrypt storage, manage per-route permissions for multiple users.

---

### 💻 PTY Interactive Terminal

A real embedded PTY terminal with full support for TUI tools like Claude Code. No more switching to an external terminal window.

---

### 🔍 Enhanced Python Auto-Detection

Newly supported: **FastAPI, Django, Streamlit, uv / pyproject.toml**. Select a project folder and Agent Manager fills in the correct startup command and port automatically.

---

## 🐛 Bug Fixes

| Issue | Details |
|-------|---------|
| Full-screen freeze after starting Agent from Dashboard | Fixed UI freeze caused by state update ordering |
| Mindmap (Node.js) entering REPL on startup | Fixed PTY interaction-mode detection for npm global packages |
| Dev-port conflict with other Vite projects | Changed dev port from 5173 to 1420 |

---

## 🔄 Upgrading from v0.1.0

Install the new package directly over the old one. Agent configuration data in `%APPDATA%\agent-manager\` is preserved.

---

## 📋 Known Limitations

- macOS build is not included in this release; it is in progress.
- When multiple users access the same Agent simultaneously, conversation history is managed by the Agent itself — Agent Manager cannot isolate sessions externally.

---

## 🤝 Feedback & Contributing

- **Bug reports / feature requests:** [GitHub Issues](https://github.com/Zafer-Liu/Agent_MCP_Manager/issues)
- **Code contributions:** PRs welcome

---

*[View full documentation →](https://github.com/Zafer-Liu/Agent_MCP_Manager#readme)*
