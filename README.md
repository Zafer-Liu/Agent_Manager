# Agent MCP Manager

<div align="center">

![Agent MCP Manager](frontend/src/assets/hero.png)

[![License](https://img.shields.io/github/license/Zafer-Liu/Agent_MCP_Manager?style=flat-square)](LICENSE)
[![Release](https://img.shields.io/github/v/release/Zafer-Liu/Agent_MCP_Manager?style=flat-square)](https://github.com/Zafer-Liu/Agent_MCP_Manager/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square)](#)
[![Build](https://img.shields.io/github/actions/workflow/status/Zafer-Liu/Agent_MCP_Manager/release.yml?style=flat-square)](https://github.com/Zafer-Liu/Agent_MCP_Manager/actions)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square&logo=tauri)](https://tauri.app)
[![React](https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react)](https://react.dev)
[![Rust](https://img.shields.io/badge/Rust-stable-CE422B?style=flat-square&logo=rust)](https://www.rust-lang.org)

**A cross-platform desktop application for managing AI Agents and MCP (Model Context Protocol) Servers**

Built with Tauri 2 · React 19 · Rust

[English](README.md) · [中文](README.zh.md)

</div>

---

## ✨ Features

- **Agent Manager** — Add, configure, start/stop, and monitor AI agents; real-time log streaming (stdout + stderr); drag-and-drop reorder
- **MCP Server Manager** — Visually manage Claude Desktop's `claude_desktop_config.json`; add/edit/remove MCP servers without touching JSON
- **Port Manager** — Scan and display all listening ports on your machine; kill processes by port with one click
- **Smart Project Detection** — Point to a project folder and auto-detect Python / Node.js / Rust / Go entry points, package manager, and default port
- **In-App UI Preview** — Open agent web UIs directly inside the app as browser-style tabs (via iframe)
- **Resizable Sidebar** — Drag the divider to adjust layout
- **Dark Mode** — Full dark mode support

## 🖥️ Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop Shell | [Tauri 2](https://tauri.app) |
| Frontend | React 19 + TypeScript + Tailwind CSS v4 |
| Backend | Rust (process management, file I/O) |
| State | Zustand |
| Drag & Drop | @dnd-kit |
| Icons | Lucide React |
| Build Tool | Vite 8 |

## 📦 Installation

### Pre-built Binaries

Download the latest installer from [Releases](https://github.com/Zafer-Liu/Agent_MCP_Manager/releases):

| Platform | File |
|----------|------|
| Windows | `Agent_MCP_Manager_x.x.x_x64-setup.exe` |
| macOS (Intel) | `Agent_MCP_Manager_x.x.x_x64.dmg` |
| macOS (Apple Silicon) | `Agent_MCP_Manager_x.x.x_aarch64.dmg` |
| Linux | `agent-mcp-manager_x.x.x_amd64.AppImage` |

### Build from Source

**Prerequisites:**
- [Node.js](https://nodejs.org) ≥ 20
- [Rust](https://rustup.rs) (stable)
- [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS

```bash
# Clone the repository
git clone https://github.com/Zafer-Liu/Agent_MCP_Manager.git
cd Agent_MCP_Manager

# Install root dependencies
npm install

# Install frontend dependencies
cd frontend && npm install && cd ..

# Run in development mode
npm run dev

# Build for production
npm run build
```

## 🚀 Quick Start

### Adding an Agent

1. Click **New Agent** in the sidebar
2. Fill in the name, command, working directory, and optional environment variables
3. Or click the **📁 folder icon** to auto-detect configuration from a project directory
4. Click **Save**, then hit ▶ to start

### Managing MCP Servers

1. Navigate to **MCP Servers** in the left nav
2. Add servers — they will be written directly to Claude Desktop's config file
3. Restart Claude Desktop for changes to take effect

> **Config file path:**
> - Windows: `%AppData%\Claude\claude_desktop_config.json`
> - macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`

### Port Manager

1. Navigate to **Port Manager**
2. View all active listening ports
3. Click **Kill** to terminate a process occupying a port

## 📁 Project Structure

```
Agent_MCP_Manager/
├── frontend/                 # React + TypeScript frontend
│   └── src/
│       ├── components/       # AgentList, AgentDetail, AgentForm, LogViewer
│       ├── pages/            # McpManager, PortManager
│       ├── store/            # Zustand state (agentStore)
│       └── types/            # TypeScript types
├── src-tauri/                # Rust backend
│   └── src/
│       ├── agent.rs          # Agent state & process store
│       ├── commands.rs       # Tauri commands (start/stop/logs/scan)
│       ├── mcp.rs            # MCP config read/write
│       ├── ports.rs          # Port scanning & kill
│       └── lib.rs            # Tauri app entry
└── .github/workflows/        # CI/CD (multi-platform release)
```

## 🔧 Configuration

Agent configurations are persisted in the OS data directory:

| Platform | Path |
|----------|------|
| Windows | `%AppData%\agent-manager\agents.json` |
| macOS | `~/Library/Application Support/agent-manager/agents.json` |
| Linux | `~/.local/share/agent-manager/agents.json` |

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/your-feature`
3. Commit your changes: `git commit -m "feat: add your feature"`
4. Push and open a Pull Request

## 📄 License

[MIT](LICENSE)
