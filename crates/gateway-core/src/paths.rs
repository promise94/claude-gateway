//! 平台目录与首次启动的配置落地
//!
//! 配置和日志不再跟着「从哪个目录启动」跑，而是固定到各平台约定的位置：
//!
//! | 平台 | 配置 | 日志 |
//! | --- | --- | --- |
//! | macOS | `~/.config/claude-gateway/` | `~/Library/Logs/claude-gateway/` |
//! | Linux | `$XDG_CONFIG_HOME/claude-gateway/` | `$XDG_STATE_HOME/claude-gateway/logs/` |
//! | Windows | `%APPDATA%\claude-gateway\` | `%LOCALAPPDATA%\claude-gateway\logs\` |
//!
//! macOS 的配置没走 `~/Library/Application Support`，因为本项目同时有 CLI 入口，
//! 配置放 `~/.config` 更好找、也能进 dotfiles；日志则跟平台惯例走，
//! `~/Library/Logs` 在 Console.app 里可以直接看到。
//!
//! 启动时保证配置文件存在：优先用已有文件；没有则尝试从旧位置（工作目录、
//! 可执行文件目录）迁移一份；都没有就写内置默认模板。迁移只读源文件，不删除。

use std::path::{Path, PathBuf};

pub const APP: &str = "claude-gateway";

/// 首次启动写入的默认配置。带注释，用户可以直接改。
pub const DEFAULT_CONFIG: &str = r#"# claude-gateway 配置
#
# 本文件由程序首次启动时生成。界面上的改动（规则开关、参数、日志级别）
# 会同步写回这里，写回时保留本文件的注释与段落顺序。
#
# upstream 是本网关的转发目标，两种接法只能选一种，混用会形成循环：
#   方式 A（推荐）：Claude Code -> cc-switch -> gateway -> 上游
#       upstream 填真实上游地址，cc-switch 里把 provider 地址改成网关监听地址
#   方式 B：Claude Code -> gateway -> cc-switch -> 上游
#       upstream 指向 cc-switch；cc-switch 会重写 settings.json，切换 provider 后容易失效
listen = "127.0.0.1:15722"
upstream = "http://127.0.0.1:15721"

# 单条请求体上限（字节），超过直接拒绝，避免被打爆内存
max_body_bytes = 67108864

# 转发上游时是否使用系统代理。上游通常是本机服务，保持 false。
use_system_proxy = false

[log]
# 级别：trace / debug / info / warn / error。RUST_LOG 环境变量优先级更高。
level = "info"
# 日志目录。留空表示用平台默认目录：
#   macOS   ~/Library/Logs/claude-gateway/
#   Linux   ~/.local/state/claude-gateway/logs/
#   Windows %LOCALAPPDATA%\claude-gateway\logs\
# 填相对路径时以本配置文件所在目录为基准；支持 ~ 展开。
dir = ""
# 保留最近多少个日志文件（按天轮转，按文件个数清理）
retain_days = 7

# 每条规则一个开关，未列出的规则视为启用
[rules.thinking_disable]
enabled = true

[rules.tool_reference]
enabled = true
"#;

/// 配置的来源，启动日志和面板都要能说清楚「这份配置是哪来的」
///
/// `Default` 是 `serde(skip)` 需要的：反序列化时该字段用 `Builtin` 填充，
/// 真正的来源由 [`ensure_config`] 决定。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// 目标位置已有配置
    Existing(PathBuf),
    /// 从旧位置迁移而来，源文件保留
    Migrated { from: PathBuf, to: PathBuf },
    /// 本次新建的默认配置
    Created(PathBuf),
    /// 由 `CLAUDE_GATEWAY_CONFIG` 指定，不创建也不迁移
    FromEnv(PathBuf),
    /// 无法确定位置，使用内置默认值（只读，不会落盘）
    #[default]
    Builtin,
}

impl ConfigOrigin {
    pub fn describe(&self) -> String {
        match self {
            Self::Existing(path) => format!("已加载配置文件 {}", path.display()),
            Self::Migrated { from, .. } => {
                format!("已从 {} 迁移配置到固定目录", from.display())
            }
            Self::Created(path) => format!("已写入默认配置 {}", path.display()),
            Self::FromEnv(path) => format!("使用环境变量指定的配置 {}", path.display()),
            Self::Builtin => "未找到配置且无法写入，使用内置默认值".to_string(),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Existing(path) | Self::Created(path) | Self::FromEnv(path) => Some(path),
            Self::Migrated { to, .. } => Some(to),
            Self::Builtin => None,
        }
    }

    /// 给前端用的稳定标识，不要拿 `describe()` 的文案做判断
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Existing(_) => "existing",
            Self::Migrated { .. } => "migrated",
            Self::Created(_) => "created",
            Self::FromEnv(_) => "env",
            Self::Builtin => "builtin",
        }
    }
}

pub fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(profile);
        }
        let drive = std::env::var_os("HOMEDRIVE").unwrap_or_default();
        let path = std::env::var_os("HOMEPATH").unwrap_or_default();
        let mut home = PathBuf::from(drive);
        home.push(path);
        if !home.as_os_str().is_empty() {
            return home;
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// 配置文件所在目录
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        env_dir("APPDATA")
            .unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
            .join(APP)
    }
    #[cfg(not(target_os = "windows"))]
    {
        // macOS 也用 ~/.config：本项目有 CLI 入口，配置要能被 dotfiles 管理
        env_dir("XDG_CONFIG_HOME")
            .unwrap_or_else(|| home_dir().join(".config"))
            .join(APP)
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// 平台默认日志目录
pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home_dir().join("Library").join("Logs").join(APP)
    }
    #[cfg(target_os = "windows")]
    {
        env_dir("LOCALAPPDATA")
            .unwrap_or_else(|| home_dir().join("AppData").join("Local"))
            .join(APP)
            .join("logs")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env_dir("XDG_STATE_HOME")
            .unwrap_or_else(|| home_dir().join(".local").join("state"))
            .join(APP)
            .join("logs")
    }
}

/// 展开 `~`，把相对路径按 `base` 解析成绝对路径。
///
/// 存在的理由：把二进制软链成全局命令后，若仍以进程工作目录为基准，
/// 日志会散落在每个启动过的目录里，配置也可能悄悄加载成默认值。
pub fn resolve_path(raw: &str, base: &Path) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return normalize(home_dir().join(rest));
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        normalize(path)
    } else {
        normalize(base.join(path))
    }
}

/// 去掉路径里的 `.` 组件，避免出现 `/path/./logs` 这种可读性差的形态
pub fn normalize(path: PathBuf) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `CLAUDE_GATEWAY_CONFIG` 指定的路径。显式指定时不创建、不迁移。
pub fn env_config_path() -> Option<PathBuf> {
    let raw = std::env::var_os("CLAUDE_GATEWAY_CONFIG")?;
    let raw = raw.to_string_lossy().to_string();
    if raw.trim().is_empty() {
        return None;
    }
    Some(resolve_path(&raw, &current_dir()))
}

/// 旧位置里可能存在的配置：当前工作目录、可执行文件真实目录。
///
/// 跳过 `/` 这类根目录，避免误读系统里的 `config.toml`；
/// 也不包含 `~/.config/claude-gateway/`，那是目标位置本身。
fn legacy_config() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    let cwd = current_dir();
    if cwd != Path::new("/") {
        candidates.push(cwd.join("config.toml"));
    }

    if let Ok(exe) = std::env::current_exe() {
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("config.toml"));
        }
    }

    candidates.into_iter().find(|path| path.is_file())
}

/// 保证配置文件存在，返回它的来源。
///
/// 顺序：环境变量 -> 目标位置已有 -> 从旧位置迁移 -> 写默认模板。
pub fn ensure_config() -> anyhow::Result<ConfigOrigin> {
    if let Some(path) = env_config_path() {
        return Ok(ConfigOrigin::FromEnv(path));
    }

    let target = config_path();
    if target.is_file() {
        return Ok(ConfigOrigin::Existing(target));
    }

    if let Some(from) = legacy_config() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&from, &target)?;
        return Ok(ConfigOrigin::Migrated { from, to: target });
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, DEFAULT_CONFIG)?;
    Ok(ConfigOrigin::Created(target))
}

#[cfg(test)]
mod tests {
    use super::{normalize, resolve_path};
    use std::path::{Path, PathBuf};

    #[test]
    fn resolve_path_expands_home() {
        let base = Path::new("/tmp/base");
        let home = super::home_dir();

        assert_eq!(resolve_path("~/logs", base), home.join("logs"));
        assert_eq!(resolve_path("~", base), home);
    }

    #[test]
    fn resolve_path_keeps_absolute() {
        let base = Path::new("/tmp/base");
        assert_eq!(
            resolve_path("/var/log/gw", base),
            PathBuf::from("/var/log/gw")
        );
    }

    #[test]
    fn normalize_drops_cur_dir() {
        assert_eq!(
            normalize(PathBuf::from("/tmp/./logs/./a")),
            PathBuf::from("/tmp/logs/a")
        );
    }

    #[test]
    fn config_dir_ends_with_app_name() {
        let dir = super::config_dir();
        assert_eq!(
            dir.file_name().and_then(|name| name.to_str()),
            Some(super::APP)
        );
    }

    #[test]
    fn default_config_parses_and_matches_defaults() {
        let config: crate::config::Config =
            toml::from_str(super::DEFAULT_CONFIG).expect("默认配置必须是合法 TOML");
        assert_eq!(config.listen, "127.0.0.1:15722");
        assert_eq!(config.log.level, "info");
        assert!(config.log.dir.is_empty(), "默认日志目录留空走平台路径");
        assert_eq!(config.log.retain_days, 7);
    }
}
