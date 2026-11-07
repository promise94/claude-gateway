//! 前端可调用的命令。
//!
//! 全部转发到 `gateway_core::panel` 的快照函数，和 HTTP 控制台共用一份实现——
//! 两条出口各写一遍必然漂移。

use gateway_core::panel;
use serde_json::{json, Value};
use tauri::State;

use crate::ctx::AppCtx;

#[tauri::command]
pub fn status(ctx: State<'_, AppCtx>) -> Value {
    let mut snapshot = panel::status_snapshot(&ctx.state);
    // 网关没起来时界面要能一眼看出来，所以把状态和原因一起给出
    snapshot["running"] = Value::Bool(ctx.listen_error.is_none());
    snapshot["listen_error"] = match &ctx.listen_error {
        Some(error) => Value::String(error.clone()),
        None => Value::Null,
    };
    snapshot
}

#[tauri::command]
pub fn logs(ctx: State<'_, AppCtx>, limit: Option<usize>, level: Option<String>) -> Value {
    panel::logs_snapshot(&ctx.state, limit.unwrap_or(300), level.as_deref())
}

/// 落盘的日志文件清单。日志目录未配置时返回空列表与 `dir: null`。
#[tauri::command]
pub fn log_files(ctx: State<'_, AppCtx>) -> Value {
    panel::files_snapshot(&ctx.state)
}

/// 读取一页落盘日志。`before` 缺省表示文件末尾，传上一次的 `start` 即向上翻页。
#[tauri::command]
pub fn log_file(
    ctx: State<'_, AppCtx>,
    name: String,
    before: Option<usize>,
    limit: Option<usize>,
) -> Result<Value, String> {
    panel::logfile_snapshot(&ctx.state, &name, before, limit.unwrap_or(400))
        .map_err(|(_, message)| message)
}

#[tauri::command]
pub fn log_level(ctx: State<'_, AppCtx>) -> Value {
    panel::log_level_snapshot(&ctx.state)
}

/// 运行期切换日志级别。不写配置文件，重启后回到 config.toml 的值。
#[tauri::command]
pub fn log_level_set(ctx: State<'_, AppCtx>, level: String) -> Result<Value, String> {
    panel::set_log_level_snapshot(&ctx.state, &level).map_err(|error| error.to_string())
}

/// 把界面当前显示的行导出成文件，返回绝对路径
#[tauri::command]
pub fn export_logs(
    ctx: State<'_, AppCtx>,
    lines: Vec<String>,
    label: Option<String>,
) -> Result<Value, String> {
    panel::export_snapshot(&ctx.state, &lines, label.as_deref()).map_err(|error| error.to_string())
}

/// 在系统文件管理器里打开日志目录
#[tauri::command]
pub fn open_log_dir(ctx: State<'_, AppCtx>) -> Result<String, String> {
    let dir = ctx
        .state
        .telemetry
        .log_dir()
        .ok_or("日志目录尚未就绪，无法打开")?;
    let path = std::path::PathBuf::from(&dir);
    if !path.is_dir() {
        return Err(format!("日志目录不存在：{dir}"));
    }

    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let opener = "xdg-open";

    std::process::Command::new(opener)
        .arg(&path)
        .spawn()
        .map_err(|error| format!("打开日志目录失败：{error}"))?;
    Ok(dir)
}

#[tauri::command]
pub fn config(ctx: State<'_, AppCtx>) -> Value {
    panel::config_snapshot(&ctx.state)
}

#[tauri::command]
pub async fn upstream(ctx: State<'_, AppCtx>) -> Result<Value, String> {
    Ok(panel::upstream_snapshot(&ctx.state).await)
}

/// 改上游地址：下一个请求立即生效，同时写回配置文件
#[tauri::command]
pub fn upstream_set(ctx: State<'_, AppCtx>, upstream: String) -> Result<Value, String> {
    panel::set_upstream_snapshot(&ctx.state, &upstream).map_err(|error| error.to_string())
}

/// 连通性探针，前端启动时先调它确认 IPC 通道可用
#[tauri::command]
pub fn ping() -> Value {
    json!({ "ok": true })
}

/// 改一条规则的开关与参数，并立即写回配置文件。
///
/// 写盘失败不影响内存生效，返回值里带 `persisted` 与 `save_error`，由界面提示。
#[tauri::command]
pub fn rule_update(
    ctx: State<'_, AppCtx>,
    id: String,
    enabled: bool,
    params: Option<std::collections::BTreeMap<String, String>>,
) -> Result<Value, String> {
    let config = gateway_core::RuleConfig {
        enabled,
        params: params.unwrap_or_default(),
    };

    let outcome = panel::update_rule_and_persist(&ctx.state, &id, config)
        .map_err(|error| error.to_string())?;

    Ok(panel::rule_update_json(&ctx.state, &outcome))
}

/// 把规则配置写回 config.toml。写前备份，保留注释与段落顺序。
#[tauri::command]
pub fn rule_save(ctx: State<'_, AppCtx>) -> Result<Value, String> {
    let path = ctx
        .state
        .gateway
        .save_rules()
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "saved": true,
        "path": path.display().to_string(),
        "dirty": ctx.state.gateway.rules_dirty(),
    }))
}

/// 丢弃未落盘的改动，从配置文件重新读取
#[tauri::command]
pub fn rule_discard(ctx: State<'_, AppCtx>) -> Result<Value, String> {
    ctx.state
        .gateway
        .discard_rules()
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "dirty": ctx.state.gateway.rules_dirty(),
        "rules": ctx
            .state
            .gateway
            .rule_stats()
            .iter()
            .map(panel::rule_json)
            .collect::<Vec<_>>(),
    }))
}
