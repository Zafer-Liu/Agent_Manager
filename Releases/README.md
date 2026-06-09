# 智管-Agent Manager v0.2.1

> **发布日期：** 2026 年 6 月 9 日
> **类型：** 功能版本（Feature Release）

---

## 📦 下载

| 平台 | 文件 | 说明 |
|------|------|------|
| Windows (x64) | `agent-manager_0.2.1.exe` | NSIS 安装程序，双击安装 |
| macOS (Intel) | *(即将发布)* | `.dmg` 格式 |

> **Windows 安全提示：** 安装时若弹出"未知发布者"警告，点击"更多信息" → "仍要运行"。安装包未经 Microsoft 代码签名，属正常现象。

---

## ✨ v0.2.1 新功能

### 🐙 从 GitHub 安装 Agent

在 **New Agent → 从 GitHub 安装** Tab 中，输入任意 GitHub 仓库地址，一键完成：

1. 自动拉取仓库信息（名称、描述、Stars、语言、README 摘要）
2. 选择本地存放目录（或使用默认的 `~/agent-repos/`）
3. Clone 到本地，表单字段自动填充，确认后保存即可启动

支持三种 URL 格式：
- `https://github.com/owner/repo`
- `github.com/owner/repo`
- `owner/repo`

---

### 🌟 内置推荐 Agent

打开 GitHub 安装 Tab 即可看到内置推荐，一键安装：

| Agent | 简介 | 技术栈 |
|-------|------|--------|
| **智析 · 数据分析 Agent** | 上传 CSV/Excel，用自然语言提问，自动生成图表与洞察报告 | Python · Streamlit |
| **BrainBoost · AI 思维导图** | 输入关键词，AI 自动生成思维导图，支持语音输入、节点拖拽、一键导出 | TypeScript · React |

---

### 🔗 系统代理自动检测

GitHub 拉取与 Clone 操作自动读取系统代理配置（环境变量或 Windows 注册表），无需手动设置。界面实时显示当前代理状态。

---

## 🐛 Bug 修复

| 问题 | 说明 |
|------|------|
| git clone 提示 `Could not resolve host: github.com` | 系统开启代理时，代理配置现已自动注入 git 子进程 |
| 文件夹选择器点击无响应 | Tauri 2 权限配置已修正，`dialog.open()` 现可正常弹出 |
| 编译报错 `UnknownPermission: fs allow-read-all` | 修正 `tauri.conf.json` 中 fs 插件的权限命名 |

---

## 🔄 从 v0.2.0 升级

直接安装新版安装包覆盖更新即可。Agent 配置数据（`%APPDATA%\agent-manager\`）不会丢失。

---

## 📜 历史版本

<details>
<summary>v0.2.0 · 2026 年 6 月</summary>

### 新功能

- **Manager Agent**：自然语言指挥所有 Agent，支持启动/停止/打开 UI/读取 README，会话持久化
- **Dashboard 教室视图**：所有 Agent 状态一眼可见，悬停操作
- **代理发布**：Cloudflare Tunnel 临时链接 + Caddy 反向代理长期发布
- **PTY 交互式终端**：内嵌真实终端，完整支持 Claude Code 等 TUI 工具
- **Python 自动识别增强**：支持 FastAPI / Django / Streamlit / uv / pyproject.toml

### Bug 修复

| 问题 | 说明 |
|------|------|
| Dashboard 启动 Agent 后全屏卡死 | 已修复状态更新导致的 UI 冻结问题 |
| Mindmap (Node.js) 启动进入 REPL | 已修复 npm 全局包的 PTY 交互模式检测 |
| 开发端口与其他 Vite 项目冲突 | 开发端口从 5173 改为 1420 |

</details>

---

## 📋 已知限制

- macOS 版本正在构建中，本次暂未提供
- 多用户同时访问同一 Agent 时，对话历史由 Agent 本身管理，Manager 无法外部隔离

---

## 🤝 反馈与贡献

- **Bug 报告 / 功能建议：** [GitHub Issues](https://github.com/Zafer-Liu/Agent_MCP_Manager/issues)
- **代码贡献：** 欢迎提交 PR

---

*[查看完整文档 →](https://github.com/Zafer-Liu/Agent_MCP_Manager#readme)*
