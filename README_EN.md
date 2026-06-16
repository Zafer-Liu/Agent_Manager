# Agent Manager

<p align="center">
  <img src="./Logo/Banner.png" alt="Agent Manager Banner" width="100%" />
</p>

<p align="right"><a href="./README.md">中文</a></p>

![Version](https://img.shields.io/badge/Version-v0.2.3-blue.svg)
![License](https://img.shields.io/badge/License-Apache%202.0-yellow.svg)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS-blue?style=flat-square)](#)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?style=flat-square&logo=tauri)](https://tauri.app)
[![React](https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react)](https://react.dev)
[![Rust](https://img.shields.io/badge/Rust-stable-CE422B?style=flat-square&logo=rust)](https://www.rust-lang.org)

> A unified local management center for AI Agents.  
> After adding your Agents, you can:
>
> - Start and stop Agents with one click, while viewing real-time logs
> - Use embedded Web UIs and interactive terminals
> - Control all Agents through natural language
> - Generate temporary public sharing links with one click

<p align="center">
  <a href="#features">✨ Highlights</a> ·
  <a href="#install">⚙️ Installation</a> ·
  <a href="#quickstart">🚀 Quick Start</a> ·
  <a href="#llm-config">🤖 LLM Config</a> ·
  <a href="#share">🌐 Share Agents</a> ·
  <a href="#faq">❓ FAQ</a>
</p>

<details>
<summary><strong>📚 Full Table of Contents</strong></summary>

<br>

- [Highlights](#features)
- [Core Capabilities](#capabilities)
  - [Dashboard Classroom View](#dashboard)
  - [Manager Agent Natural-Language Commander](#manager)
  - [Agent Management](#agents)
  - [MCP Agent Tool Conversation](#mcp)
  - [Agent Publishing and Temporary Sharing](#share)
  - [Port Manager](#ports)
- [Installation](#install)
- [Quick Start](#quickstart)
- [LLM Configuration](#llm-config)
- [Supported Project Types](#project-types)
- [Data Storage Paths](#data-paths)
- [Roadmap and Changelog](#roadmap)
- [FAQ](#faq)
- [Contributing](#contributing)
- [License](#license)

</details>

---

<a id="features"></a>

# ✨ Highlights

**Agent Manager** is a desktop application built with Tauri 2 and React 19. It is designed to solve a common problem: once you run multiple AI Agents locally, managing them quickly becomes messy.

Core idea: **manage all Agents from one window.**

- No need to keep multiple terminals open
- No need to remember different startup commands
- No need to manually open browser tabs and search for ports
- Control every Agent through natural language

---

<a id="capabilities"></a>

# 🧠 Core Capabilities

<a id="dashboard"></a>

## 1️⃣ Dashboard — Classroom View

All Agents are displayed visually: the **Manager Agent stands at the podium**, while other Agents sit in student seats.

![classroom](Images/classroom.png)

You can see the status of every Agent at a glance. Hover over an Agent seat to:

- Start or stop the Agent
- View details
- Open its UI

Click the Manager Agent at the podium to jump directly to the natural-language command interface.

---

<a id="manager"></a>

## 2️⃣ Manager Agent — Natural-Language Commander

Control all Agents with a single sentence. No manual clicking required.

![manager](Images/manager.png)

```text
Start the Business Analytics Agent and open its interface.
```

```text
Stop Mindmap and tell me what it does.
```

```text
Which Agents are currently running?
```

The Manager Agent understands your intent, calls the required tools automatically, completes the operation, and reports the result back to you.

**Supported operations:**

| Natural-language request | Actual behavior |
|--------------------------|-----------------|
| Start / stop an Agent | Calls the start / stop API |
| Open an Agent interface | Opens the Web UI in a tab automatically |
| Open an Agent terminal | Opens an embedded PTY terminal in a tab automatically |
| Check all Agent statuses | Returns a real-time status summary table |
| Learn what an Agent does | Reads the README inside that Agent directory |
| Navigate to a page | Navigates automatically |

**Persistent conversations:** when you switch to another page and come back, your Manager Agent chat history is preserved.

> An LLM provider must be configured before using this feature. See [LLM Configuration](#llm-config).

---

<a id="agents"></a>

## 3️⃣ Agent Management

### Automatic Project Type Detection

After you select a project directory, Agent Manager detects the project type and fills in the startup command automatically. See [Supported Project Types](#project-types) for details.

### Start and Monitor

![Agent1](Images/Agent1.png)

- Start / stop Agent processes with one click
- Stream real-time logs from stdout and stderr, with auto-scroll support
- View PID, port status, and startup time
- Drag and reorder Agents in the sidebar

### Embedded Web UI

![Agent2](Images/Agent2.png)

Agents with Web UIs, such as Streamlit, Flask, and FastAPI projects, can be opened directly inside the app. You no longer need to switch back and forth between browsers.

- Open multiple Agent UIs in tabs
- Resize the tab bar height by dragging
- Enter full-screen mode with one click
- Automatically fill WebSocket tokens for OpenClaw-type Agents

---

<a id="mcp"></a>

## 4️⃣ MCP Agent — AI Tool Conversation

![mcp](Images/mcp.png)

Connect local tool services through MCP, short for Model Context Protocol, and chat with AI through multi-turn tool calls.

**Ways to add MCP servers:**

- **Local scan:** automatically detects globally installed npm MCP packages
- **Smart parsing:** paste any text, such as official docs or installation instructions, and AI extracts the configuration automatically
- **Manual setup:** enter stdio or SSE configuration manually

---

<a id="share"></a>

## 5️⃣ Agent Publishing — Temporary Sharing and Public Access

### 🔗 Temporary Sharing Recommended for Meetings

![agency](Images/Agency.png)

No fixed IP, domain name, or server is required. Generate a temporary public link with one click:

```text
Your local computer localhost:5001
        ↓ Cloudflare Tunnel
https://abc-xyz.trycloudflare.com  ← Share this with teammates
```

**Workflow:**

1. Install cloudflared, as shown below
2. Make sure the target Agent is running
3. Go to the **Agent Publishing** page, find the target Agent, then click **Generate Link**
4. Wait about 5–15 seconds until `https://xxx.trycloudflare.com` appears
5. Copy the link and send it to teammates
6. Click **Close** after the meeting, and the link becomes invalid immediately

**Install cloudflared:**

```powershell
# Windows, recommended with Scoop
scoop install cloudflared

# macOS
brew install cloudflared
```

You can also download the executable directly from [GitHub Releases](https://github.com/cloudflare/cloudflared/releases/latest).

> ⚠️ Temporary links do not include access control. Open them only when needed and close them immediately after use.

### 🛡️ Caddy Reverse Proxy for Long-Term Publishing

This is suitable when you need a fixed domain name and persistent public access.

- Bind a custom domain with automatic HTTPS certificates
- Add username / password access control with bcrypt-encrypted storage
- Manage fine-grained multi-user permissions, with independent access rules for each route
- Generate a Caddyfile and start or reload Caddy with one click

---

<a id="ports"></a>

## 6️⃣ Port Manager

![port](Images/port.png)

View all ports currently listening on your machine.

- Display port number, protocol, PID, and process name
- Kill the process occupying a specific port with one click
- Quickly troubleshoot Agent startup failures caused by port conflicts

---

<a id="install"></a>

# ⚙️ Installation

### Download Prebuilt Installers Recommended

Download the latest version from [Releases](https://github.com/Zafer-Liu/Agent_Manager/releases):

| Platform | File |
|----------|------|
| Windows (x64) | `智管-Agent Manager_0.2.3_x64-setup.exe` |
| macOS | Coming soon |

Double-click the installer and follow the prompts.

### Build from Source

**Prerequisites:**

- [Node.js](https://nodejs.org) 18 or later
- [Rust](https://rustup.rs), stable toolchain
- [Tauri prerequisites](https://tauri.app/start/prerequisites/), including Visual Studio C++ Build Tools on Windows

```bash
git clone https://github.com/Zafer-Liu/Agent_Manager.git
cd Agent_Manager

# Install dependencies
npm install
cd frontend && npm install && cd ..

# Run in development mode
npm run dev

# Build for production
npm run build
```

Build outputs are located in `src-tauri/target/release/bundle/`.

---

<a id="quickstart"></a>

# 🚀 Quick Start

## Step 1: Add an Agent

1. Click **Agents** in the sidebar, then click **+ New Agent** in the top-right corner
2. Click 📁 to select the Agent project directory
3. Agent Manager detects the project type and fills in the startup command automatically
4. Enter a name and confirm the port number, if the Agent has a Web UI, then click **Save**
5. Click ▶ to start the Agent

## Step 2: View the Agent Interface

- For Agents with Web UIs, such as Streamlit or Flask: click **Open UI** to open it inside an embedded tab
- For TUI Agents, such as Claude Code: click **Open Terminal** to open an embedded terminal

## Step 3: Use Manager Agent Optional

1. Add an LLM provider in **MCP Agent → LLM Settings**. See [LLM Configuration](#llm-config)
2. Click **Manager** in the sidebar
3. Select an LLM provider
4. Send instructions in natural language

## Step 4: Share an Agent During Meetings Optional

1. Install cloudflared
2. Open the **Agent Publishing** page
3. Find the target Agent, click **Generate Link**, then copy and share the link

---

<a id="llm-config"></a>

# 🤖 LLM Configuration

Both Manager Agent and MCP Agent require an LLM provider. Add one in **MCP Agent → LLM Settings**:

| Field | Description | Example |
|-------|-------------|---------|
| Name | Custom display name | DeepSeek |
| Base URL | OpenAI-compatible API endpoint | `https://api.deepseek.com/v1` |
| API Key | API key for the provider | `sk-xxx` |
| Model | Model name | `deepseek-chat` |

Click **Test Connection**. A green status means the configuration is valid.

**Built-in provider presets:**

| Provider | Base URL | Recommended model |
|----------|----------|-------------------|
| DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` |
| OpenAI | `https://api.openai.com/v1` | `gpt-4o-mini` |
| Any compatible API | Custom | Custom |

---

<a id="project-types"></a>

# 🔍 Supported Project Types

After you select a project directory, Agent Manager detects and fills in the following configurations automatically:

| Project type | Detection condition | Auto-generated command |
|--------------|---------------------|------------------------|
| Python · uv | `pyproject.toml` + `uv.lock` | `uv run python main.py` |
| Python · FastAPI | requirements contains `fastapi` | `uvicorn main:app --reload --port PORT` |
| Python · Django | `manage.py` exists | `python manage.py runserver 0.0.0.0:8000` |
| Python · Streamlit | requirements contains `streamlit` | `streamlit run app.py --server.port 8501` |
| Python · Flask | requirements contains `flask` | `python app.py` |
| Python · Generic | `main.py`, `app.py`, or similar files | `python main.py` |
| Node.js | `package.json` | `npm run dev` |
| Rust | `Cargo.toml` | `cargo run` |
| Go | `go.mod` | `go run .` |
| npm global command | `%APPDATA%\npm\*.cmd` | Interactive PowerShell + automatic command input |
| Executable file | `.exe`, `.bat`, `.cmd`, `.sh` | Run directly |

Port numbers are also detected automatically by scanning `.env` files, `pyproject.toml` scripts, and `port=` configurations in source code.

---

<a id="data-paths"></a>

# 📁 Data Storage Paths

| Data | Windows | macOS |
|------|---------|-------|
| Agent configuration | `%APPDATA%\agent-manager\agents.json` | `~/Library/Application Support/agent-manager/agents.json` |
| LLM providers | `%APPDATA%\agent-manager\llm_config.json` | Same as Windows |
| Proxy / user configuration | `%APPDATA%\agent-manager\proxy.json` | Same as Windows |
| Generated Caddyfile | `%APPDATA%\agent-manager\Caddyfile` | Same as Windows |
| MCP server configuration | `%APPDATA%\Claude\claude_desktop_config.json` | `~/Library/Application Support/Claude/claude_desktop_config.json` |

---

<a id="roadmap"></a>

# 🗺️ Roadmap and Changelog

> **Current version `v0.2.3`** · June 13, 2026

## Major Updates in v0.2.3

**New features:**

- ✅ **Visual Workflows:** compose MCP tools, LLM steps, and complete MCP Agent nodes with streamed step feedback
- ✅ **Enhanced MCP Agent:** enable multiple MCP servers per conversation and inspect collapsible tool-call steps
- ✅ **Smart MCP Configuration:** scan local packages or parse JSON, README text, commands, and stdio/SSE settings with AI
- ✅ **Chinese and English UI:** full i18n support and an in-app language switcher across the main workflows
- ✅ **Persistent Selections:** remember the selected LLM, enabled MCP servers, and active workflow
- ✅ **Continuous Integration:** run frontend checks and Rust tests for pushes, pull requests, and releases

**Fixes:**

- Fixed silent failures when clicking **Parse with AI** and now surface the actual API error
- Fixed unsafe truncation of Chinese README content at UTF-8 byte boundaries
- Fixed async runtime conflicts and broken context passing in MCP Agent workflow nodes
- Fixed workflow text remaining truncated after expansion and a server-list refresh race after deletion

---

<a id="faq"></a>

# ❓ FAQ

<details>
<summary><b>🤖 Manager Agent</b></summary>

<br>

<details>
<summary><b>Manager Agent does not respond, or says no provider is selected. What should I do?</b></summary>

1. Go to **MCP Agent → LLM Settings** and add an LLM provider
2. Click **Test Connection** and make sure it passes with a green status
3. Return to the Manager page and select that provider from the dropdown at the top

</details>

<details>
<summary><b>Manager Agent says it cannot find an Agent. Why?</b></summary>

The LLM uses the Agent name to identify it. Make sure the name you typed matches the name in the Agent configuration. Fuzzy matching is supported, case-insensitive matching is supported, and underscores can be used instead of spaces.

</details>

<details>
<summary><b>Manager Agent chat history disappeared after switching pages.</b></summary>

This was fixed in v0.2.0. The Manager Agent component now stays mounted in the background and is only hidden through CSS. If the issue still appears, make sure you are using v0.2.0 or later.

</details>

</details>

---

<details>
<summary><b>🤖 Agent Management</b></summary>

<br>

<details>
<summary><b>The Agent status shows "Error" after startup.</b></summary>

Open the Agent detail page and check the logs. Common causes include:

- Port conflict: use Port Manager to find and kill the process occupying the port
- Missing dependencies: run the startup command manually in a terminal to see the exact error
- Incorrect working directory: check whether the working directory in the Agent configuration is correct

</details>

<details>
<summary><b>The Agent has a Web UI, but the page is blank after opening it.</b></summary>

The Agent may still be starting and the port may not be listening yet. Wait a few seconds, then click the refresh button in the UI panel toolbar.

</details>

<details>
<summary><b>The Claude Code terminal is blank after opening.</b></summary>

This is normal. The app automatically starts PowerShell, waits about 800 ms, and then writes the `claude` command to stdin. Wait another 1–2 seconds and the Claude Code TUI should render.

</details>

<details>
<summary><b>The detected command for my Python Agent is incorrect.</b></summary>

Automatic detection is based on file scanning, so edge cases can be misclassified. You can manually edit the command and arguments in the Agent edit screen. Saved changes take effect immediately.

</details>

<details>
<summary><b>Can I reorder the Agent list?</b></summary>

Yes. On the Agents page, hover over an Agent, hold the drag handle on the left, shown as the ⠿ icon, and drag it to the desired position. The order is saved automatically.

</details>

</details>

---

<details>
<summary><b>🌐 Agent Publishing</b></summary>

<br>

<details>
<summary><b>No URL appears after I click "Generate Link".</b></summary>

Possible causes:

1. cloudflared is not installed or is not in PATH. Click the **Rescan** button and make sure the path is detected
2. Network issue. cloudflared needs access to Cloudflare, so check proxy and firewall settings
3. The Agent is not running. Temporary links forward traffic to a local port, so the Agent must be running

</details>

<details>
<summary><b>My teammate opens the link but sees "This site can’t be reached".</b></summary>

Check the following:

1. The Agent is running on your local machine
2. The tunnel is still open and the app still shows a green URL
3. The link is complete and follows the `https://xxx.trycloudflare.com` format

</details>

<details>
<summary><b>Multiple teammates access the Agent at the same time, and conversations are mixed together.</b></summary>

This is a limitation of the Agent itself. Conversation history is stored inside the Agent process memory, and Agent Manager cannot isolate it externally.

If isolation is required, session support must be added to the Agent code. For example, Streamlit naturally supports per-session state through `st.session_state`.

</details>

<details>
<summary><b>Caddy cannot be found, or proxy_apply fails.</b></summary>

Install Caddy:

```powershell
# Windows
scoop install caddy

# macOS
brew install caddy
```

After installation, click the refresh button on the Agent Publishing page.

</details>

</details>

---

<details>
<summary><b>⚙️ Installation and Runtime</b></summary>

<br>

<details>
<summary><b>Windows shows an "Unknown Publisher" warning during installation.</b></summary>

Click **More info**, then click **Run anyway**. This happens because the installer is not signed with a Microsoft code-signing certificate.

</details>

<details>
<summary><b>macOS says the app cannot be opened because the developer cannot be verified.</b></summary>

Run the following command in Terminal:

```bash
xattr -d com.apple.quarantine /Applications/智管-Agent\ Manager.app
```

Alternatively, right-click the app, choose **Open**, and then click **Open** again.

</details>

<details>
<summary><b>`npm run dev` reports that the development port is already in use.</b></summary>

This project uses development port **1420** to avoid conflicts with Mindmap and other Vite projects that commonly use 5173. If port 1420 is occupied:

```powershell
# Find the process occupying the port
netstat -ano | findstr :1420

# Kill the process. Replace <PID> with the actual PID
taskkill /PID <PID> /F
```

</details>

</details>

---

<a id="contributing"></a>

# 🤝 Contributing

PRs and Issues are welcome.

---

<a id="license"></a>

# 📄 License

[Apache 2.0](LICENSE)

---

# ⭐ Project Goal

Let Agent Manager handle every Agent, so you can spend your time on what truly matters.
