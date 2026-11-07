//! 把 `tool_result` 里的 `tool_reference` 降级为文本
//!
//! Claude Code 的 ToolSearch（延迟工具加载）会返回这种内容块：
//!
//! ```json
//! { "type": "tool_reference", "tool_name": "WebFetch" }
//! ```
//!
//! 上游 token 计数器只认 text / image_url / image / document / tool_use /
//! tool_result / thinking，遇到该类型直接抛异常：
//!
//! ```text
//! 500 api_error
//! Invalid content item type: tool_reference
//! ```
//!
//! 降级为 text 后语义等价：它原本只是「以下工具已加载」的声明。
//! 降级文本的模板可配置，见 [`SPEC`] 里的 `text_template` 参数。

use super::{ParamSpec, RewriteRule, RuleSpec};
use crate::config::DEFAULT_TOOL_REFERENCE_TEMPLATE;
use serde_json::{json, Value};
use std::collections::BTreeMap;

static SPEC: RuleSpec = RuleSpec {
    id: "tool_reference",
    label: "tool_reference 降级为文本",
    description:
        "上游不认识 tool_reference 内容块，降级为 text 后语义等价；模板可配置，须含 {names} 占位符",
    params: &[ParamSpec {
        key: "text_template",
        label: "降级文本模板",
        default: DEFAULT_TOOL_REFERENCE_TEMPLATE,
        required_placeholder: Some("{names}"),
    }],
};

pub struct ToolReferenceDowngrade;

impl RewriteRule for ToolReferenceDowngrade {
    fn spec(&self) -> &'static RuleSpec {
        &SPEC
    }

    fn apply(&self, body: &mut Value, params: &BTreeMap<String, String>) -> usize {
        let template = params
            .get("text_template")
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_TOOL_REFERENCE_TEMPLATE);

        let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
            return 0;
        };

        let mut hits = 0usize;
        for message in messages.iter_mut() {
            let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
                continue;
            };
            for block in blocks.iter_mut() {
                hits += rewrite_tool_result(block, template);
            }
        }
        hits
    }
}

/// 处理单个 content block，返回替换掉的 tool_reference 数量
fn rewrite_tool_result(block: &mut Value, template: &str) -> usize {
    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
        return 0;
    }
    let Some(items) = block.get_mut("content").and_then(Value::as_array_mut) else {
        return 0;
    };

    // 先扫一遍确认有没有可替换项，避免无谓克隆
    if !items
        .iter()
        .any(|item| item.get("type").and_then(Value::as_str) == Some("tool_reference"))
    {
        return 0;
    }

    let mut names: Vec<String> = Vec::new();
    let mut kept: Vec<Value> = Vec::with_capacity(items.len());
    for item in items.iter() {
        if item.get("type").and_then(Value::as_str) == Some("tool_reference") {
            names.push(
                item.get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
            );
        } else {
            kept.push(item.clone());
        }
    }

    let hits = names.len();
    kept.push(json!({
        "type": "text",
        "text": template.replace("{names}", &names.join(", "))
    }));
    *items = kept;
    hits
}
