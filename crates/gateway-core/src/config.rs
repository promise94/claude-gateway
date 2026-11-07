use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::paths::ConfigOrigin;

fn default_listen() -> String {
    "127.0.0.1:15722".to_string()
}

fn default_upstream() -> String {
    "http://127.0.0.1:15721".to_string()
}

fn default_max_body_bytes() -> usize {
    64 * 1024 * 1024
}

/// tool_reference 降级文本的默认模板，`{names}` 会被替换成工具名列表
pub const DEFAULT_TOOL_REFERENCE_TEMPLATE: &str = "[tool_search] 已加载工具: {names}";

/// 单条规则的配置
///
/// 参数放在 `params` 表里而不是平铺成字段：加新参数不需要改结构体，
/// 规则通过 [`crate::rewrite::ParamSpec`] 自行声明有哪些参数、默认值是什么。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RuleConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// 规则参数，键名由各规则的 `ParamSpec` 定义
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

impl RuleConfig {
    /// 取某个参数，缺省时回退到规则声明的默认值
    pub fn param(&self, key: &str, default: &str) -> String {
        self.params
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    /// 生效的 tool_reference 降级模板
    pub fn text_template(&self) -> String {
        self.param("text_template", DEFAULT_TOOL_REFERENCE_TEMPLATE)
    }
}

fn default_true() -> bool {
    true
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            params: BTreeMap::new(),
        }
    }
}

/// 校验规则参数。只校验当前已知的参数，未来加参数在这里补。
pub fn validate_rule_config(config: &RuleConfig) -> Result<(), String> {
    if let Some(template) = config.params.get("text_template") {
        if template.trim().is_empty() {
            return Err("text_template 不能为空".to_string());
        }
        if !template.contains("{names}") {
            return Err("text_template 必须包含 {names} 占位符".to_string());
        }
        if template.chars().count() > 500 {
            return Err("text_template 不能超过 500 个字符".to_string());
        }
    }
    Ok(())
}

fn default_log_level() -> String {
    "info".to_string()
}

/// 留空表示用平台默认目录，与 `paths.rs` 首次生成的配置模板保持一致。
fn default_log_dir() -> String {
    String::new()
}

fn default_log_retain_days() -> usize {
    7
}

/// 日志配置
#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    /// 日志级别：trace / debug / info / warn / error
    #[serde(default = "default_log_level")]
    pub level: String,

    /// 日志目录。留空用平台默认目录（见 [`crate::paths::log_dir`]），日志始终落盘。
    ///
    /// 相对路径以**配置文件所在目录**为基准解析，不随启动位置变化。
    /// 支持 `~` 展开。
    #[serde(default = "default_log_dir")]
    pub dir: String,

    /// 保留最近多少个日志文件。按本地日期轮转，切换文件时按文件个数清理，最小 1。
    #[serde(default = "default_log_retain_days")]
    pub retain_days: usize,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            dir: default_log_dir(),
            retain_days: default_log_retain_days(),
        }
    }
}

impl LogConfig {
    /// 解析出实际日志目录。
    ///
    /// `dir` 为空时用平台默认目录（见 [`crate::paths::log_dir`]）；
    /// 相对路径以**配置文件所在目录**为基准，不随启动位置变化。
    pub fn resolve_dir(&self, base_dir: &Path) -> PathBuf {
        let raw = self.dir.trim();
        if raw.is_empty() {
            return crate::paths::log_dir();
        }
        crate::paths::resolve_path(raw, base_dir)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// 监听地址，形如 `127.0.0.1:15722`
    #[serde(default = "default_listen")]
    pub listen: String,
    /// 上游地址，不带结尾斜杠
    #[serde(default = "default_upstream")]
    pub upstream: String,

    /// 单条请求体上限，超过直接拒绝
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,

    /// 转发上游时是否使用系统代理（HTTP_PROXY / HTTPS_PROXY 等环境变量）。
    ///
    /// 默认 false。上游通常是本机 cc-switch，走代理会让请求被错误路由，
    /// 并且 URI 会被改成 absolute-form，部分上游会直接拒绝。
    #[serde(default)]
    pub use_system_proxy: bool,

    /// 规则开关，键为规则 id
    #[serde(default)]
    pub rules: BTreeMap<String, RuleConfig>,

    /// 日志配置
    #[serde(default)]
    pub log: LogConfig,

    /// 相对路径的解析基准：配置文件所在目录；使用内置默认配置时为当前工作目录
    #[serde(skip)]
    pub base_dir: PathBuf,

    /// 实际加载的配置文件路径。`None` 表示使用的内置默认配置。
    #[serde(skip)]
    pub source: Option<PathBuf>,

    /// 这份配置是怎么来的：已有 / 迁移 / 新建 / 环境变量 / 内置默认
    #[serde(skip)]
    pub origin: ConfigOrigin,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            upstream: default_upstream(),
            max_body_bytes: default_max_body_bytes(),
            use_system_proxy: false,
            rules: BTreeMap::new(),
            log: LogConfig::default(),
            base_dir: crate::paths::current_dir(),
            source: None,
            origin: ConfigOrigin::Builtin,
        }
    }
}

impl Config {
    /// 加载配置：先保证固定目录下有一份配置，再读它。
    ///
    /// 顺序：
    /// 1. `CLAUDE_GATEWAY_CONFIG` 环境变量（显式指定，不创建也不迁移）
    /// 2. 固定目录已有配置（见 [`crate::paths::config_path`]）
    /// 3. 从旧位置迁移：当前工作目录、可执行文件真实目录
    /// 4. 都没有则写一份带注释的默认配置
    ///
    /// 写盘失败（权限、只读文件系统）不阻塞启动，退回内置默认值。
    ///
    /// 这里不打日志：调用时日志系统还没初始化（级别来自本文件），
    /// 由入口在初始化之后把来源打出来。
    pub fn discover() -> anyhow::Result<Self> {
        let origin = match crate::paths::ensure_config() {
            Ok(origin) => origin,
            Err(error) => {
                eprintln!("claude-gateway: 准备配置文件失败：{error}");
                ConfigOrigin::Builtin
            }
        };

        let Some(path) = origin.path().map(Path::to_path_buf) else {
            return Ok(Self {
                origin,
                ..Self::default()
            });
        };

        let mut config = Self::load(&path)?;
        config.origin = origin;
        Ok(config)
    }

    /// 从指定文件加载。文件不存在时返回内置默认配置。
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&raw)?;

        // 用真实路径，保证 base_dir 是绝对路径，不受软链和相对路径影响
        let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        config.base_dir = absolute
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(crate::paths::current_dir);
        config.source = Some(absolute);
        Ok(config)
    }

    /// 上游地址去掉结尾斜杠，避免拼接出双斜杠
    pub fn upstream_base(&self) -> &str {
        self.upstream.trim_end_matches('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log_dir_falls_back_to_platform_dir() {
        let log = LogConfig {
            dir: "   ".to_string(),
            ..LogConfig::default()
        };
        assert_eq!(log.resolve_dir(Path::new("/tmp")), crate::paths::log_dir());
    }

    /// 配置里删掉 `log.dir` 时，serde 默认值必须和首次生成的模板一致，
    /// 否则同一份配置在「有模板/无模板」两种情况下会落到不同目录。
    #[test]
    fn default_log_dir_matches_generated_template() {
        let log = LogConfig::default();
        assert!(log.dir.is_empty(), "默认应留空以走平台目录");
        assert_eq!(log.resolve_dir(Path::new("/tmp")), crate::paths::log_dir());
    }

    #[test]
    fn log_dir_resolves_against_base_not_cwd() {
        let log = LogConfig {
            dir: "./logs".to_string(),
            ..LogConfig::default()
        };
        assert_eq!(
            log.resolve_dir(Path::new("/opt/gw")),
            Path::new("/opt/gw/logs")
        );
    }

    #[test]
    fn load_sets_base_dir_to_config_parent() {
        let dir = std::env::temp_dir().join("cg-config-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "listen = \"127.0.0.1:19999\"\n").unwrap();

        let config = Config::load(&path).unwrap();

        assert_eq!(config.listen, "127.0.0.1:19999");
        assert_eq!(config.base_dir, std::fs::canonicalize(&dir).unwrap());
        assert!(config.source.is_some());

        std::fs::remove_dir_all(&dir).ok();
    }
}
