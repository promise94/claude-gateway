//! 配置文件就地写回
//!
//! 规则由 `rules_store::persist` 负责；这里承载其他需要同步落盘的字段
//! （目前是日志级别）。两者都走 `toml_edit`，保留注释与段落顺序；
//! 写临时文件再 rename，避免写到一半留下半个配置。

use anyhow::{Context, Result};
use std::path::Path;

/// 把日志级别写回 `[log].level`
pub fn persist_log_level(path: &Path, level: &str) -> Result<()> {
    edit(path, |doc| {
        if !doc.contains_key("log") {
            doc["log"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        if let Some(table) = doc["log"].as_table_mut() {
            table["level"] = toml_edit::value(level);
            return;
        }
        // `log = { level = "..." }` 这种内联表也接受
        if let Some(inline) = doc["log"].as_inline_table_mut() {
            inline.insert("level", toml_edit::Value::from(level));
        }
    })
}

/// 把上游地址写回顶层 `upstream`
pub fn persist_upstream(path: &Path, upstream: &str) -> Result<()> {
    edit(path, |doc| {
        doc["upstream"] = toml_edit::value(upstream);
    })
}

/// 校验并规范化上游地址。
///
/// 只接受 http/https 的绝对地址；允许带路径（有些网关是 `https://host/api/gw`），
/// 不允许查询串和锚点——它们会被拼到转发路径后面，语义不明确。
pub fn normalize_upstream(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("上游地址不能为空");
    }

    let url = reqwest::Url::parse(trimmed).map_err(|error| {
        anyhow::anyhow!("地址格式不正确：{error}（需要形如 http://127.0.0.1:15721）")
    })?;

    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("只支持 http 或 https，当前是 {}", url.scheme());
    }
    if url.host_str().is_none() {
        anyhow::bail!("地址缺少主机名");
    }
    if url.query().is_some() {
        anyhow::bail!("上游地址不能带查询串（? 之后的部分）");
    }
    if url.fragment().is_some() {
        anyhow::bail!("上游地址不能带锚点（# 之后的部分）");
    }

    Ok(trimmed.trim_end_matches('/').to_string())
}

/// 就地编辑配置文件并原子替换
pub fn edit(path: &Path, apply: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .with_context(|| format!("解析配置文件失败: {}", path.display()))?;

    apply(&mut doc);

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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_config(content: &str) -> PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("cg-config-store-{}-{seq}.toml", std::process::id()));
        std::fs::write(&path, content).expect("写临时配置");
        path
    }

    #[test]
    fn persist_log_level_keeps_comments_and_other_sections() {
        let path = temp_config(
            "# 顶部注释\n\n[log]\n# 级别注释\nlevel = \"info\"\nretain_days = 7\n\n[rules.x]\nenabled = true\n",
        );

        persist_log_level(&path, "debug").expect("写回级别");

        let written = std::fs::read_to_string(&path).expect("读回");
        assert!(written.contains("# 顶部注释"), "顶部注释应保留");
        assert!(written.contains("# 级别注释"), "段落注释应保留");
        assert!(written.contains("level = \"debug\""), "级别应更新");
        assert!(written.contains("retain_days = 7"), "其他字段不应丢失");
        assert!(written.contains("[rules.x]"), "其他段落不应丢失");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn persist_log_level_creates_missing_section() {
        let path = temp_config("listen = \"127.0.0.1:15722\"\n");

        persist_log_level(&path, "warn").expect("写回级别");

        let written = std::fs::read_to_string(&path).expect("读回");
        assert!(written.contains("[log]"));
        assert!(written.contains("level = \"warn\""));
        assert!(written.contains("listen = \"127.0.0.1:15722\""));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn persist_upstream_updates_key_and_keeps_rest() {
        let path = temp_config(
            "# 顶部注释\nlisten = \"127.0.0.1:15722\"\nupstream = \"http://127.0.0.1:15721\"\n\n[log]\nlevel = \"info\"\n",
        );

        persist_upstream(&path, "https://api.example.com/gw").expect("写回上游");

        let written = std::fs::read_to_string(&path).expect("读回");
        assert!(written.contains("upstream = \"https://api.example.com/gw\""));
        assert!(written.contains("# 顶部注释"));
        assert!(written.contains("listen = \"127.0.0.1:15722\""));
        assert!(written.contains("[log]"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn normalize_upstream_accepts_http_and_trims_trailing_slash() {
        assert_eq!(
            normalize_upstream("http://127.0.0.1:15721/").expect("合法"),
            "http://127.0.0.1:15721"
        );
        assert_eq!(
            normalize_upstream("  https://api.example.com/gw  ").expect("合法"),
            "https://api.example.com/gw"
        );
    }

    #[test]
    fn normalize_upstream_rejects_bad_input() {
        for raw in [
            "",
            "   ",
            "127.0.0.1:15721",
            "ftp://example.com",
            "http://example.com/?token=1",
            "http://example.com/#frag",
        ] {
            assert!(normalize_upstream(raw).is_err(), "{raw} 应被拒绝");
        }
    }
}
