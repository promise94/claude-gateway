//! 日志初始化与访问日志中间件
//!
//! 三件事：
//! 1. 把 `tracing` 的初始化从 `main.rs` 收进来，统一时间格式与输出目标
//! 2. 为每个转发的请求产出一条访问日志，含请求 ID、耗时、上游耗时、改写命中
//! 3. 允许运行期切换过滤级别：内存立即生效，界面入口会同步写回配置文件
//!
//! 请求 ID 是 6 位十六进制短串，由访问日志中间件生成并放进 request extensions。
//! 转发层把它带进流事件，于是一条 `rid=` 能把「访问日志 / 流异常 / 控制台请求行」
//! 三处对上号——排查 `Server error mid-response` 时不用再靠时间戳猜。
//!
//! 明确不做的事：**不记录请求体和响应体**。里面是用户的完整对话内容和代码，
//! 落盘等于把聊天记录和源码副本明文存在磁盘上，而且没有期限。

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter, Registry};

use crate::config::Config;
use crate::telemetry::{self, RequestRecord, TeeWriter, Telemetry};
use std::sync::Arc;

/// 本地时间，格式 `YYYY-MM-DD HH:mm:ss.SSS`
///
/// 与常见运维日志对齐，便于直接 grep 时间段。
struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(
            w,
            "{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
        )
    }
}

/// 请求 ID，由访问日志中间件生成，转发层与流观测共用。
///
/// 只用于把同一次请求的几条日志串起来，不参与上游语义，也不发给上游。
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 生成 6 位十六进制短 ID。
///
/// 回绕上限 1600 万次请求，远大于单进程环形缓冲（200 条）的对照范围；
/// 比 UUID 短，肉眼对日志时不容易错行。
pub fn next_request_id() -> String {
    let value = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{:06x}", value & 0xff_ffff)
}

/// 上游耗时，由转发层写入 response extensions
#[derive(Debug, Clone, Copy)]
pub struct RequestTiming {
    pub upstream_ms: u64,
}

/// 本次请求的改写命中情况，由转发层写入 response extensions
#[derive(Debug, Clone)]
pub struct RewriteStats {
    pub per_rule: Vec<(&'static str, usize)>,
}

impl RewriteStats {
    /// 渲染成 `thinking_disable:1,tool_reference:2`，无命中时为 `-`
    pub fn describe(&self) -> String {
        if self.per_rule.is_empty() {
            return "-".to_string();
        }
        self.per_rule
            .iter()
            .map(|(id, hits)| format!("{id}:{hits}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// 持有后台写盘线程。必须在 main 里存活到进程结束，否则日志会丢尾部。
pub struct LogGuard {
    _file: Option<tracing_appender::non_blocking::WorkerGuard>,
    /// 实际落盘目录
    pub log_dir: std::path::PathBuf,
}

/// 可热替换的过滤器句柄。`Registry` 是过滤器所附着的 subscriber 类型。
type LogReload = reload::Handle<EnvFilter, Registry>;

static RELOAD: OnceLock<LogReload> = OnceLock::new();

#[derive(Clone)]
struct LevelState {
    level: String,
    /// config（来自 config.toml）| env（来自 RUST_LOG）| runtime（运行期切换过）
    source: &'static str,
}

static LEVEL: OnceLock<Mutex<LevelState>> = OnceLock::new();

/// 可选级别，顺序即严重程度递增
pub const LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];

/// 当前级别对外快照，面板与 IPC 共用
#[derive(Clone, serde::Serialize)]
pub struct LogLevelState {
    pub level: String,
    pub source: String,
}

/// 级别名归一化。只接受固定几档，避免把拼错的值塞进 `EnvFilter` 后静默失效。
pub fn normalize_level(raw: &str) -> Option<&'static str> {
    let lowered = raw.trim().to_ascii_lowercase();
    LEVELS.iter().copied().find(|level| *level == lowered)
}

/// 运行期切换过滤级别。
///
/// 本函数只改当前进程的过滤器，不碰配置文件；是否写回由调用方决定：
/// 面板入口 `panel::set_log_level_snapshot` 会同步写回 `config.toml`，
/// 写不进时返回 `persisted=false`。CLI 直接调用则只在本次进程生效。
pub fn set_level(raw: &str) -> anyhow::Result<LogLevelState> {
    let level = normalize_level(raw)
        .ok_or_else(|| anyhow::anyhow!("未知日志级别 {raw}，可选：{}", LEVELS.join(" / ")))?;

    // 点当前级别不该产生副作用：否则界面会把「没变」显示成「运行期切换过」
    let current = current_level();
    if current.level == level {
        return Ok(current);
    }

    let handle = RELOAD
        .get()
        .ok_or_else(|| anyhow::anyhow!("日志系统尚未初始化，无法切换级别"))?;
    handle
        .reload(EnvFilter::new(default_directive(level)))
        .map_err(|error| anyhow::anyhow!("切换日志级别失败：{error}"))?;

    if let Some(state) = LEVEL.get() {
        let mut state = state.lock().expect("日志级别锁被投毒");
        state.level = level.to_string();
        state.source = "runtime";
    }

    // 切到 error 时这条 info 自己会被挡掉，属预期
    tracing::info!(level, "日志级别已切换，仅当前进程生效");
    Ok(current_level())
}

pub fn current_level() -> LogLevelState {
    LEVEL
        .get()
        .map(|state| {
            let state = state.lock().expect("日志级别锁被投毒");
            LogLevelState {
                level: state.level.clone(),
                source: state.source.to_string(),
            }
        })
        .unwrap_or_else(|| LogLevelState {
            level: "info".to_string(),
            source: "config".to_string(),
        })
}

/// 本仓库自己的日志 target 列表。
///
/// EnvFilter 不支持通配目标，只能逐个列。三处容易踩的：
///
/// 1. `gateway_core` 是 lib 的 crate 名，库内 `tracing::info!` 的 target 就是它。
/// 2. `claude_gateway` 是 CLI 的 **bin 名**（`[[bin]] name = "claude-gateway"`）推导出的
///    crate 名，不是包名 `gateway-cli`。
/// 3. `gateway_app` 是 Tauri 桌面端的 crate 名（包名 `gateway-app`），
///    `ctx.rs` / `main.rs` 的启动日志 target 是它。
///
/// 写错的表现是"某些日志凭空消失"：全局 `warn` 会把没列进来的 target 的 info 全挡掉。
const OWN_CRATES: &[&str] = &["gateway_core", "claude_gateway", "gateway_app"];

/// 默认过滤指令：自己的 crate 用配置级别，其余（第三方依赖）压到 warn。
pub fn default_directive(level: &str) -> String {
    let directives: Vec<String> = OWN_CRATES
        .iter()
        .map(|name| format!("{name}={level}"))
        .collect();
    format!("warn,{}", directives.join(","))
}

/// 初始化日志。
///
/// 级别优先级：`RUST_LOG` 环境变量 > `config.log.level`。
/// 日志始终落盘：`log.dir` 为空时用平台默认目录，相对路径以配置文件所在目录为基准解析。
pub fn init(config: &Config, telemetry: Arc<Telemetry>) -> anyhow::Result<LogGuard> {
    // 环境变量优先级最高：临时排查时 `RUST_LOG=debug` 比改配置快。
    // 记下来源，面板上要能说清「这个级别是谁定的」。
    let (filter, from_env) = match EnvFilter::try_from_default_env() {
        Ok(filter) => (filter, true),
        Err(_) => (EnvFilter::new(default_directive(&config.log.level)), false),
    };
    let initial = if from_env {
        LevelState {
            level: config.log.level.clone(),
            source: "env",
        }
    } else {
        LevelState {
            level: config.log.level.clone(),
            source: "config",
        }
    };
    let _ = LEVEL.set(Mutex::new(initial));

    // 包一层 reload，运行期可整体换过滤器；转发路径不读这把锁
    let (filter_layer, reload_handle) = reload::Layer::new(filter);
    let _ = RELOAD.set(reload_handle);

    // 终端层：写 stdout，同时把每行剥掉颜色后塞进环形缓冲，供 /_panel 读取。
    // 合成一个 writer 而不是叠两个 tracing 层：多个 fmt 层叠在 registry 上
    // 会让类型推断解不开，而且格式化两次纯属浪费。
    let console_layer = tracing_subscriber::fmt::layer()
        .with_timer(LocalTimer)
        .with_target(false)
        .with_writer(TeeWriter::new(telemetry));

    // 日志固定落盘：目录留空时用平台默认目录，不再有「只输出终端」的模式
    let dir = config.log.resolve_dir(&config.base_dir);
    std::fs::create_dir_all(&dir)?;

    // 按本地日期切分。不能用 tracing_appender 的 DAILY：它按 UTC 命名，
    // 北京时间下 0:00–8:00 的日志会写进前一天的文件。
    let appender = crate::local_appender::LocalDailyAppender::new(
        &dir,
        "gateway",
        "log",
        config.log.retain_days,
    )?;

    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    // 文件层：不带 ANSI 转义，否则日志里会有颜色码
    let file_layer = tracing_subscriber::fmt::layer()
        .with_timer(LocalTimer)
        .with_target(false)
        .with_ansi(false)
        .with_writer(non_blocking);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(console_layer)
        .with(file_layer)
        .init();

    Ok(LogGuard {
        _file: Some(guard),
        log_dir: dir,
    })
}

/// 访问日志中间件：每个转发请求一条
///
/// 输出示例：
/// ```text
/// 2026-10-24 14:56:09.123 INFO POST /v1/messages status=200 elapsed_ms=1832 upstream_ms=1810 rewritten=thinking_disable:1
/// ```
///
/// 控制面（`/_gateway/*`）和面板（`/_panel/*`）不产生访问日志：
/// 前者避免被自己的健康检查刷屏，后者否则每 2 秒两次轮询会把
/// 环形缓冲里的真实日志挤掉。
pub async fn access_log(mut request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let is_internal = path.starts_with("/_gateway/") || path.starts_with("/_panel");

    // 内部端点不生成 ID：它们本来就不进日志，也没人拿它对照。
    // 在转发之前插入 extensions，转发层才能把同一个 ID 带进流事件。
    let rid = if is_internal {
        None
    } else {
        let rid = next_request_id();
        request.extensions_mut().insert(RequestId(rid.clone()));
        Some(rid)
    };

    let started = Instant::now();
    let response = next.run(request).await;

    if is_internal {
        return response;
    }
    let rid = rid.unwrap_or_else(|| "-".to_string());

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    // 上游没连上时不会有 timing，用 0 表示"没走到上游"
    let upstream_ms = response
        .extensions()
        .get::<RequestTiming>()
        .map(|timing| timing.upstream_ms)
        .unwrap_or(0);
    let rewritten = response
        .extensions()
        .get::<RewriteStats>()
        .map(RewriteStats::describe)
        .unwrap_or_else(|| "-".to_string());

    // 不使用自定义 target：EnvFilter 按模块路径前缀匹配，
    // 自定义 target 会绕过 `gateway_core=info` 这条规则被全局级别过滤掉。
    tracing::info!(
        rid = %rid,
        status,
        elapsed_ms,
        upstream_ms,
        rewritten = %rewritten,
        "{method} {path}"
    );

    telemetry::submit_request(RequestRecord {
        at: telemetry::now_local(),
        rid,
        method: method.to_string(),
        path,
        status,
        elapsed_ms,
        upstream_ms,
        rewritten,
    });

    response
}

#[cfg(test)]
mod tests {
    use super::{
        current_level, default_directive, next_request_id, normalize_level, set_level, OWN_CRATES,
    };

    #[test]
    fn normalize_level_accepts_known_levels_only() {
        assert_eq!(normalize_level("INFO"), Some("info"));
        assert_eq!(normalize_level(" debug "), Some("debug"));
        assert_eq!(normalize_level("verbose"), None);
        assert_eq!(normalize_level(""), None);
    }

    #[test]
    fn request_ids_are_unique_and_short() {
        let first = next_request_id();
        let second = next_request_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 6);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    /// 界面允许重复点同一个级别，不能每次都标成「运行期切换」
    #[test]
    fn setting_the_current_level_is_a_noop() {
        let before = current_level();
        let after = set_level(&before.level).expect("切到当前级别不应失败");
        assert_eq!(after.level, before.level);
        assert_eq!(after.source, before.source);
    }

    #[test]
    fn default_directive_covers_every_own_crate() {
        let directive = default_directive("debug");
        assert!(directive.starts_with("warn,"), "第三方依赖应压到 warn");
        for name in OWN_CRATES {
            assert!(
                directive.contains(&format!("{name}=debug")),
                "{name} 未出现在指令里：{directive}"
            );
        }
    }

    /// 拆分 workspace 时踩过：列表里漏了 bin 的 crate 名，导致 main.rs 的启动日志
    /// 被全局 warn 挡掉，表现为"进程起来了但一行日志都没有"。
    #[test]
    fn default_directive_covers_bin_crate_not_package_name() {
        let directive = default_directive("info");
        assert!(
            directive.contains("claude_gateway=info"),
            "bin 的 crate 名（由 bin 名推导）必须在内：{directive}"
        );
        assert!(
            !directive.contains("gateway_cli"),
            "包名不是日志 target，写进指令没有意义：{directive}"
        );
    }

    /// 桌面端曾漏列：进程起来了、转发也正常，但启动日志一行都没有，
    /// 当天日志文件也不生成，容易误判成日志系统坏了。
    #[test]
    fn default_directive_covers_tauri_app_crate() {
        let directive = default_directive("info");
        assert!(
            directive.contains("gateway_app=info"),
            "桌面端 crate 名必须在内：{directive}"
        );
    }
}
