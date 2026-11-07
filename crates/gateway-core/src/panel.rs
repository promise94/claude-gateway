//! 控制台：`/_panel`
//!
//! 展示运行状态、规则命中、最近请求、流事件、内存日志、落盘日志、生效配置。
//!
//! 写操作只放开规则开关与参数：改完立即生效，保存时用 `toml_edit` 就地写回。
//! 其余配置仍然只读——端口、上游、日志级别改了本来也无法热生效，
//! 混在一起只会让「什么能热改」变模糊。
//!
//! 页面本身是 `ui/panel.html`，编译期内嵌进二进制，不依赖 CDN 或外部文件。

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::RuleConfig;
use crate::logging;
use crate::server::AppState;
use crate::RuleStat;

/// 单次最多返回的内存日志行数，和环形缓冲容量对齐
const MAX_LOG_LINES: usize = 1000;
/// 落盘文件单页最多返回的行数
const MAX_LOG_PAGE: usize = 2000;
/// 导出上限。前端只导当前视图，正常远小于这个数
const MAX_EXPORT_LINES: usize = 20_000;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/_panel", get(index))
        .route("/_panel/", get(index))
        .route("/_panel/api/status", get(status))
        .route("/_panel/api/logs", get(logs))
        .route("/_panel/api/files", get(files))
        .route("/_panel/api/logfile", get(logfile))
        .route("/_panel/api/logs/export", post(export_logs))
        .route("/_panel/api/log-level", get(log_level).post(set_log_level))
        .route("/_panel/api/config", get(config))
        .route("/_panel/api/upstream", get(upstream).post(set_upstream))
        // 规则开关与参数：改内存立即生效，保存才落盘
        .route("/_panel/api/rules", post(update_rule))
        .route("/_panel/api/rules/save", post(save_rules))
        .route("/_panel/api/rules/discard", post(discard_rules))
}

async fn index() -> impl IntoResponse {
    Html(include_str!("../ui/panel.html"))
}

/// 运行状态快照。
///
/// 抽成普通函数是为了让 HTTP 出口（`/_panel/api/status`）和桌面端的 Tauri command
/// 共用同一份实现——两条出口各写一遍必然会漂移。
pub fn status_snapshot(state: &AppState) -> Value {
    let telemetry = &state.telemetry;
    let rules: Vec<Value> = state.gateway.rule_stats().iter().map(rule_json).collect();

    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "uptime_secs": telemetry.uptime_secs(),
        "started_at": telemetry.started_wall()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "listen": state.listen,
        "upstream": state.gateway.upstream(),
        "config_source": telemetry.config_source(),
        "log_dir": telemetry.log_dir(),
        "log_buffered": telemetry.log_len(),
        "counters": telemetry.counters(),
        "rules": rules,
        "rules_dirty": state.gateway.rules_dirty(),
        "requests": telemetry.recent_requests(30),
        "streams": telemetry.recent_streams(20),
    })
}

/// 单条规则的对外结构：参数同时给出声明（label/default/占位符）与当前值，
/// 前端据此渲染表单，不需要在前端硬编码参数名。
///
/// 公开给桌面端的 Tauri command 复用，保证 HTTP 与 IPC 两条出口不会漂移。
pub fn rule_json(stat: &RuleStat) -> Value {
    let params: Vec<Value> = stat
        .param_specs
        .iter()
        .map(|spec| {
            json!({
                "key": spec.key,
                "label": spec.label,
                "default": spec.default,
                "required_placeholder": spec.required_placeholder,
                "value": stat
                    .params
                    .get(spec.key)
                    .cloned()
                    .unwrap_or_else(|| spec.default.to_string()),
            })
        })
        .collect();

    json!({
        "id": stat.id,
        "label": stat.label,
        "description": stat.description,
        "enabled": stat.enabled,
        "hits": stat.hits,
        "params": params,
    })
}

#[derive(Deserialize)]
struct RuleUpdate {
    id: String,
    enabled: bool,
    #[serde(default)]
    params: BTreeMap<String, String>,
}

/// 改一条规则的开关与参数。只写内存，响应里带 `dirty` 表示尚未落盘。
async fn update_rule(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RuleUpdate>,
) -> Response {
    let config = RuleConfig {
        enabled: payload.enabled,
        params: payload.params,
    };

    match update_rule_and_persist(&state, &payload.id, config) {
        Ok(outcome) => Json(rule_update_json(&state, &outcome)).into_response(),
        Err(error) => fail(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

/// 规则改动的结果：是否已写进配置文件，写失败时带上原因
pub struct RuleUpdateOutcome {
    pub stat: RuleStat,
    pub persisted: bool,
    pub save_error: Option<String>,
}

/// 改规则并立即落盘。
///
/// 界面上已经没有「保存」按钮，内存改了就必须写进文件；写失败不回滚内存，
/// 但把原因带给界面，由用户决定重试还是手工改配置。
pub fn update_rule_and_persist(
    state: &AppState,
    id: &str,
    config: RuleConfig,
) -> anyhow::Result<RuleUpdateOutcome> {
    let stat = state.gateway.update_rule(id, config)?;
    let mut outcome = RuleUpdateOutcome {
        stat,
        persisted: false,
        save_error: None,
    };

    match state.gateway.save_rules() {
        Ok(_) => outcome.persisted = true,
        Err(error) => {
            tracing::warn!(%error, rule = %id, "规则已生效，但写入配置文件失败");
            outcome.save_error = Some(error.to_string());
        }
    }

    Ok(outcome)
}

pub fn rule_update_json(state: &AppState, outcome: &RuleUpdateOutcome) -> Value {
    json!({
        "rule": rule_json(&outcome.stat),
        "dirty": state.gateway.rules_dirty(),
        "persisted": outcome.persisted,
        "save_error": outcome.save_error,
    })
}

/// 落盘。写前备份为 config.toml.bak，写入失败时内存态保持不变。
async fn save_rules(State(state): State<Arc<AppState>>) -> Response {
    match state.gateway.save_rules() {
        Ok(path) => Json(json!({
            "saved": true,
            "path": path.display().to_string(),
            "dirty": state.gateway.rules_dirty(),
        }))
        .into_response(),
        Err(error) => fail(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

/// 丢弃未落盘改动，从配置文件重新读取
async fn discard_rules(State(state): State<Arc<AppState>>) -> Response {
    match state.gateway.discard_rules() {
        Ok(()) => Json(json!({
            "dirty": state.gateway.rules_dirty(),
            "rules": state.gateway.rule_stats().iter().map(rule_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => fail(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(status_snapshot(&state))
}

#[derive(Deserialize)]
struct LogQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    level: Option<String>,
}

/// 日志快照，供 HTTP 与 IPC 共用
pub fn logs_snapshot(state: &AppState, limit: usize, level: Option<&str>) -> Value {
    let limit = limit.clamp(1, MAX_LOG_LINES);
    let level = level.filter(|value| !value.is_empty() && *value != "all");
    json!({
        "lines": state.telemetry.recent_logs(limit, level),
        "buffered": state.telemetry.log_len(),
        "source": "memory",
    })
}

async fn logs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LogQuery>,
) -> impl IntoResponse {
    Json(logs_snapshot(
        &state,
        query.limit.unwrap_or(300),
        query.level.as_deref(),
    ))
}

/// 落盘日志文件清单，供 HTTP 与 IPC 共用
pub fn files_snapshot(state: &AppState) -> Value {
    let Some(dir) = log_dir(state) else {
        return json!({ "dir": Value::Null, "files": [] });
    };

    let mut entries: Vec<(String, u64, u64)> = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !is_log_name(&name) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            entries.push((name, meta.len(), modified));
        }
    }
    // 文件名里带日期，倒序即最新在前
    entries.sort_by(|a, b| b.0.cmp(&a.0));

    let files: Vec<Value> = entries
        .into_iter()
        .map(
            |(name, bytes, modified)| json!({ "name": name, "bytes": bytes, "modified": modified }),
        )
        .collect();

    json!({ "dir": dir.display().to_string(), "files": files })
}

async fn files(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(files_snapshot(&state))
}

#[derive(Deserialize)]
struct FileQuery {
    name: String,
    /// 从这个行号（不含）往前取，用于向上翻页；缺省表示取到文件末尾
    #[serde(default)]
    before: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

/// 一页落盘日志
pub struct LogPage {
    pub lines: Vec<String>,
    pub total: usize,
    pub start: usize,
    pub end: usize,
    pub bytes: u64,
}

impl LogPage {
    pub fn has_more(&self) -> bool {
        self.start > 0
    }
}

/// 计算分页窗口。`before` 是「不含」的上界，缺省取文件末尾。
///
/// 单独抽出来是为了能用纯函数测边界：空文件、before 超过总数、limit 超上限。
pub fn page_bounds(total: usize, before: Option<usize>, limit: usize) -> (usize, usize) {
    let limit = limit.clamp(1, MAX_LOG_PAGE);
    let end = before.map_or(total, |before| before.min(total));
    let start = end.saturating_sub(limit);
    (start, end)
}

/// 逐行数总行数。最后一行没有换行符也算一行。
fn count_lines(path: &Path) -> std::io::Result<usize> {
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut buffer = Vec::new();
    let mut total = 0;
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }
        total += 1;
    }
    Ok(total)
}

/// 读取一页日志。
///
/// 两遍顺序读：先数总行数拿到窗口，再跳到窗口取行。内存占用与文件大小无关，
/// 只有 O(limit)，所以当日志涨到几百 MB 也不会把面板拖垮。
pub fn read_log_page(
    dir: &Path,
    name: &str,
    before: Option<usize>,
    limit: usize,
) -> anyhow::Result<LogPage> {
    let path = dir.join(name);
    let bytes = std::fs::metadata(&path)?.len();
    let total = count_lines(&path)?;
    let (start, end) = page_bounds(total, before, limit);

    let mut reader = std::io::BufReader::new(std::fs::File::open(&path)?);
    let mut buffer = Vec::new();
    let mut index = 0;
    let mut lines = Vec::with_capacity(end - start);
    while index < end {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }
        if index >= start {
            let mut line = String::from_utf8_lossy(&buffer).into_owned();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            lines.push(line);
        }
        index += 1;
    }

    Ok(LogPage {
        lines,
        total,
        start,
        end,
        bytes,
    })
}

pub fn logfile_snapshot(
    state: &AppState,
    name: &str,
    before: Option<usize>,
    limit: usize,
) -> Result<Value, (StatusCode, String)> {
    let Some(dir) = log_dir(state) else {
        return Err((
            StatusCode::NOT_FOUND,
            "日志目录尚未就绪，无法读取日志文件".to_string(),
        ));
    };
    if !is_log_name(name) {
        return Err((
            StatusCode::BAD_REQUEST,
            "文件名不合法，只允许 gateway.YYYY-MM-DD.log".to_string(),
        ));
    }

    let limit = limit.clamp(1, MAX_LOG_PAGE);
    let page = read_log_page(&dir, name, before, limit)
        .map_err(|error| (StatusCode::NOT_FOUND, format!("读取 {name} 失败：{error}")))?;

    Ok(json!({
        "name": name,
        "lines": page.lines,
        "total": page.total,
        "start": page.start,
        "end": page.end,
        "has_more": page.has_more(),
        "bytes": page.bytes,
        "source": "file",
    }))
}

async fn logfile(State(state): State<Arc<AppState>>, Query(query): Query<FileQuery>) -> Response {
    let limit = query.limit.unwrap_or(400);
    // 数行与取行都是阻塞 IO，放到阻塞线程池，别占 tokio worker
    let snapshot = tokio::task::spawn_blocking({
        let state = state.clone();
        let name = query.name.clone();
        move || logfile_snapshot(&state, &name, query.before, limit)
    })
    .await;

    match snapshot {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err((status, message))) => fail(status, &message),
        Err(error) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("读取日志任务失败：{error}"),
        ),
    }
}

#[derive(Deserialize)]
struct ExportBody {
    lines: Vec<String>,
    #[serde(default)]
    label: Option<String>,
}

/// 把界面当前显示的行导出成一个文件，返回绝对路径。
///
/// 不直接复用落盘文件：内存模式没有对应文件，文件模式也可能只看了筛选后的子集，
/// 导出的语义统一成「你看到的这些行」。
async fn export_logs(State(state): State<Arc<AppState>>, Json(body): Json<ExportBody>) -> Response {
    match export_snapshot(&state, &body.lines, body.label.as_deref()) {
        Ok(value) => Json(value).into_response(),
        Err(error) => fail(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

pub fn export_snapshot(
    state: &AppState,
    lines: &[String],
    label: Option<&str>,
) -> anyhow::Result<Value> {
    if lines.is_empty() {
        anyhow::bail!("当前视图没有可导出的日志行");
    }
    if lines.len() > MAX_EXPORT_LINES {
        anyhow::bail!(
            "单次最多导出 {MAX_EXPORT_LINES} 行，当前 {} 行",
            lines.len()
        );
    }

    let dir = export_dir(state);
    std::fs::create_dir_all(&dir)?;

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let suffix = label.map(sanitize_label).unwrap_or_default();
    let name = if suffix.is_empty() {
        format!("gateway-export-{stamp}.log")
    } else {
        format!("gateway-export-{stamp}-{suffix}.log")
    };
    let path = dir.join(&name);

    let mut file = std::fs::File::create(&path)?;
    for line in lines {
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
    }
    file.sync_all()?;

    let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    Ok(json!({
        "path": path.display().to_string(),
        "dir": dir.display().to_string(),
        "bytes": bytes,
        "lines": lines.len(),
    }))
}

/// 导出目录：优先日志目录，其次 `~/Downloads`，最后系统临时目录。
pub fn export_dir(state: &AppState) -> PathBuf {
    if let Some(dir) = log_dir(state) {
        return dir;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let downloads = PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }
    std::env::temp_dir()
}

/// 只保留字母数字与短横线，避免标签被拼成路径穿越
fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(40)
        .collect()
}

#[derive(Deserialize)]
struct LevelBody {
    level: String,
}

/// 日志级别快照，供 HTTP 与 IPC 共用
pub fn log_level_snapshot(state: &AppState) -> Value {
    let current = logging::current_level();
    json!({
        "level": current.level,
        "source": current.source,
        "config_level": state.gateway.config().log.level,
        "options": logging::LEVELS,
    })
}

/// 切换级别并返回新快照。
///
/// 内存立即生效；配置文件能写就同步写回，写不进（只读、非文件来源）时
/// 在返回值里带 `persisted=false` 与原因，界面据此提示，不假装成功。
pub fn set_log_level_snapshot(state: &AppState, level: &str) -> anyhow::Result<Value> {
    let changed = logging::set_level(level)?;
    let mut snapshot = log_level_snapshot(state);

    let (persisted, error) = match state.gateway.config().source.clone() {
        Some(path) => match crate::config_store::persist_log_level(&path, &changed.level) {
            Ok(()) => (true, Value::Null),
            Err(error) => (false, Value::String(error.to_string())),
        },
        None => (
            false,
            Value::String("配置不是从文件加载的，改动只在当前进程生效".to_string()),
        ),
    };

    snapshot["persisted"] = Value::Bool(persisted);
    snapshot["persist_error"] = error;
    if persisted {
        // 已经写进文件，配置值就是当前值
        snapshot["config_level"] = Value::String(changed.level);
    }
    Ok(snapshot)
}

async fn log_level(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(log_level_snapshot(&state))
}

/// 运行期切换日志级别。只改当前进程的过滤器，不写配置文件。
async fn set_log_level(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LevelBody>,
) -> Response {
    match set_log_level_snapshot(&state, &body.level) {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(error) => fail(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

/// 生效配置快照（脱敏：配置里本来就没有凭证字段）
pub fn config_snapshot(state: &AppState) -> Value {
    let config = state.gateway.config();
    let stats = state.gateway.rule_stats();
    let registered: Vec<&str> = stats.iter().map(|stat| stat.id).collect();
    // 配置里写了但规则没注册，通常是拼写错误，值得在控制台上直接暴露
    let unregistered: Vec<&String> = config
        .rules
        .keys()
        .filter(|id| !registered.contains(&id.as_str()))
        .collect();

    json!({
        "source": config.source.as_ref().map(|path| path.display().to_string()),
        "origin": config.origin.kind(),
        "origin_text": config.origin.describe(),
        "base_dir": config.base_dir.display().to_string(),
        "listen": config.listen,
        // 读运行时的值，改完上游后这里马上反映新地址
        "upstream": state.gateway.upstream(),
        "max_body_bytes": config.max_body_bytes,
        "use_system_proxy": config.use_system_proxy,
        "log": {
            "level": config.log.level,
            "runtime_level": logging::current_level().level,
            "level_source": logging::current_level().source,
            "dir": config.log.resolve_dir(&config.base_dir).display().to_string(),
            "retain_days": config.log.retain_days,
        },
        "unregistered_rules": unregistered,
    })
}

async fn config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(config_snapshot(&state))
}

/// 上游探测快照
pub async fn upstream_snapshot(state: &AppState) -> Value {
    let probe = state.gateway.probe().await;
    json!({
        "url": state.gateway.upstream(),
        "reachable": probe.reachable,
        "status": probe.status,
        "latency_ms": probe.latency_ms,
        "error": probe.error,
    })
}

async fn upstream(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(upstream_snapshot(&state).await)
}

#[derive(Deserialize)]
struct UpstreamBody {
    upstream: String,
}

/// 改上游地址：内存立即生效（下一个请求就走新地址），同时写回配置文件。
///
/// 新地址不做连通性检查就落盘：用户可能先填地址、再启动上游。
/// 界面上有独立的探测入口，避免保存被网络状态卡住。
pub fn set_upstream_snapshot(state: &AppState, raw: &str) -> anyhow::Result<Value> {
    let upstream = crate::config_store::normalize_upstream(raw)?;
    state.gateway.set_upstream(&upstream);

    let (persisted, error) = match state.gateway.config().source.clone() {
        Some(path) => match crate::config_store::persist_upstream(&path, &upstream) {
            Ok(()) => (true, Value::Null),
            Err(error) => (false, Value::String(error.to_string())),
        },
        None => (
            false,
            Value::String("配置不是从文件加载的，改动只在当前进程生效".to_string()),
        ),
    };

    Ok(json!({
        "upstream": upstream,
        "persisted": persisted,
        "persist_error": error,
    }))
}

async fn set_upstream(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpstreamBody>,
) -> Response {
    match set_upstream_snapshot(&state, &body.upstream) {
        Ok(value) => Json(value).into_response(),
        Err(error) => fail(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

fn log_dir(state: &AppState) -> Option<PathBuf> {
    state.telemetry.log_dir().map(PathBuf::from)
}

/// 只接受本程序自己按天轮转出来的文件名，防止用 `../` 读到别处
fn is_log_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("gateway.") else {
        return false;
    };
    let Some(date) = rest.strip_suffix(".log") else {
        return false;
    };
    let parts: Vec<&str> = date.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
}

fn fail(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::{is_log_name, page_bounds, read_log_page, sanitize_label};

    #[test]
    fn accepts_rotated_log_names() {
        assert!(is_log_name("gateway.2026-11-03.log"));
    }

    #[test]
    fn rejects_path_traversal_and_other_files() {
        for name in [
            "../gateway.2026-11-03.log",
            "/etc/passwd",
            "gateway.log",
            "gateway.2026-11.log",
            "gateway.2026-11-03.log.gz",
            "other.2026-11-03.log",
        ] {
            assert!(!is_log_name(name), "{name} 不应被接受");
        }
    }

    #[test]
    fn page_bounds_covers_tail_earlier_and_limits() {
        assert_eq!(page_bounds(0, None, 400), (0, 0));
        assert_eq!(page_bounds(1000, None, 400), (600, 1000));
        assert_eq!(page_bounds(1000, Some(600), 400), (200, 600));
        assert_eq!(page_bounds(1000, Some(50), 400), (0, 50));
        // before 超过总数按总数算，不能越界
        assert_eq!(page_bounds(10, Some(99), 400), (0, 10));
        // limit 下限 1、上限 MAX_LOG_PAGE
        assert_eq!(page_bounds(10, None, 0), (9, 10));
        let (start, end) = page_bounds(5000, None, 999_999);
        assert_eq!(end - start, 2000);
    }

    #[test]
    fn reads_pages_before_and_tail() {
        let dir = std::env::temp_dir().join(format!("cg-log-page-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let name = "gateway.2026-11-05.log";
        let mut text = String::new();
        for index in 0..10 {
            text.push_str(&format!("line-{index}\n"));
        }
        std::fs::write(dir.join(name), text).expect("写入测试日志");

        let tail = read_log_page(&dir, name, None, 3).expect("读尾部");
        assert_eq!(tail.total, 10);
        assert_eq!(tail.start, 7);
        assert_eq!(tail.lines, ["line-7", "line-8", "line-9"].map(String::from));
        assert!(tail.has_more());

        let earlier = read_log_page(&dir, name, Some(tail.start), 3).expect("向上翻页");
        assert_eq!(earlier.start, 4);
        assert_eq!(
            earlier.lines,
            ["line-4", "line-5", "line-6"].map(String::from)
        );

        let first = read_log_page(&dir, name, Some(4), 10).expect("读开头");
        assert_eq!(first.start, 0);
        assert!(!first.has_more());
        assert_eq!(first.lines.len(), 4);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn export_label_cannot_escape_directory() {
        assert_eq!(sanitize_label("../a b/c"), "abc");
        assert_eq!(sanitize_label("gateway.2026-11-05"), "gateway2026-11-05");
    }
}
