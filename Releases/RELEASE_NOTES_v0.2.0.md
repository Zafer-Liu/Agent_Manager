# 智管-Agent Manager v0.2.0

> **发布日期：** 2026 年 6 月  
> **类型：** 功能版本（Major Feature Release）

---

## 📦 下载

| 平台 | 文件 | 说明 |
|------|------|------|
| Windows (x64) | `agent-manager_0.2.0.exe` | NSIS 安装程序，双击安装 |
| macOS (Intel) | *(即将发布)* | `.dmg` 格式 |

> **Windows 安全提示：** 安装时若弹出"未知发布者"警告，点击"更多信息" → "仍要运行"。安装包未经 Microsoft 代码签名，属正常现象。

---

## ✨ v0.2.0 新功能

### 🤖 Manager Agent — 自然语言指挥官

用一句话控制所有 Agent，无需手动操作界面：

```
帮我启动 Business Analytics Agent，然后打开它的界面
把 Mindmap 停掉，顺便告诉我它是干什么的
现在有哪些 Agent 在运行？
```

支持：一键启动/停止、打开 UI、打开终端、读取 Agent README、页面跳转。会话持久化，切换页面后对话历史不丢失。

> 需配置 LLM 提供商（支持 DeepSeek / OpenAI / 任意兼容 API）。

---

### 🏫 Dashboard 教室视图

所有 Agent 以"教室"布局可视化呈现：Manager Agent 在讲台，其他 Agent 依次就座。一眼看清所有 Agent 状态；悬停可直接启停、查看详情、打开 UI。

---

### 🌐 代理发布 — 一键公网分享

无需固定 IP、域名或服务器：

- **Cloudflare Tunnel 临时链接**：一键生成 `https://xxx.trycloudflare.com`，会后关闭链接立即失效，适合开会演示
- **Caddy 反向代理长期发布**：绑定自定义域名，自动申请 HTTPS 证书，支持用户名/密码访问控制，多用户权限精细管理

---

### 💻 PTY 交互式终端

内嵌真实的 PTY 终端，完整支持 Claude Code 等 TUI 工具。不再需要切换到外部终端。

---

### 🔍 Python 项目自动识别增强

新增支持：**FastAPI、Django、Streamlit、uv / pyproject.toml**。选目录后自动填写正确的启动命令与端口号。

---

## 🐛 Bug 修复

| 问题 | 说明 |
|------|------|
| Dashboard 启动 Agent 后全屏卡死 | 已修复状态更新导致的 UI 冻结问题 |
| Mindmap (Node.js) 启动进入 REPL | 已修复 npm 全局包的 PTY 交互模式检测 |
| 开发端口与其他 Vite 项目冲突 | 开发端口从 5173 改为 1420 |

---

## 🔄 从 v0.1.0 升级

直接安装新版安装包即可覆盖更新。Agent 配置数据（`%APPDATA%\agent-manager\`）不会丢失。

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
