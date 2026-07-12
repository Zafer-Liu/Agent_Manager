"""
Mock Agent — 模拟子 Agent 接收任务并回调结果

用法：
  python mock_agent.py --port 8501

工作流：
  1. 启动后在 http://localhost:8501/task 监听任务
  2. 收到任务后，等待 2 秒模拟处理
  3. 处理完成后 POST 结果到 Agent_Manager 的 /agent/submit

测试完整闭环：
  1. 启动 Agent_Manager（已内置 HTTP server 在 9420）
  2. 启动这个 mock agent: python mock_agent.py --port 8501
  3. 在另一个终端发任务:
     curl -X POST http://localhost:9420/agent/dispatch \
       -H "Content-Type: application/json" \
       -d '{"task_id":"test-1","agent_id":"mock-agent","agent_url":"http://localhost:8501/task","task":"分析这批数据","context":{}}'
  4. 查看结果:
     curl http://localhost:9420/agent/results
"""

import argparse
import json
import time
import threading
from http.server import HTTPServer, BaseHTTPRequestHandler

AGENT_MANAGER_URL = "http://localhost:9420/agent/submit"


class AgentHandler(BaseHTTPRequestHandler):
    def do_OPTIONS(self):
        self.send_response(204)
        self._send_cors_headers()
        self.end_headers()

    def do_POST(self):
        if self.path != "/task":
            self.send_error(404)
            return

        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length).decode("utf-8")
        payload = json.loads(body)

        task_id = payload.get("task_id", "unknown")
        task = payload.get("task", "")
        callback_url = payload.get("callback_url", AGENT_MANAGER_URL)

        print(f"\n[mock-agent] 收到任务:")
        print(f"  task_id: {task_id}")
        print(f"  task: {task}")
        print(f"  callback_url: {callback_url}")

        # 立即返回 202（异步处理）
        self.send_response(202)
        self._send_cors_headers()
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps({"status": "accepted", "task_id": task_id}).encode())

        # 异步处理任务
        def process_and_callback():
            print(f"[mock-agent] 处理中... (模拟 2 秒)")
            time.sleep(2)

            result = {
                "task_id": task_id,
                "agent_id": "mock-agent",
                "result": {
                    "summary": f"已完成: {task}",
                    "details": "这是模拟处理结果",
                    "processed_at": time.strftime("%Y-%m-%d %H:%M:%S"),
                },
                "verdict": "pass",
                "note": "mock agent 处理成功",
                "submitted_at": int(time.time() * 1000),
            }

            print(f"[mock-agent] 提交结果到 {callback_url}")
            try:
                import urllib.request
                req = urllib.request.Request(
                    callback_url,
                    data=json.dumps(result).encode("utf-8"),
                    headers={"Content-Type": "application/json"},
                    method="POST",
                )
                resp = urllib.request.urlopen(req, timeout=10)
                print(f"[mock-agent] 结果已提交 (HTTP {resp.status})")
            except Exception as e:
                print(f"[mock-agent] 提交失败: {e}")

        threading.Thread(target=process_and_callback, daemon=True).start()

    def _send_cors_headers(self):
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")

    def log_message(self, format, *args):
        pass  # 静默默认日志


def main():
    parser = argparse.ArgumentParser(description="Mock Agent for Agent_Manager HTTP spike")
    parser.add_argument("--port", type=int, default=8501, help="监听端口")
    args = parser.parse_args()

    server = HTTPServer(("127.0.0.1", args.port), AgentHandler)
    print(f"[mock-agent] 监听 http://localhost:{args.port}/task")
    print(f"[mock-agent] 回调地址: {AGENT_MANAGER_URL}")
    print(f"[mock-agent] 等待任务...\n")

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n[mock-agent] 退出")
        server.shutdown()


if __name__ == "__main__":
    main()
