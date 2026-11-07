# 架构

## 在链路中的位置

推荐接法（下称方式 A）：gateway 在 cc-switch **后面**。

```
Claude Code
    │  ANTHROPIC_BASE_URL（不动，仍指向 cc-switch）
    ▼
cc-switch 本地代理  127.0.0.1:15721
    │  provider 的 API 地址（改成 127.0.0.1:15722）
    ▼
claude-gateway      127.0.0.1:15722
    │  config.toml 的 upstream（改成真实上游）
    ▼
上游模型网关        https://your-upstream.example.com/v1 之类
```

选这个方向的原因：客户端到 cc-switch 这一段完全不动，`settings.json` 不会被改写，
cc-switch 的用量统计、熔断、故障转移继续正常工作，本项目只承担改写职责。

另一种接法（方式 B）是 gateway 在 cc-switch **前面**：

```
Claude Code -> claude-gateway(15722) -> cc-switch(15721) -> 上游网关
```

此时 `upstream` 指向 cc-switch（`config.toml` 的默认值就是这个），
`ANTHROPIC_BASE_URL` 指向 gateway。缺点是 cc-switch 会重写 `settings.json`，切换 provider 后静默失效。

**两个方向不能混配。** `upstream` 指向 cc-switch、同时又让 cc-switch 的 provider 指向 gateway，
就构成一个环：gateway 转发给 cc-switch，cc-switch 又转回 gateway，请求会一直转直到客户端超时。
判断方法是看访问日志——正常一条请求一条记录，成环时同一路径会反复出现。

两种接法的完整配置组合见 `README.md` 的「接入方式」。

## 请求生命周期

一个请求从进入到返回，只经过四个阶段：

```
1. 接收
   axum Router
     ├─ GET /_gateway/health  → 控制面
     ├─ GET /_gateway/stats   → 控制面
     └─ 其余所有路径 → fallback → server::proxy
          axum::body::to_bytes(body, max_body_bytes)
          超限直接 413，不再向上游发

2. 改写  Gateway::rewrite_body
   非 JSON content-type        → 原样返回
   JSON 解析失败               → 原样返回（打 debug 日志）
   解析成功 → Registry::apply
        ├─ 逐条判断 config 里的规则开关
        ├─ 依次调用 RewriteRule::apply
        └─ 汇总命中次数，写 info 日志

3. 转发  Gateway::handle
   URL = upstream_base + path_and_query
   拷贝请求 header，跳过 host / content-length / accept-encoding / 逐跳头
   强制补 accept-encoding: identity
   reqwest 发送（不设总超时，只有 connect_timeout 10s）

4. 回传
   状态码原样
   响应 header 跳过 content-length / 逐跳头
   response.bytes_stream() → Body::from_stream 逐块给客户端
```

第 4 步不做任何缓冲。SSE 能不能逐块到达，取决于这里是否被中间层攒起来。

## 模块职责

| 文件 | 职责 | 不该做的事 |
| --- | --- | --- |
| `main.rs` | 读配置、建 client、绑端口、处理 SIGINT/SIGTERM | 不含任何改写或路由逻辑 |
| `config.rs` | TOML 解析、默认值、规则开关查询 | 不做 IO 之外的事 |
| `server.rs` | 路由、控制面端点、读取请求体、413 兜底 | 不直接拼上游请求 |
| `forward.rs` | 改写调度、上游请求构造、流式回传 | 不含任何具体规则 |
| `rewrite/mod.rs` | `RewriteRule` 契约、注册表、累计命中 | 不知道 HTTP 的存在 |
| `rewrite/*.rs` | 单条规则的 JSON 改写 | 不读配置、不发请求、不做日志之外的事 |

这个划分的目的是：**加规则时只碰 `rewrite/`，转发出问题时只看 `forward.rs`**。

## 关键设计决策

### 流式不缓冲

```rust
let stream = response.bytes_stream();
builder.body(Body::from_stream(stream))
```

如果用 `response.bytes().await` 再返回，SSE 会被完整攒下来，客户端的"逐字输出"就没了。
验证方式是让上游分两次、间隔 300ms 发数据，看客户端是否也间隔到达。

### 强制 `accept-encoding: identity`

目的是让上游不压缩响应。一旦上游返回 gzip，而我们又在流式转发，
响应头里的 `content-length` 和 `content-encoding` 就无法正确对应，客户端会解析失败。
所以主动要求上游给明文，同时响应侧跳过 `content-length`（流式长度未知）。

### 默认不走系统代理

reqwest 默认读 `HTTP_PROXY` / `HTTPS_PROXY`。上游是 `127.0.0.1` 时，走代理会导致两个问题：

1. 请求被送到代理，而不是本地 cc-switch
2. 请求行从 `POST /v1/messages` 变成 `POST http://127.0.0.1:15722/v1/messages`（absolute-form），部分上游直接拒绝

所以 `use_system_proxy` 默认 false，构建 client 时 `.no_proxy()`。
只有当上游确实需要经代理访问时才打开。

### 请求体上限

`max_body_bytes` 默认 64MiB。`to_bytes` 本身就是带上限的读取，
超限返回 413 而不是把进程拖死。

### 规则返回值即"命中数"

`apply` 返回 `usize`，注册表把它累加成统计。这个数字的作用不是给人看热闹，
而是让你能回答"这条规则到底有没有在生效"。命中数长期为 0，说明规则写错了或者上游已经不需要它。

## 控制面

| 端点 | 返回 |
| --- | --- |
| `GET /_gateway/health` | `{"ok":true,"upstream":"..."}` |
| `GET /_gateway/stats` | 每条规则的 `id`、`label`、`enabled`、累计 `hits` |

两个端点都是只读，不暴露请求内容、不暴露上游凭证。

## 日志

日志分两类，走同一条 `tracing` 管道但用途不同。

**运行日志**：启动、关闭、错误。数量少，看一次就知道进程活着。

**访问日志**：每个转发请求一条，由 `logging::access_log` 中间件产出。

```text
2026-10-24 14:59:34.291  INFO POST /v1/messages status=200 elapsed_ms=4 upstream_ms=4 rewritten=thinking_disable:1,tool_reference:1
```

中间件包在 Router 最外层，所以能测到包含转发在内的完整耗时。转发层把两个信息挂到 response
extensions 上供它读取：

- `RequestTiming { upstream_ms }` — 在 `request.send().await` 返回时立即记录
- `RewriteStats { per_rule }` — 改写阶段的命中汇总

用 extensions 而不是返回值传递，是为了不让 `handle` 的签名为了日志变形。

`upstream_ms` 刻意只测到响应头到达：`send().await` 返回时 body 还没读，所以这个值近似 TTFB。
如果计入流式 body 的传输时长，它就会退化成"整个对话的时长"，失去诊断价值。

控制面请求（`/_gateway/*`）直接返回，不产生访问日志，避免健康检查把自己刷屏。

**流级日志**：由 `stream_observer::ObservedStream` 产出，回答一个访问日志回答不了的问题——
响应流是完整收完的，还是中途断的。

访问日志只记录到响应头返回（`upstream_ms` 是 TTFB），所以上游发完 `message_start`
就把连接掐掉时，访问日志仍然是一条 `status=200`。流级日志补上这一段：

| 情况 | 级别 | 日志 |
| --- | --- | --- |
| 正常收完（见到 `message_stop` / `[DONE]` / `response.completed`） | DEBUG | `流式响应完成 path=... bytes=... chunks=... elapsed_ms=...` |
| 上游正常关闭但没发结束标记 | **WARN** | `上游流提前结束（未见结束标记） ...` |
| 上游连接出错 | **WARN** | `上游流中断（响应未完成） ... error=...` |
| 客户端先断开 | DEBUG | `客户端提前断开，上游流未读完 ...` |

判读方式：客户端报 `Server error mid-response` 时，`grep WARN` 日志——

- 有 `上游流中断` → 上游连接层出问题（重置、解码失败）
- 有 `上游流提前结束` → 上游自己关的流，但没发完，属于上游行为
- 只有 `客户端提前断开` → 客户端主动取消，与上游无关
- 什么都没有 → 请求根本没到网关

正常完成默认是 DEBUG，不改变日常 `info` 级别的日志量；排查时把 `RUST_LOG` 调到
`gateway_core=debug,claude_gateway=debug` 即可看到完整流生命周期。后者是 bin 的
crate 名（由 `[[bin]] name` 推导），不是包名 `gateway-cli`。

## 输出与轮转

| 配置 | 行为 |
| --- | --- |
| `log.dir` 非空 | 终端 + 文件双写；文件按天轮转为 `gateway.YYYY-MM-DD.log`，保留 `retain_days` 天 |
| `log.dir` 留空 | 只写终端 |

`log.dir` 若为相对路径，以**实际加载的那个配置文件所在目录**为基准解析（`Config::base_dir`），
不由进程工作目录决定；支持 `~` 展开，并会规范化掉 `.` 组件。

这条规则的意义在于：把二进制软链成全局命令后，从任何目录启动，日志都落在同一个位置。
如果按进程工作目录解析，日志会散落在每个启动过的目录里，配置也可能悄悄退化成内置默认值。

`logging::init` 会把解析后的实际目录通过 `LogGuard::log_dir` 返回，由 `main` 打进启动日志，
所以启动时一眼就能看到日志到底写到了哪里。

文件层关掉 ANSI 转义，否则日志文件里会混入终端颜色码。终端层保留颜色。

`logging::init` 返回一个 `LogGuard`，必须在 `main` 里活到进程结束。它是后台写盘线程的句柄，
提前 drop 会导致退出时的尾部日志丢失。

## 不记录内容

访问日志只记元数据。请求体和响应体一律不入日志——里面是用户的完整对话内容和代码，
落盘等于把聊天记录和源码副本明文存在磁盘上，没有期限、没有访问控制。

排查需要看具体内容时，手工起一个 echo 上游抓一次即可。这个能力不常驻、不配置化。

## 已知边界

- 只处理 `POST` 带 JSON body 的请求。`GET`、二进制上传等原样透传。
- 改写是同步的、单次遍历。单条请求体很大时（几十 MB）会占住一个 tokio worker 一小段时间，
  对个人使用场景足够。
- 规则之间没有依赖声明，按注册顺序执行。需要顺序保证时，在 `with_default_rules()` 里排好并写注释。
- 不处理 WebSocket / HTTP2 明文升级。
- 上游返回的错误体原样透传，不做错误码映射。要判断"是不是网关自己的问题"，
  看响应体里的 `claude-gateway:` 前缀——那是本项目自己产生的错误。

### 请求侧不是流式，响应侧是

这是两个方向必须分开理解：

**响应方向（下游 ← 网关）是真正的流式。** `Body::from_stream` 直接透传，不缓冲、不重组字节。
实测：一个含中文、注释行、超长单行的 9 事件 SSE，直连上游与经网关的响应体
字节完全相同（3986 字节，sha256 一致），事件按上游设定的 120ms 间隔逐块到达。

**请求方向（客户端 → 网关）会被收拢。** 客户端即使以 `Transfer-Encoding: chunked` 上传，
网关也会先完整读完（`axum::body::to_bytes`），改写后再以带 `content-length` 的普通请求发给上游。
实测上游收到的是 `content-length: 185`、`transfer-encoding: null`。

原因不是实现偷懒，而是 JSON 改写的前提：必须拿到完整文档才能解析。
流式 JSON 无法边读边改，除非引入增量 JSON 解析器，复杂度与收益不成比例。

代价是请求体会完整驻留内存，由 `max_body_bytes` 兜底（默认 64MiB，超限返回 413）。
对 Claude Code 的实际场景没有影响——它的 `/v1/messages` 请求本身就是一次性完整 JSON，
chunked 上传只在手工 curl 或特殊客户端上才会出现。

如果将来确实需要请求侧流式（比如透传大文件上传），正确的做法是**旁路**而非改造：
判断 `content-type` 不是 JSON 时不做改写，直接把 request body 的 stream 接到上游请求上。
