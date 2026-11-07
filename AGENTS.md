# AGENTS.md

本文件是本仓库的工作约定，面向在本仓库内改代码的 AI 助手和协作者。
人类可读的项目介绍见 `README.md`，深入文档见 `docs/`。

## ## 选型收口

一次只让用户拍**一个**决策。不要把同一个选型拆成风格、定位、配色、骨架等多个并行维度
逐项让用户挑选——维度会越开越多，且产品形态未定时配色根本没有判据。
先给依赖关系，把可合并的合并、可给出唯一建议的收拢，最后只留一个能用一句话回答的问题。
**"给一套建议"不等于"替用户决定"**：收口维度是整理，确认权始终在用户；
不得用"我按判据定、不用你挑"把决策权收归实现方。
视觉预览是辅助判断的材料，不是让用户逐张评分的考卷。

## 开工前必读：决策与状态

按顺序读，不要跳过：

1. `AGENTS.md`（本文件）—— 行为约束
2. `docs/decisions.md` —— 已确认 / 待确认 / 已废弃的决策及理由
3. `docs/STATUS.md` —— 当前分支、未提交改动、已验证与未验证、阻塞项、下一步

**决策只看 `decisions.md`。** 聊天记录、某一侧的记忆文件、以及"我记得上次说过"都不算依据。
状态为「待确认」的条目不得据此动手。

## 决策、内容与记忆的实时同步

用户要求：项目相关决策、内容、记忆实时更新到对应本地文档，防止 Agent 与新会话缺失上下文。

触发即更新，不等会话结束、不等用户催：

| 发生了什么 | 立刻更新 |
| --- | --- |
| 用户确认或推翻一个方案 | `docs/decisions.md`（含被否方案与理由）；若改变架构，同步 `docs/architecture.md` |
| 改完代码 / 跑完验证 / 切分支 / 替换线上二进制 | `docs/STATUS.md` |
| 新增或改动兼容规则 | `docs/rules.md` 清单 + `docs/troubleshooting.md` 案例 |
| 出现新故障并定位 | `docs/troubleshooting.md`（只增不删） |
| 当日工作结束 | 工作区日志 `YYYY-MM-DD.md` 追加一条 |

分工不要混：工作区日志记当日流水，属 Agent 侧记忆，**不替代项目文档**；
项目文档记可复用的结论。重要结论两边都要有，但只有项目文档对新会话生效。

写文档的规则：中文、不用 emoji；区分「已验证 / 未验证 / 被阻塞」，未验证项写明原因；
不把未执行的检查写成已通过；不把推测写成用户已确认。

## 视觉类交付

涉及 UI、配色、页面设计的讨论，**必须直接渲染可观看的界面效果预览**：完整窗口、
真实数据、必要时深浅两套主题。不得只用文字、表格、色卡或面积分析代替。
色卡与设计原则是分析材料，不是交付物——用户已多次指出"只给文案看不出长什么样"。

**换配色不等于产品化**。若界面仍是"指标卡 + 请求表 + 原始日志 + 监控/配置侧栏"的
内部遥测骨架，无论换多少套色彩都只是调试台。商业化界面要按用户任务组织第一屏：
价值主张、主行动、功能入口、引导与下一步，而不是系统模块和原始遥测数据。

**数量必须对齐**：文案里列了 N 套候选，效果图就要有 N 套，编号一一对应。
不得压缩数量，也不得用"深浅双主题""同一体系的两个变体"偷换概念——
那是主题维度，不是方案维度。曾出现列了 A/B/C/D 四套、只渲染两张的交付缺漏。

预览属设计示意稿，不是真实渲染截图；真实界面仍以阶段一跑通后的窗口截图验收为准。
视觉验收前不要把"看起来应该可以"写成"已确认"。

## 这个项目在做什么

`claude-gateway` 是一个本地 HTTP 中间层，夹在 Claude Code（可经 cc-switch）与上游模型网关之间。
它只做一件事：**在转发前改写上游网关无法翻译的请求字段**。

不做的事，也不要让它开始做：

- 不做鉴权、不做 provider 管理、不做用量统计、不做故障转移。这些是 cc-switch 的职责。
- 不解析、不缓存、不改写响应体。响应一律原样流式透传。
- 不按端点做特殊处理。`/v1/messages`、`/v1/messages/count_tokens` 等全部走同一条兜底转发。

一旦把上面这些职责吸进来，它就会和 cc-switch 的版本演进互相牵连，失去"可以随时摘掉"的属性。

## 常用命令

```bash
cargo build -p gateway-cli            # 调试构建网关
cargo build -p gateway-cli --release  # 发布构建，产物在 target/release/claude-gateway
cargo test --workspace                # 全部测试
cargo test -p gateway-core --test rewrite_test   # 只跑规则测试
cargo clippy --workspace --all-targets -- -D warnings   # 静态检查，提交前必须干净
cargo fmt                             # 格式化
cargo run -p gateway-cli              # 前台启动，读当前目录 config.toml
CLAUDE_GATEWAY_CONFIG=/path/to/config.toml cargo run -p gateway-cli   # 指定配置（优先级最高）
```

受限环境注意：如果当前环境禁止在仓库内创建 `target`（cargo 建目录时的 rename/delete 会被拦），
用外部 target 目录，不改源码：

```bash
CARGO_TARGET_DIR=/tmp/cg-target cargo test
```

不要为了绕开这个限制去改 `Cargo.toml` 或加 `.cargo/config.toml`，那是环境问题不是项目问题。

默认不要主动执行下面这类动作，先问：

- 起常驻服务（launchd / systemd / pm2）
- 改 cc-switch 的 provider 配置或代理端口
- 改 `~/.claude/settings.json`
- 向真实上游发请求做联调（会产生真实计费）

## 目录职责

```
crates/
├── gateway-core/         lib：全部业务实现，不依赖任何 GUI 框架
│   ├── src/
│   │   ├── config.rs         config.toml → 结构体；规则开关查询在这里
│   │   ├── logging.rs        日志初始化 + 访问日志中间件 + 计时/命中信息载体
│   │   ├── server.rs         HTTP 入口：控制面路由 + 兜底转发
│   │   ├── forward.rs        请求体改写 + 上游转发 + 流式回传（核心）
│   │   ├── stream_observer.rs 包裹响应流：统计字节/块数，判断是否正常结束
│   │   ├── telemetry.rs      内存环形缓冲：请求、流事件、日志尾部
│   │   ├── panel.rs          只读控制台 /_panel 与它的 6 个 JSON 接口
│   │   └── rewrite/
│   │       ├── mod.rs        RewriteRule trait、Registry、命中统计
│   │       ├── thinking_disable.rs
│   │       └── tool_reference.rs
│   ├── tests/                规则与控制台路由的集成测试
│   └── ui/panel.html         控制台页面，编译期内嵌进二进制
├── gateway-cli/          bin：入口装配，只做读配置、建 client、起服务、处理信号
│   └── src/main.rs
└── gateway-app/          bin：Tauri 桌面端，网关内嵌在同一进程
    ├── src/
    │   ├── main.rs            Tauri 装配：注册命令、起服务、托盘、关窗改为隐藏
    │   ├── ctx.rs             启动上下文：先同步 bind 端口，失败不 panic 而是显示在界面上
    │   ├── commands.rs        前端 IPC 命令，全部转调 gateway_core::panel 的快照函数
    │   └── tray.rs            托盘图标与菜单，退出入口只在这里
    ├── ui/index.html          界面，编译期内嵌
    └── icons/                 应用图标（PNG）与托盘图标（原始 RGBA）
docs/
```

`gateway-core` 不认识任何 GUI 框架，这条边界是刻意的：core 必须始终能被测试、
能被 headless 复用。桌面端只是并列的另一个入口 crate。

**控制台数据只有一份实现**：`gateway_core::panel` 里的 `*_snapshot()` 是唯一取数入口，
HTTP 路由（`/_panel/api/*`）和 Tauri IPC 命令都调它。新增一个界面字段时改快照函数，
两条出口同时生效；不要在 `commands.rs` 里另写一份取数逻辑。

## 前端

`crates/gateway-app/ui` 是 React 19 + Vite 8 + TypeScript 6 的单页界面，包管理器固定 **pnpm**
（`packageManager: pnpm@12.15.0`，只提交 `pnpm-lock.yaml`）。安装、门禁、打包都用 pnpm：

```bash
cd crates/gateway-app/ui
pnpm install --frozen-lockfile
pnpm run lint && pnpm run test && pnpm run build
cd ..
./ui/node_modules/.bin/tauri build
```

组件库是 **Ant Design 6** + `@ant-design/icons`。主题在 `ui/src/App.tsx` 的 `ConfigProvider`
里把 `lib/tokens.ts` 的暖米白 + 赤陶色值映射成 antd token；`styles/base.css` 只放布局骨架
与共用原子类，`styles/app.css` 放页面间距与日志显示区。不要在组件里写死颜色，也不要再手搓
按钮、表格、输入框等 antd 已有的控件。功能图标统一用 `@ant-design/icons`；品牌标记是 SVG，
不属于功能图标。

改代码前先判断落在哪个格子（路径相对 `crates/gateway-core/src/`）：

- 新的字段不兼容 → `rewrite/` 新增文件，不要动 `forward.rs`
- 转发行为（header、超时、流式）→ `forward.rs`
- 流的完成判定、SSE 结束标记 → `stream_observer.rs`
- 配置项 → `config.rs` + `config.toml` + 文档
- 路由与端点 → `server.rs`
- 日志格式、级别、落盘 → `logging.rs`
- 控制台展示什么 → `telemetry.rs`（取数）+ `panel.rs`（出口）

## 日志

初始化在 `logging.rs`，级别取 `RUST_LOG`（优先）或 `config.log.level`。时间格式固定为本地时间
`YYYY-MM-DD HH:mm:ss.SSS`。日志始终落盘，按本地日期轮转为 `gateway.YYYY-MM-DD.log`；
`config.log.dir` 留空用平台默认目录（macOS 为 `~/Library/Logs/claude-gateway/`），
`retain_days` 按文件个数清理旧日志，不是按自然日过期。

级别约定，新增代码照此执行：

| 场景 | 级别 | 说明 |
| --- | --- | --- |
| 每个转发请求一条访问日志 | `info` | 由 `logging::access_log` 中间件统一产生，业务代码不要重复打 |
| 启动、关闭、控制面地址 | `info` | 只在 `main.rs` |
| 上游请求失败（502） | `warn` | 带 url 和 error |
| 请求体超限（413） | `warn` | 带 limit 和 path |
| 改写后序列化失败 | `warn` | 带 error，回退原始请求体 |
| 上游流提前结束（无结束标记） | `warn` | 由 `stream_observer` 产出，带 bytes/chunks/elapsed_ms |
| 上游流中断（连接出错） | `warn` | 同上，额外带 error |
| 流式响应正常完成 | `debug` | 带 bytes/chunks/elapsed_ms，不占日常 info 量 |
| 客户端提前断开 | `debug` | 与上游无关，不要报警 |
| 规则命中详情 | `debug` | 访问日志里已有汇总，逐条详情只在排查时用 |
| 请求体非 JSON 被跳过 | `debug` | 属于正常情况，不该污染 info |

**绝不记录请求体或响应体。** 里面是用户的完整对话内容和代码，写进日志等于把聊天记录和源码
副本明文落盘、无期限保留。需要看具体内容时，手工起一个 echo 上游抓一次，不要做成常驻能力，
也不要加 `log_bodies` 之类的开关——开关早晚会被打开然后忘记关。

访问日志的三个数字含义不要搞混：

- `elapsed_ms` — 网关视角的总耗时，从收到请求到响应头构造完成
- `upstream_ms` — 到上游**首字节**的时间。流式 body 的传输时长不计入，否则这个数会变成
  整个对话的时长，失去参考价值
- `rewritten` — 本次命中的规则与次数，无命中为 `-`。数字长期为 0 说明规则没生效

用 launchd 托管时不需要在 `config.log.dir` 里写绝对路径：相对路径以配置文件所在目录为基准解析。
只要配置文件本身能被找到（放 `~/.config/claude-gateway/config.toml`，或在 plist 里用绝对路径设
`CLAUDE_GATEWAY_CONFIG`）即可。**不要**再把相对路径按进程工作目录处理——那是改掉的老行为，
launchd 下工作目录是 `/`，会把日志写到 `/logs`。

## 配置与路径解析

配置文件查找顺序（`Config::discover`），命中第一个就用：

1. `CLAUDE_GATEWAY_CONFIG` 环境变量
2. 当前工作目录的 `config.toml`
3. 可执行文件真实所在目录的 `config.toml`
4. `~/.config/claude-gateway/config.toml`

第 3 步用了两次路径解析：`current_exe()` 在 macOS 上**不解析软链**，所以还要
`canonicalize` 一次，否则 `ln -s` 到 `/usr/local/bin` 之后会去 `/usr/local/bin/` 找配置。
这个坑实际踩过，改动这段代码时不要省掉。

所有相对路径（目前是 `log.dir`）都以**实际加载的那个配置文件所在目录**为基准，通过
`config.rs::resolve_path` 统一解析，支持 `~` 展开，并会去掉 `.` 组件。
`Config::base_dir` 就是解析基准，`Config::source` 记录实际加载的文件（`None` 表示用了内置默认）。

新增配置项只要涉及路径，一律走 `resolve_path`，不要再直接用 `Path::new(raw)`——
那会退回到依赖进程工作目录的老行为。

## 代码规则

- 规则实现必须是无副作用的纯函数式改写：只接收 `&mut Value`，返回命中次数。不要在规则里读配置、发请求、写文件。
- 规则必须**幂等**。同一份 body 连续过两遍，结果必须一致。否则重试逻辑会产生意外行为。
- 规则必须**只碰自己认识的结构**。找不到目标字段时返回 0，绝不能顺手"规范化"其他内容。
- 改写失败要能降级。`rewrite_body` 的既有行为是：非 JSON 或解析失败时原样透传。新增逻辑不要破坏这条兜底。
- 日志按上面「日志」一节的级别约定执行。生产代码不留 `println!`、`dbg!`。
- 注释和文档用中文，不用 emoji。
- 加依赖要克制。这个项目的价值之一是启动快、依赖少，不要为了省几行代码引入重型 crate。

## 加一条新规则

标准流程，照着做：

1. 在 `crates/gateway-core/src/rewrite/` 新建文件，实现 `RewriteRule`：

```rust
pub struct MyRule;

impl RewriteRule for MyRule {
    fn id(&self) -> &'static str { "my_rule" }
    fn label(&self) -> &'static str { "一句话说明这条规则在干什么" }
    fn apply(&self, body: &mut Value) -> usize { /* 返回命中次数，0 表示没动 */ }
}
```

2. 在 `crates/gateway-core/src/rewrite/mod.rs` 的 `with_default_rules()` 里注册一行。
3. 在 `config.toml` 加开关段落（不加也行，未配置默认视为启用）。
4. 在 `crates/gateway-core/tests/rewrite_test.rs` 补至少三个用例：命中、不命中、边界保留。
5. 在 `docs/rules.md` 的规则清单表里加一行。
6. 更新 `docs/troubleshooting.md`，写清这条规则对应哪类上游报错。

不要写"万能规则"（比如遍历所有字段删掉不认识的键）。上游可能新增合法字段，
一刀切会把正常请求改坏，而且很难排查为什么模型行为变了。

## 测试要求

- 每条规则至少覆盖：命中、不命中、相邻情况不受影响。
- 注册表测试要验证开关关闭时 body 一个字节都不变。
- 改动 `forward.rs` 的转发逻辑后，除了单测还要做一次本地回环验证
  （起一个 echo 上游，确认改写结果、请求行是相对路径、SSE 逐块到达）。
- 不要在测试里连真实上游或用真实密钥。

## 已知环境坑

**`HTTP_PROXY` 会把本地请求劫持走。** reqwest 默认读环境变量里的代理。
上游是 `127.0.0.1:15721` 时，走代理会导致请求被错误路由，请求行变成 absolute-form URI，
部分上游直接拒绝。所以 `use_system_proxy` 默认 false，client 构建时走 `.no_proxy()`。
不要为了"支持代理"把这个默认值改掉。

**CARGO_TARGET_DIR。** 见"常用命令"，只在受限环境用，不要写进仓库配置。

**cc-switch 会重写 `~/.claude/settings.json`。** 它按 `proxy_config.listen_port` 拼 `ANTHROPIC_BASE_URL`。
所以不要靠手改这个文件来接入本项目——切换 provider 时会被覆盖，而且失效时没有任何提示。
正确的接入点是 cc-switch 里的 provider API 地址。

## 历史背景

- 项目的起因是两个具体故障：Claude Code 2.1.307 的 ToolSearch 返回 `tool_reference` 内容块，
  上游 token 计数器不认，返回 500 且因为历史重放导致会话永久卡死；
  同一版本 auto 权限模式的分类器请求带 `thinking:{type:"disabled"}`，上游未配
  `model_info.thinking_disable_mode`，返回 400。
- 两条规则降级后的语义与原字段等价，不是"丢弃信息"：`tool_reference` 原本只表示"以下工具已加载"，
  `thinking=disabled` 删除后等价于"不声明 thinking"。
- 曾考虑过在 cc-switch 源码里加开关（`normalize_thinking_type` 目前是空操作，是天然的插入点），
  但那样要长期维护 fork。独立项目的取舍是：多一个进程，换取规则可以无限加、且不与上游版本绑死。
