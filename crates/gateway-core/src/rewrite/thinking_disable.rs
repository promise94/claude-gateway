//! 剥离 `thinking: { type: "disabled" }`
//!
//! Claude Code 的 auto 权限模式在分类器请求里显式禁用 thinking，
//! 部分上游网关（LiteLLM 系）未配置 `model_info.thinking_disable_mode`，
//! 无法翻译该取值，直接返回：
//!
//! ```text
//! 400 invalid_request_error
//! thinking.type=disabled cannot be translated
//! ```
//!
//! 删除整个 `thinking` 字段等价于「不声明 thinking」，语义一致。

use super::{RewriteRule, RuleSpec};
use serde_json::Value;
use std::collections::BTreeMap;

static SPEC: RuleSpec = RuleSpec {
    id: "thinking_disable",
    label: "剥离 thinking.type=disabled",
    description: "上游无法翻译 disabled，删掉整个 thinking 字段后语义等价；只匹配 type=disabled，enabled 原样保留",
    params: &[],
};

pub struct ThinkingDisable;

impl RewriteRule for ThinkingDisable {
    fn spec(&self) -> &'static RuleSpec {
        &SPEC
    }

    fn apply(&self, body: &mut Value, _params: &BTreeMap<String, String>) -> usize {
        let Some(obj) = body.as_object_mut() else {
            return 0;
        };

        let is_disabled = obj
            .get("thinking")
            .and_then(|thinking| thinking.get("type"))
            .and_then(Value::as_str)
            == Some("disabled");

        if !is_disabled {
            return 0;
        }

        obj.remove("thinking");
        1
    }
}
