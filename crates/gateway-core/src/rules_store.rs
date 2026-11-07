//! 规则运行时状态与持久化
//!
//! 规则参数可以在界面上修改，改完立即生效；落盘用 `toml_edit` 就地修改，
//! 保留 `config.toml` 里的注释与段落顺序，不做整篇重写。

use crate::config::RuleConfig;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 运行时规则状态
pub struct RuleState {
    configs: BTreeMap<String, RuleConfig>,
    /// 有未落盘的改动
    dirty: bool,
    /// config.toml 路径，用于落盘与回读。配置来自内置默认值时为空。
    source: Option<PathBuf>,
}

impl RuleState {
    pub fn new(configs: BTreeMap<String, RuleConfig>, source: Option<PathBuf>) -> Self {
        Self {
            configs,
            dirty: false,
            source,
        }
    }

    /// 取某条规则的当前配置，未配置时返回默认值（启用、无参数）
    pub fn config(&self, id: &str) -> RuleConfig {
        self.configs.get(id).cloned().unwrap_or_default()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 改一条规则的配置，只作用于内存，调用 [`persist`] 才落盘
    pub fn set(&mut self, id: &str, config: RuleConfig) {
        self.configs.insert(id.to_string(), config);
        self.dirty = true;
    }

    /// 用文件里的内容替换全部规则配置
    pub fn replace(&mut self, configs: BTreeMap<String, RuleConfig>) {
        self.configs = configs;
        self.dirty = false;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn snapshot(&self) -> BTreeMap<String, RuleConfig> {
        self.configs.clone()
    }

    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }
}

/// 就地写回 `config.toml` 的 `[rules.<id>]` 段
///
/// 只增改目标字段，不动其他段落；参数由 `params` 子表承载。
pub fn persist(path: &Path, configs: &BTreeMap<String, RuleConfig>) -> Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .with_context(|| format!("解析配置文件失败: {}", path.display()))?;

    if !doc.contains_key("rules") {
        doc["rules"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let rules = doc["rules"]
        .as_table_mut()
        .context("config.toml 的 rules 必须是表")?;

    for (id, config) in configs {
        if !rules.contains_key(id) {
            rules.insert(id, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let table = rules[id]
            .as_table_mut()
            .with_context(|| format!("rules.{id} 必须是表"))?;
        table["enabled"] = toml_edit::value(config.enabled);

        if config.params.is_empty() {
            table.remove("params");
            continue;
        }

        if !table.contains_key("params") {
            table.insert("params", toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let params = table["params"]
            .as_table_mut()
            .with_context(|| format!("rules.{id}.params 必须是表"))?;

        // 删掉已经被移除的参数，避免文件里留下无效配置
        let stale: Vec<String> = params
            .iter()
            .map(|(key, _)| key.to_string())
            .filter(|key| !config.params.contains_key(key))
            .collect();
        for key in stale {
            params.remove(&key);
        }
        for (key, value) in &config.params {
            params[key] = toml_edit::value(value.clone());
        }
    }

    // 先写临时文件再重命名，避免写入中断留下半个配置文件
    let temp = path.with_extension("toml.tmp");
    std::fs::write(&temp, doc.to_string())
        .with_context(|| format!("写临时文件失败: {}", temp.display()))?;
    std::fs::rename(&temp, path)
        .with_context(|| format!("替换配置文件失败: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cg-rules-store-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn persist_keeps_comments_and_other_sections() {
        let path = temp_config(
            r#"# 顶部注释，必须保留
listen = "127.0.0.1:15722"

[rules.thinking_disable]
enabled = true

# 规则说明注释
[rules.tool_reference]
enabled = true
"#,
        );

        let mut configs = BTreeMap::new();
        configs.insert(
            "tool_reference".to_string(),
            RuleConfig {
                enabled: false,
                params: BTreeMap::from([(
                    "text_template".to_string(),
                    "已加载 {names}".to_string(),
                )]),
            },
        );
        persist(&path, &configs).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# 顶部注释，必须保留"), "注释丢了");
        assert!(written.contains("# 规则说明注释"), "段落注释丢了");
        assert!(written.contains("enabled = false"), "开关没写进去");
        assert!(written.contains("已加载 {names}"), "参数没写进去");
        assert!(
            written.contains("listen = \"127.0.0.1:15722\""),
            "其他段落被改坏"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persist_removes_stale_params() {
        let path = temp_config(
            r#"[rules.tool_reference]
enabled = true

[rules.tool_reference.params]
text_template = "旧模板 {names}"
"#,
        );

        let mut configs = BTreeMap::new();
        configs.insert(
            "tool_reference".to_string(),
            RuleConfig {
                enabled: true,
                params: BTreeMap::new(),
            },
        );
        persist(&path, &configs).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("旧模板"), "过期参数没清掉");

        let _ = std::fs::remove_file(path);
    }
}
