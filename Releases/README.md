## v0.3.0 重点更新

### 阶段四：外部协作能力（P0-P2 全部完成）

将 Agent Manager 从"用户手动操作"的本地工具，升级为"外部系统可推送任务、多 Agent 协作执行、结果自动回传"的自动化平台。

#### P0 — 基础通信层

- **本地 Hook Server**（`127.0.0.1:9420`）
  - 外部系统通过 HTTP POST `/hook` 推送任务，按 `template_key` 匹配工作流模板
  - HTTP API：`GET /runs/:id` 查询状态、`POST /runs/:id/approve` 验收、`POST /runs/:id/reject` 驳回、`POST /agent/submit` 子 Agent 提交结果
  - 端口和 auth_token 可配置，持久化到 `hook_server.json`
  - **Callback 出站通知**：Run 终态时回调外部 URL，指数退避重试 3 次

- **agent_task 节点**
  - 工作流可调度本地/远程子 Agent（不限于 MCP Server）
  - oneshot channel 挂起等待子 Agent 结果（120s 超时）
  - `RunTrigger` 枚举：Manual / Hook / Rework / Schedule

#### P1 — 前端可视化 + 并行执行

- **ExternalTriggers 设置页**：Hook Server 状态显示、端口/Token 配置、curl 示例、重启功能
- **运行历史来源列**：区分手动/外部/返工/定时触发，彩色标签
- **Fan-out 并行执行**：支持 static / by_field / llm_split 拆分策略，`join_all` 并行执行子任务
- **Fan-out 子步骤展示**：子任务结果记录为独立 StepInstance，前端折叠分组渲染
- **Cloudflare Tunnel 隧道条目**：ProxyManager 动态获取 Hook 端口传入隧道配置

#### P2 — 高级调度 + 传输扩展

- **DispatchStrategy 调度策略**：Fixed / Failover / CapabilityMatch / Random，含候选 Agent 列表和能力匹配
- **定时触发（cron）**：自建 5 字段 cron 解析器（不引入外部 crate），后台线程每分钟检查
- **McpTransport Http 变体**：支持远程 MCP Server（Streamable HTTP transport）
- **SVG DAG 画布**：从线性列表升级为 SVG 绘制的 DAG 画布，支持拖拽定位、工具栏、配置面板

### 工作流引擎核心能力

- **Acceptance/Rework 闭环**：验收节点支持通过/驳回，驳回定向回到出问题节点，对话流内嵌验收卡片
- **Sweeper 自愈巡检**：后台 30s interval 检测 Step 超时并标记 failed
- **FailureTrace 诊断链路**：MCP stderr ring buffer + 失败原因分类（timeout/agent_exit/stderr/tool_error等）+ 重试历史
- **Metrics 事件源**：append-only `workflow_events.jsonl` + 前端四卡片本地聚合（总运行/成功率/平均耗时/活跃 Agent）

### 文件变更

| 文件 | 类型 | 说明 |
|------|------|------|
| `src-tauri/src/agent_http.rs` | 修改 | Hook Server + Callback 重试 + HTTP API |
| `src-tauri/src/scheduler.rs` | **新增** | 定时触发调度器 + cron 解析 |
| `src-tauri/src/workflow.rs` | 修改 | agent_task/FanOut/DispatchStrategy/RunTrigger/schedule |
| `src-tauri/src/workflow_store.rs` | 修改 | StepInstance 新增 kind/failure_trace 字段 |
| `src-tauri/src/mcp_agent.rs` | 修改 | McpTransport Http 变体 + stderr ring buffer |
| `src-tauri/src/sweeper.rs` | **新增** | 后台巡检 + Step 超时检测 |
| `src-tauri/src/workflow_events.rs` | **新增** | JSONL 事件源 + Metrics 聚合 |
| `src-tauri/src/lib.rs` | 修改 | 启动 Hook Server + scheduler + sweeper |
| `frontend/src/pages/ExternalTriggers.tsx` | **新增** | Hook Server 设置页 |
| `frontend/src/pages/WorkflowBuilder.tsx` | 修改 | SVG DAG 画布 + DispatchEditor + cron 校验 |
| `frontend/src/pages/WorkflowRunView.tsx` | 修改 | 触发徽章 + 节点类型徽章 + FanOut 分组 + FailureTrace 面板 |
| `frontend/src/pages/RunsHistory.tsx` | 修改 | 触发来源列 |

---

## 从 v0.2.3 升级

直接安装 v0.3.0 覆盖旧版本即可。Agent、LLM、代理、MCP、GitHub Token 和工作流配置会继续保留。

升级后如需使用 Hook Server，前往 **设置 → External Triggers** 配置端口和 auth_token。

---

## 历史版本

- **v0.2.3**：可视化工作流、MCP Agent 节点、中英文 i18n、CI 流水线
- **v0.2.2**：工作流引擎、MCP Agent 节点、国际化及 MCP 配置持久化
- **v0.2.1**：GitHub 安装 Agent、推荐项目、系统代理检测与 Tauri 权限修复
- **v0.2.0**：Manager Agent、Dashboard、PTY、代理发布和项目自动识别
