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

**跨平台 AI Agent 与 MCP Server 管理桌面应用**

基于 Tauri 2 · React 19 · Rust 构建

[English](README.md) · [中文](README.zh.md)

</div>

---

## ✨ 功能特性

- **Agent 管理器** — 新增、配置、启动/停止 AI Agent；实时流式显示 stdout + stderr 日志；支持拖拽排序
- **MCP Server 管理器** — 可视化管理 Claude Desktop 的 `claude_desktop_config.json`，无需手动编辑 JSON
- **端口管理器** — 扫描并展示本机所有监听端口；一键终止占用端口的进程
- **智能项目检测** — 指定项目目录，自动识别 Python / Node.js / Rust / Go 入口文件、包管理器和默认端口
- **内嵌 UI 预览** — 以浏览器标签页（iframe）形式在应用内直接打开 Agent 的 Web UI
- **可拖拽分隔栏** — 自由调整侧边栏宽度
- **深色模式** — 完整支持 Dark Mode

## 🖥️ 技术栈

| 层级 | 技术 |
|------|------|
| 桌面外壳 | [Tauri 2](https://tauri.app) |
| 前端 | React 19 + TypeScript + Tailwind CSS v4 |
| 后端 | Rust（进程管理、文件 I/O） |
| 状态管理 | Zustand |
| 拖拽排序 | @dnd-kit |
| 图标库 | Lucide React |
| 构建工具 | Vite 8 |

## 📦 安装

### 下载预构建安装包

从 [Releases](https://github.com/Zafer-Liu/Agent_MCP_Manager/releases) 下载最新安装包：

| 平台 | 文件 |
|------|------|
| Windows | `Agent_MCP_Manager_x.x.x_x64-setup.exe` |
| macOS（Intel） | `Agent_MCP_Manager_x.x.x_x64.dmg` |
| macOS（Apple Silicon） | `Agent_MCP_Manager_x.x.x_aarch64.dmg` |
| Linux | `agent-mcp-manager_x.x.x_amd64.AppImage` |

### 从源码构建

**前置依赖：**
- [Node.js](https://nodejs.org) ≥ 20
- [Rust](https://rustup.rs)（stable）
- 对应平台的 [Tauri 前置依赖](https://tauri.app/start/prerequisites/)

```bash
# 克隆仓库
git clone https://github.com/Zafer-Liu/Agent_MCP_Manager.git
cd Agent_MCP_Manager

# 安装根目录依赖
npm install

# 安装前端依赖
cd frontend && npm install && cd ..

# 开发模式运行
npm run dev

# 生产构建
npm run build
```

## 🚀 快速上手

### 添加 Agent

1. 点击侧边栏的 **New Agent**
2. 填写名称、启动命令、工作目录和环境变量（可选）
3. 或点击 **📁 文件夹图标**，从项目目录自动检测配置
4. 保存后点击 ▶ 启动

### 管理 MCP Server

1. 点击左侧导航的 **MCP Servers**
2. 添加的 Server 将直接写入 Claude Desktop 的配置文件
3. 重启 Claude Desktop 后生效

> **配置文件路径：**
> - Windows：`%AppData%\Claude\claude_desktop_config.json`
> - macOS：`~/Library/Application Support/Claude/claude_desktop_config.json`

### 端口管理器

1. 点击左侧导航的 **Port Manager**
2. 查看当前所有监听端口
3. 点击 **Kill** 终止占用指定端口的进程

## 📁 项目结构

```
Agent_MCP_Manager/
├── frontend/                 # React + TypeScript 前端
│   └── src/
│       ├── components/       # AgentList、AgentDetail、AgentForm、LogViewer
│       ├── pages/            # McpManager、PortManager
│       ├── store/            # Zustand 状态（agentStore）
│       └── types/            # TypeScript 类型定义
├── src-tauri/                # Rust 后端
│   └── src/
│       ├── agent.rs          # Agent 状态与进程存储
│       ├── commands.rs       # Tauri 命令（启动/停止/日志/扫描）
│       ├── mcp.rs            # MCP 配置读写
│       ├── ports.rs          # 端口扫描与终止
│       └── lib.rs            # Tauri 应用入口
└── .github/workflows/        # CI/CD（多平台自动构建发布）
```

## 🔧 配置文件路径

Agent 配置持久化存储于系统数据目录：

| 平台 | 路径 |
|------|------|
| Windows | `%AppData%\agent-manager\agents.json` |
| macOS | `~/Library/Application Support/agent-manager/agents.json` |
| Linux | `~/.local/share/agent-manager/agents.json` |

## 🤝 参与贡献

1. Fork 本仓库
2. 创建功能分支：`git checkout -b feat/your-feature`
3. 提交更改：`git commit -m "feat: 添加你的功能"`
4. 推送并发起 Pull Request

## 📄 许可证

[MIT](LICENSE)
