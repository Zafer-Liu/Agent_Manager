#!/bin/bash
# HTTP 双向通信 spike 测试脚本
#
# 前置条件：
#   1. Agent_Manager 已启动（HTTP server 在 9420）
#   2. mock_agent.py 已启动（在 8501）
#
# 用法：
#   bash spike/test_spike.sh

set -e

BASE="http://localhost:9420"

echo "=========================================="
echo "  Agent HTTP 双向通信 Spike 测试"
echo "=========================================="

echo ""
echo "1. 健康检查"
echo "   GET $BASE/health"
curl -s "$BASE/health" | python -m json.tool

echo ""
echo "2. 发任务给 mock agent"
echo "   POST $BASE/agent/dispatch"
TASK_ID="test-$(date +%s)"
RESP=$(curl -s -X POST "$BASE/agent/dispatch" \
  -H "Content-Type: application/json" \
  -d "{
    \"task_id\": \"$TASK_ID\",
    \"agent_id\": \"mock-agent\",
    \"agent_url\": \"http://localhost:8501/task\",
    \"task\": \"分析这批销售数据并给出趋势判断\",
    \"context\": {\"data\": [100, 120, 115, 130, 145]}
  }")
echo "   响应: $RESP"

echo ""
echo "3. 等 3 秒让 mock agent 处理..."
sleep 3

echo ""
echo "4. 查看任务列表"
echo "   GET $BASE/agent/tasks"
curl -s "$BASE/agent/tasks" | python -m json.tool

echo ""
echo "5. 查看结果列表"
echo "   GET $BASE/agent/results"
curl -s "$BASE/agent/results" | python -m json.tool

echo ""
echo "=========================================="
echo "  测试完成"
echo "=========================================="
echo ""
echo "如果结果列表里有 task_id=$TASK_ID 的记录，说明双向通信成功。"
echo "  Agent_Manager → mock agent: POST /agent/dispatch"
echo "  mock agent → Agent_Manager: POST /agent/submit"
