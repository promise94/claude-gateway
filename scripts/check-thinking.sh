#!/bin/bash
# 检查某模型在当前链路下是否真的关闭了思考。
#
# 用法:
#   ./check-thinking.sh sk-你的key [模型名] [网关地址]
#
# 原理: 同一请求分别带/不带 thinking:{type:disabled} 发两次,
#       对比响应里 content 块的类型。带 thinking 的请求会先被网关剥掉字段,
#       所以两次实际到达上游的 payload 相同; 若两者都出现 thinking 块,
#       说明上游对该模型的默认行为是开思考, 客户端无法关闭。
#
# 注意: key 只在本进程内使用, 不会落盘、不会打印。

set -u

KEY="${1:-}"
MODEL="${2:-gpt-6.1-sol-lite}"
GATEWAY="${3:-http://127.0.0.1:15722}"

if [ -z "$KEY" ]; then
  echo "用法: $0 sk-你的key [模型名] [网关地址]" >&2
  exit 1
fi

run_one() {
  local TH="$1"
  local BODY="{\"model\":\"$MODEL\",\"max_tokens\":256,${TH}\"messages\":[{\"role\":\"user\",\"content\":\"用一句话解释TCP三次握手\"}]}"
  curl -s -m 90 "$GATEWAY/v1/messages" \
    -H "Authorization: Bearer $KEY" -H "x-api-key: $KEY" \
    -H "anthropic-version: 2023-06-01" -H "content-type: application/json" \
    --data-raw "$BODY"
}

report() {
  local LABEL="$1" RESP="$2"
  echo "--- $LABEL ---"
  echo "$RESP" | python3 -c '
import json,sys
try:
    d=json.load(sys.stdin)
except Exception as e:
    print("  响应不是 JSON:", e); sys.exit(1)
if d.get("type")=="error":
    print("  请求失败:", json.dumps(d.get("error",d),ensure_ascii=False)[:300]); sys.exit(1)
types=[b.get("type") for b in d.get("content",[])]
usage=d.get("usage",{})
print("  content 类型:", types or "(空)")
print("  usage:", {k:usage.get(k) for k in ("input_tokens","output_tokens")})
'
}

RESP_NO=$(run_one "")
RESP_DIS=$(run_one '"thinking":{"type":"disabled"},')

echo "模型: $MODEL   网关: $GATEWAY"
report "无 thinking 字段" "$RESP_NO"
report "带 thinking:disabled (网关会剥离)" "$RESP_DIS"

echo
echo "判读: 两次都没有 thinking 块 → 已等效关闭;"
echo "      两次都有 → 上游默认开思考, 客户端无法关闭, 需上游配置 thinking_disable_mode。"
