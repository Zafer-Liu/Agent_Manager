#!/bin/bash
# 完整工作流链路测试脚本
#
# 前置条件：
#   1. Agent_Manager 已启动（npm run dev，HTTP server 在 9420）
#   2. 子 Agent 已启动（python your_agent.py 或 node agent_server.js，在 8502）
#
# 测试链路：
#   curl POST /hook → 启动工作流 → agent_task 节点 POST 到子 Agent
#   → 子 Agent 处理 → POST /agent/submit → 工作流推进到 acceptance 节点
#   → 等验收 → curl POST /runs/:id/approve → Run 关闭

set -e
BASE="http://localhost:9420"

echo "=========================================="
echo "  工作流引擎接通测试"
echo "=========================================="

echo ""
echo "0. 健康检查"
curl -s "$BASE/health" | python -m json.tool 2>/dev/null || echo "Agent_Manager 未启动"

echo ""
echo "1. 注册测试工作流模板（如果还没有）"
# 先读取现有工作流列表，检查是否已有 agent-test 模板
EXISTING=$(curl -s "http://localhost:1420" 2>/dev/null || true)
# 直接通过 Tauri 命令注册（前端 invoke）或手动在应用里导入
# 这里假设用户已在 Agent_Manager UI 里导入 test-workflow-template.json
echo "   请确保已在 Agent_Manager UI 中导入 spike/test-workflow-template.json"
echo "   （Agents 页面 → 工作流 tab → 导入）"

echo ""
echo "2. 通过 Hook 触发工作流"
echo "   POST $BASE/hook  template_key=agent-test"
HOOK_RESP=$(curl -s -X POST "$BASE/hook" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "测试任务",
    "description": "calculate 2+3*4",
    "template_key": "agent-test"
  }')
echo "   响应: $HOOK_RESP"

echo ""
echo "3. 等 5 秒让子 Agent 处理..."
sleep 5

echo ""
echo "4. 查看 Run 列表"
curl -s "$BASE/runs" | python -m json.tool 2>/dev/null | head -30

echo ""
echo "5. 查看 agent tasks"
curl -s "$BASE/agent/tasks" | python -m json.tool 2>/dev/null | head -30

echo ""
echo "6. 查看 agent results"
curl -s "$BASE/agent/results" | python -m json.tool 2>/dev/null | head -30

echo ""
echo "=========================================="
echo "  测试完成"
echo "=========================================="
echo ""
echo "如果 Run 状态是 waiting_acceptance，说明工作流跑通了："
echo "  Hook → 工作流启动 → agent_task 节点调子 Agent → 子 Agent 回结果 → 推进到验收"
echo ""
echo "通过验收："
echo '  curl -X POST http://localhost:9420/runs/<RUN_ID>/approve'
