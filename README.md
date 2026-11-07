# claude-gateway

夹在 Claude Code（可经 cc-switch）与上游模型网关之间的兼容层。

上游网关（多为 LiteLLM 系）的字段支持总是落后于 Claude Code 客户端。
客户端每加一个新字段，网关就可能因为不认识而直接返回 4xx/5xx。
这个项目在转发前把网关无法翻译的字段就地改写掉，让链路继续可用。

## 它解决的两个具体问题

| 现象 | 成因 | 本项目的处理 |
| --- | --- | --- |
| `500 Invalid content item type: tool_reference`，且会话永久卡死 | Claude Code 的 ToolSearch 返回 `{"type":"tool_reference"}` 内容块，网关 token 计数器不认 | 降级为等价的 text 块 |
| `400 thinking.type=disabled cannot be translated` | auto 权限模式的分类器请求显式禁用 thinking，网关未配 `model_info.thinking_disable_mode` | 删除整个 `thinking` 字段 |

第一条之所以会"卡死"，是因为该内容块一旦写进会话历史，之后每轮请求都会重放，所以不是偶发失败。

## 快速开始

命令行：

```bash
cargo build -p gateway-cli --release
./target/release/claude-gateway
```

桌面端（macOS，前端用 pnpm 管理）：

```bash
cd crates/gateway-app/ui
pnpm install --frozen-lockfile

cd ..
./ui/node_modules/.bin/tauri build   # 产物 target/release/bundle/macos/Claude Gateway.app
```

配置和日志固定在平台目录，不随启动目录变化：

| 内容 | macOS | Linux | Windows |
| --- | --- | --- | --- |
| 配置 | `~/.config/claude-gateway/config.toml` | 同左 | `%APPDATA%\claude-gateway\config.toml` |
| 日志 | `~/Library/Logs/claude-gateway/` | `~/.local/state/claude-gateway/logs/` | `%LOCALAPPDATA%\claude-gateway\logs\` |

首次启动会自动生成配置（有旧配置则迁移一份），不需要手动建目录或复制文件。
上游地址、日志级别、规则开关都能在设置页修改，改完立即生效并写回配置文件。

仓库里的 `config.example.toml` 只是示例，程序不读它。`CLAUDE_GATEWAY_CONFIG`
环境变量可显式指定配置路径，优先级最高。

## 接入方式

`upstream` 和「cc-switch 的 provider 地址」必须配套，**两种接法只能选一种**。
混用会形成循环——网关把请求发给 cc-switch，cc-switch 又按 provider 地址转回网关，
表现是请求卡住直到超时。

### 方式 A：gateway 在 cc-switch 后面（推荐）

```
Claude Code -> cc-switch(15721) -> claude-gateway(15722) -> 上游网关
```

| 改哪里 | 改成什么 |
| --- | --- |
| 设置页的「上游地址」 | 真实上游，如 `https://your-upstream.example.com/v1` |
| cc-switch 里该 provider 的 API 地址 | `http://127.0.0.1:15722` |
| Claude Code 的 `ANTHROPIC_BASE_URL` | 不动 |

优点：`settings.json` 完全不动，cc-switch 的用量统计、熔断、故障转移全部照常工作。

### 方式 B：gateway 在 cc-switch 前面

```
Claude Code -> claude-gateway(15722) -> cc-switch(15721) -> 上游网关
```

| 改哪里 | 改成什么 |
| --- | --- |
| 设置页的「上游地址」 | `http://127.0.0.1:15721`（默认值） |
| Claude Code 的 `ANTHROPIC_BASE_URL` | `http://127.0.0.1:15722` |
| cc-switch 里该 provider 的 API 地址 | 不动，保持真实上游 |

缺点：cc-switch 会按 `proxy_config.listen_port` 重写 `settings.json` 的 `ANTHROPIC_BASE_URL`，
切换 provider 后这个接法会静默失效，表现为问题重新出现但没有任何提示。

### 怎么确认没接成环

改动后发一个请求，看日志里出现的是 **一条** 访问记录还是同一路径反复出现：

```bash
# macOS；Linux 换成 ~/.local/state/claude-gateway/logs/
tail -f ~/Library/Logs/claude-gateway/gateway.$(date +%Y-%m-%d).log
```

正常是一条 `status=200`。如果看到同一个 `POST /v1/messages` 连续多次且客户端超时，
就是接成环了，把上面三处按同一种方式重新对齐。

## 配置

下面是 `config.example.toml` 的完整内容。实际配置文件在平台目录，首次启动自动生成：

```toml
listen = "127.0.0.1:15722"           # 监听地址
upstream = "http://127.0.0.1:15721"  # 上游地址，不带结尾斜杠
max_body_bytes = 67108864             # 单条请求体上限
use_system_proxy = false              # 见下方说明

[log]
level = "info"                        # trace/debug/info/warn/error，RUST_LOG 优先
dir = ""                              # 留空用平台日志目录；相对路径以配置文件目录为基准
retain_days = 7                       # 最多保留多少个日志文件

[rules.thinking_disable]
enabled = true

[rules.tool_reference]
enabled = true
```

`use_system_proxy` 默认 false 是有原因的：上游通常是本机 cc-switch，
如果环境里有 `HTTP_PROXY`，请求会被路由到代理，请求行还会变成 absolute-form，
部分上游会直接拒绝。除非上游确实需要经代理访问，否则保持 false。

单条规则未在配置里出现时视为启用。

## 加一条新规则

三步，主流程不用动：

1. 在 `crates/gateway-core/src/rewrite/` 下新建文件，实现 `RewriteRule`：

```rust
pub trait RewriteRule: Send + Sync {
    fn id(&self) -> &'static str;      // 同时是配置文件里的键名
    fn label(&self) -> &'static str;   // 面板/统计里展示的说明
    fn apply(&self, body: &mut Value) -> usize;  // 返回本次命中次数
}
```

2. 在 `crates/gateway-core/src/rewrite/mod.rs` 的 `Registry::with_default_rules()` 里注册一行
3. 在配置文件里加一个开关（可选，不加视为启用）

`apply` 返回的命中次数会累加到 `/_gateway/stats`，用于确认规则是否真的在生效。

## 控制面

| 端点 | 说明 |
| --- | --- |
| `GET /_gateway/health` | 存活检查，返回当前 upstream |
| `GET /_gateway/stats` | 各规则的开关状态与累计命中次数 |

其余所有路径都透传给上游，不区分 `/v1/messages` 等具体端点。

## 设计取舍

- **不做鉴权、不做 provider 管理、不统计用量**。这些是 cc-switch 的职责，
  本项目只做一件事：改写。职责单一，才不会和 cc-switch 的版本演进互相牵连。
- **流式不缓冲**。响应用 `Body::from_stream` 直接透传，SSE 才能逐块到达客户端。
- **请求体上限可配**。超过上限直接 413，避免被超大请求打爆内存。
- **不改写非 JSON 请求**。解析失败时原样透传，不让网关成为新的故障点。

## 日志

每个转发的请求产出一条访问日志，含耗时、上游首字节耗时、改写命中：

```
2026-10-24 14:59:34.291  INFO POST /v1/messages status=200 elapsed_ms=4 upstream_ms=4 rewritten=thinking_disable:1,tool_reference:1
```

- `elapsed_ms` 网关视角的总耗时
- `upstream_ms` 到上游首字节的时间，不含流式 body 的传输时长
- `rewritten` 本次命中的规则及次数，无命中为 `-`

`/_gateway/*` 控制面请求不产生访问日志。日志按**本地日期**轮转为
`gateway.YYYY-MM-DD.log`，落在平台日志目录，最多保留 `retain_days` 个文件。

**不记录请求体和响应体**，只记元数据。请求体里是完整对话内容和代码，落盘等于把聊天记录明文存到磁盘。

## 文档

| 文档 | 内容 |
| --- | --- |
| `AGENTS.md` | 仓库开发约定：开工前必读、实时同步规则、命令、目录职责、代码规则、已知环境坑 |
| `docs/decisions.md` | 决策记录：已确认 / 待确认 / 已废弃，含理由与被否方案 |
| `docs/STATUS.md` | 当前状态快照：分支、未提交改动、已验证与未验证、下一步 |
| `docs/architecture.md` | 请求生命周期、模块职责、关键设计决策及理由、已知边界 |
| `docs/rules.md` | 规则开发指南：契约、现有规则清单、从零加一条规则的走查、反模式 |
| `docs/troubleshooting.md` | 排障手册：六个真实故障案例 + 速查命令表 |
| `docs/README.md` | 文档索引与维护约定 |

## 已验证

- 61 个 Rust 测试覆盖改写规则边界、按本地日期轮转、配置迁移与落盘、面板接口
- 12 个前端测试覆盖日志页与格式化；`clippy` / `eslint` 全绿
- 本地回环：改写生效、SSE 两段间隔 306ms 证实无缓冲、请求行保持相对路径
- 真实链路：同一请求直连 cc-switch 返回 400，经本网关返回 200

## 开源协议

[MIT](LICENSE) © 2026 promise94
