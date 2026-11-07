# 排障手册

这里的每一条都对应一个真实发生过的故障，按"现象 → 判断 → 处理"组织。

## 第 0 步：先确认网关到底在不在工作

出任何问题，先回答这三个问题，能排掉一半情况。

```bash
# 1. 进程在不在，端口通不通
lsof -nP -iTCP:15722 -sTCP:LISTEN

# 2. 上游连得上吗（返回的 upstream 是否是你以为的地址）
curl -s http://127.0.0.1:15722/_gateway/health

# 3. 规则是否真的命中过
curl -s http://127.0.0.1:15722/_gateway/stats
```

`stats` 里 `hits` 全是 0 有两种可能：请求根本没经过网关，或者规则没命中。
区分方法看网关日志，有没有 `请求体已改写` 这一行。

如果请求压根没到网关：检查 cc-switch 里那个 provider 的 API 地址是不是真的指向 `127.0.0.1:15722`。
cc-switch 切换 provider 时会按自己的配置重写 `~/.claude/settings.json`，
手改 `ANTHROPIC_BASE_URL` 的方式会在某次切换后静默失效。

## 日志在哪，怎么用日志定位

每个转发请求有一条访问日志，终端和日志文件同时输出。日志始终落盘，
目录由 `config.log.dir` 决定；留空用平台默认目录（macOS 为 `~/Library/Logs/claude-gateway/`，
可在设置页点「打开日志目录」确认实际路径，其他平台见配置文件里的注释）：

```bash
LOG_DIR="$HOME/Library/Logs/claude-gateway"

# 实时看
tail -f "$LOG_DIR/gateway.$(date +%Y-%m-%d).log"

# 只看有改写的请求
grep -v 'rewritten=-' "$LOG_DIR"/gateway.*.log

# 只看出错的请求
grep -E 'status=(4|5)[0-9]{2}' "$LOG_DIR"/gateway.*.log
```

日志行示例与各字段含义：

```
2026-10-24 14:59:34.291  INFO POST /v1/messages status=200 elapsed_ms=4 upstream_ms=4 rewritten=thinking_disable:1,tool_reference:1
```

| 字段 | 含义 | 怎么用来判断 |
| --- | --- | --- |
| `status` | 返回给客户端的状态码 | 与上游报错对照 |
| `elapsed_ms` | 网关视角总耗时 | 减去 upstream_ms 就是网关自身的开销 |
| `upstream_ms` | 到上游首字节的时间 | 明显偏大说明上游慢，不是网关的问题 |
| `rewritten` | 命中规则与次数，无命中为 `-` | 一直是 `-` 说明规则没生效或请求根本没经过 |

**没有任何访问日志**：请求没到网关，回去查第 0 步，重点是 cc-switch 的 provider 地址。

**有访问日志但 `rewritten=-`**：请求到了、转发正常，但规则没命中。按 `docs/rules.md` 检查规则的
匹配路径是否写错，或者用 echo 上游看一眼实际请求体长什么样。

**`status=502`**：网关连不上上游。日志里会紧跟着一条 `WARN 上游请求失败`，带 `url` 和 `error`，
对照 `config.upstream` 看是不是地址写错或 cc-switch 没启动。

**`status=413`**：请求体超过 `max_body_bytes`。日志里有 `limit` 和 `path`。
Claude Code 正常对话不会到这个量级，出现了通常是某个工具回传了超大内容。

**请求卡住不返回**：多半是接入配置接成了环，同一路径在日志里反复出现。见案例 7。

需要更细的信息时临时开 debug：

```bash
RUST_LOG=gateway_core=debug,claude_gateway=debug cargo run -p gateway-cli
```

注意 `RUST_LOG` 会覆盖 `config.log.level`，且是全局的，别长期开着——单条规则命中详情会刷屏。

## 案例 1：`500 Invalid content item type: tool_reference`

完整报错：

```json
{"type":"error","error":{"type":"api_error","message":"Error getting number of tokens from content list:
Error counting field 'content': Invalid content item type: tool_reference. Expected str or dict with
'type' field (text, image_url, image, document, tool_use, tool_result, thinking)."}}
```

### 怎么判断是这一类

三个特征同时出现：

1. 状态码 500，错误在 **token 计数阶段**抛的（文案里有 `number of tokens` / `counting field`）
2. 文案点名 `tool_reference`
3. **重试无效，换会话就好，但那个会话再也打不开**

第三条是这类问题最典型的表现。原因不是上游偶发，而是 `tool_reference` 已经写进会话历史，
之后每一轮请求都会把它作为上下文重放上去，所以每次都在同一处失败。

### 成因

Claude Code 2.1.x 的 `ToolSearch`（延迟工具加载）在会话里写入这种内容块：

```json
{ "type": "tool_reference", "tool_name": "WebFetch" }
```

上游网关的 token 计数器维护了一份类型白名单，只认
`text / image_url / image / document / tool_use / tool_result / thinking`，
遇到白名单外的类型直接抛异常。

### 处理

对应规则 `tool_reference`，把这类块降级成文本。语义无损：它原本只表达"以下工具已加载"。

```bash
# 确认规则开着
curl -s http://127.0.0.1:15722/_gateway/stats | grep tool_reference
```

如果确认开着但仍然报错，说明请求没经过网关，回到第 0 步。

### 已经被污染的会话怎么办

网关只能拦住之后的请求，历史里那几条 `tool_reference` 还在。两个选择：

- 直接 `/clear` 或新开会话，最省事
- 想保住上下文，就把会话 jsonl 里的 `tool_reference` 换成等价文本后重算快照 hash

第二种要小心：`~/.claude/projects/<project>/<session>.jsonl` 里 `api-request-blob` 类型的行带一个
`hash` 字段，改内容不重算会导致快照失配。hash 算法是
`sha256(json.dumps(message, sort_keys=True, separators=(',',':'), ensure_ascii=False))`。

## 案例 2：`400 thinking.type=disabled cannot be translated`

完整报错：

```json
{"type":"error","error":{"type":"invalid_request_error","message":"thinking.type=disabled cannot be
translated: configure model_info.thinking_disable_mode only after verifying the upstream contract"}}
```

### 怎么判断是这一类

- 状态码 400，不是 500
- 文案点名 `thinking_disable_mode`
- **只在开了 auto 权限模式时出现**，普通模式下一切正常
- 用 cc-switch 的请求日志看，这类请求全部是**非流式**的短请求（几百毫秒），
  而主对话请求都是流式

### 成因

Claude Code 的 auto 权限模式有一个分类器，每次要判定工具调用是否放行就发一个独立的短请求。
这个请求显式带 `thinking: {"type": "disabled"}`。上游用的 LiteLLM 需要配置
`model_info.thinking_disable_mode` 才知道怎么翻译这个取值，没配就直接 400。

这解释了为什么只有 auto mode 报错：主对话请求带的是 `thinking.enabled`（甚至带 budget），
分类器请求才带 `disabled`。

### 处理

对应规则 `thinking_disable`，把整个 `thinking` 字段删掉。删除等价于"不声明 thinking"，语义一致。

### 这个错误会影响功能吗

分类器请求失败，auto 模式拿不到判定结果，通常会退回询问用户，不至于让主流程崩掉。
所以症状是"能用但一直弹报错"。网关修好之后这个噪音就消失了。

## 案例 3：上游收到的 URL 变成完整地址

### 现象

在上游服务端的日志里看到请求行是绝对地址：

```
POST http://127.0.0.1:15722/v1/messages HTTP/1.1
```

而不是：

```
POST /v1/messages HTTP/1.1
```

### 成因

环境里有 `HTTP_PROXY` / `HTTPS_PROXY`（常见于 IDE、容器、代理软件注入）。
reqwest 默认读这些变量，于是本该直连本地的请求被送到代理，代理转发时用 absolute-form。

```bash
env | grep -i proxy
```

### 处理

确认 `config.toml` 里：

```toml
use_system_proxy = false
```

这个默认值不要改。只有当上游确实需要经代理访问时才打开，而且打开后要接受请求行变成绝对形式。

## 案例 4：规则命中数在涨，请求却还是失败

说明改写生效了，但不是上游要的那个形态。逐层查：

**4.1 改写后的 body 到底长什么样**

把上游临时换成本地 echo，直接看收到的内容：

```bash
# 临时配置，listen 换个端口避免冲突
cat > /tmp/probe.toml <<'EOF'
listen = "127.0.0.1:15796"
upstream = "http://127.0.0.1:15795"
use_system_proxy = false
EOF
```

起一个能打印 body 的小服务当上游，把真实请求打过去，比对改写前后的差异。
多数时候是字段位置不对——比如规则写在 `messages[].content`，但实际字段在 `system` 或顶层。

**4.2 是不是改完之后才报的新错**

有些字段是联动关系。删掉 `thinking` 之后，如果 assistant 消息里还留着 `thinking` 内容块，
上游会换一个报错（常见的是签名相关的 `Invalid signature in thinking block`）。
这种情况要加规则把消息体里的残留块也清掉，而不是回退。

**4.3 上游缓存了旧的失败**

少数网关会按请求体 hash 缓存失败结果。改写后 hash 变了，通常不会命中，
但如果命中，重启上游或换个 `max_tokens` 试一次。

**4.4 你改的不是这条链路**

最常见的乌龙：本地改了代码，跑的是另一个端口的旧进程。用 `lsof -nP -iTCP:15722` 对一下 PID，
`ps -p <pid> -o command=` 看它跑的到底是哪个二进制。

## 案例 5：想临时关掉某条规则验证

不用改代码：

```toml
[rules.tool_reference]
enabled = false
```

重启进程即可。改完记得改回来，或者明确决定这条规则该删。

关掉规则后请求重新开始报错，说明这条规则是必需的；
关掉后一切正常，说明上游已经支持该字段，可以按 `docs/rules.md` 末尾的流程删掉它。

## 案例 6：SSE 变成一次性输出

客户端不再逐字显示，而是等很久后整段刷出来。

### 判断

用 curl 直接看分段：

```bash
curl -N http://127.0.0.1:15722/v1/messages -H 'content-type: application/json' -d @payload.json
```

看数据块是不是分批到达。如果经过网关后才变成一次性，问题在本项目的回传路径。

### 常见原因

- 用了 `.bytes().await` 把响应读完再返回（本项目用的是 `Body::from_stream`，不该出现）
- 中间又插了一层会缓冲的代理
- 上游自己就在缓冲（换直线的请求对比一下）

## 案例 7：请求卡住不返回，最后客户端超时

### 现象

Claude Code 发出请求后没有任何输出，等很久最后超时。网关日志里同一个路径反复出现，
比如连续多条 `POST /v1/messages`，且都没有正常返回。

### 成因

接成环了。`upstream` 指向 cc-switch，同时 cc-switch 里该 provider 的 API 地址又指向网关：

```
gateway  ──upstream──▶  cc-switch  ──provider──▶  gateway  ──▶ ...
```

请求在两个进程之间来回转发，谁都不去真上游。

### 处理

两种接法只能选一种，三处配置必须同向对齐：

**方式 A（gateway 在 cc-switch 后面，推荐）**

| 配置项 | 值 |
| --- | --- |
| `config.toml` 的 `upstream` | 真实上游，如 `https://your-upstream.example.com/v1` |
| cc-switch 的 provider API 地址 | `http://127.0.0.1:15722` |
| `ANTHROPIC_BASE_URL` | 不动 |

**方式 B（gateway 在 cc-switch 前面）**

| 配置项 | 值 |
| --- | --- |
| `config.toml` 的 `upstream` | `http://127.0.0.1:15721` |
| cc-switch 的 provider API 地址 | 不动，保持真实上游 |
| `ANTHROPIC_BASE_URL` | `http://127.0.0.1:15722` |

### 怎么验证已经修好

发一个请求，日志里应该只有 **一条** 访问记录：

```bash
tail -f "$HOME/Library/Logs/claude-gateway/gateway.$(date +%Y-%m-%d).log"
```

看到 `status=200` 且 `rewritten=` 有内容，就对了。

## 案例 8：手工测试返回 401 `LiteLLM Virtual Key expected`

**识别特征**：用 `PROXY_MANAGED` 占位符 curl 网关，响应是
`authentication_error ... Received=PROX****AGED, expected to start with 'sk-'`。

**原因**：`PROXY_MANAGED` 占位符只在方式 B（网关 upstream 指向 cc-switch 的 15721）下有效——
cc-switch 收到请求后会用 provider 的真实 key 替换它。
方式 A 下网关直连真实上游，占位符被原样转发，LiteLLM 只认 `sk-` 开头的 key，直接 401。
历史上能用是因为当时按方式 B 接入；切换接法后这个「免 key 测试通道」随之失效。

**验证一点**：日志里零星的 `status=401` 访问记录，多半就是这类手工测试请求，
与 Claude Code 的正常流量无关。

**正确做法**：测试脚本自带 key 参数，见 `scripts/check-thinking.sh`：

```bash
./scripts/check-thinking.sh sk-你的key gpt-6.1-sol-lite
```

key 只在进程内使用，不落盘、不打印。

## 案例 9：客户端报 `Server error mid-response`

**识别特征**：Claude Code 显示 `API Error: Server error mid-response. The response above may be
incomplete.`，而网关访问日志里对应请求是 `status=200`。

**为什么访问日志看不出来**：状态码在响应头返回时就定了，流是之后才断的。

**判读**：看 WARN 级日志，按三种情况分流。

```bash
grep WARN logs/gateway.$(date +%F).log | tail -20
```

| 日志 | 含义 | 责任方 |
| --- | --- | --- |
| `上游流中断（响应未完成） ... error=...` | 上游连接层出错（重置、解码失败） | 上游网络/网关 |
| `上游流提前结束（未见结束标记）` | 上游主动关闭，但没发 `message_stop` | 上游服务端 |
| `客户端提前断开，上游流未读完`（DEBUG） | 客户端主动取消 | 客户端 |
| 无任何流日志 | 请求没到网关 | 更上游的链路 |

判定依据是 `message_stop`（Anthropic）、`[DONE]`（OpenAI）、`response.completed`（Responses API）
任一结束标记；跨 chunk 被切开的情况已经处理，不会误报。

**一个实测案例**：上游在发完 `message_start`（369 字节）后静默关闭，既没有 error 事件也没有
usage 统计，占当天请求的 1.3%。特征是两个 WARN 之一 + `bytes` 很小。这类问题应在
上游侧修，网关只是如实转发它看到的结束方式。

## 速查

| 命令 | 用途 |
| --- | --- |
| `lsof -nP -iTCP:15722 -sTCP:LISTEN` | 网关是否在跑 |
| `curl -s localhost:15722/_gateway/health` | 上游地址是否符合预期 |
| `curl -s localhost:15722/_gateway/stats` | 各规则开关与累计命中 |
| `./scripts/check-thinking.sh sk-xxx [模型]` | 验证某模型是否真的关闭了思考 |
| `env \| grep -i proxy` | 是否有代理变量劫持 |
| `ps -p <pid> -o command=` | 跑的是不是你以为的那个二进制 |
| `RUST_LOG=gateway_core=debug,claude_gateway=debug cargo run -p gateway-cli` | 打开逐请求的详细日志 |
