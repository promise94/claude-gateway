use gateway_core::config::{validate_rule_config, RuleConfig};
use gateway_core::rewrite::thinking_disable::ThinkingDisable;
use gateway_core::rewrite::tool_reference::ToolReferenceDowngrade;
use gateway_core::rewrite::{Registry, RewriteRule};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn no_params() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn enabled() -> RuleConfig {
    RuleConfig {
        enabled: true,
        params: BTreeMap::new(),
    }
}

fn disabled() -> RuleConfig {
    RuleConfig {
        enabled: false,
        params: BTreeMap::new(),
    }
}

fn tool_result_body(items: Value) -> Value {
    json!({
        "model": "claude-opus-5",
        "messages": [
            { "role": "user", "content": "开始" },
            {
                "role": "user",
                "content": [
                    { "type": "tool_result", "tool_use_id": "t1", "content": items }
                ]
            }
        ]
    })
}

#[test]
fn strips_thinking_disabled() {
    let mut body = json!({
        "model": "claude-opus-5",
        "thinking": { "type": "disabled" },
        "messages": []
    });

    let hits = ThinkingDisable.apply(&mut body, &no_params());

    assert_eq!(hits, 1);
    assert!(body.get("thinking").is_none());
}

#[test]
fn keeps_thinking_enabled_untouched() {
    let mut body = json!({
        "model": "claude-opus-5",
        "thinking": { "type": "enabled", "budget_tokens": 4096 },
        "messages": []
    });

    let hits = ThinkingDisable.apply(&mut body, &no_params());

    assert_eq!(hits, 0);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 4096);
}

#[test]
fn ignores_body_without_thinking() {
    let mut body = json!({ "model": "claude-opus-5", "messages": [] });

    let hits = ThinkingDisable.apply(&mut body, &no_params());

    assert_eq!(hits, 0);
    assert_eq!(body["model"], "claude-opus-5");
}

#[test]
fn downgrades_tool_reference_to_text() {
    let mut body = tool_result_body(json!([
        { "type": "tool_reference", "tool_name": "WebFetch" },
        { "type": "tool_reference", "tool_name": "WebSearch" }
    ]));

    let hits = ToolReferenceDowngrade.apply(&mut body, &no_params());

    assert_eq!(hits, 2);
    let content = &body["messages"][1]["content"][0]["content"];
    let items = content.as_array().expect("content 应为数组");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "text");
    let text = items[0]["text"].as_str().unwrap();
    assert!(text.contains("WebFetch"));
    assert!(text.contains("WebSearch"));
    // 原始类型已不存在
    assert!(!body.to_string().contains("tool_reference"));
}

#[test]
fn keeps_other_content_alongside_tool_reference() {
    let mut body = tool_result_body(json!([
        { "type": "text", "text": "先说这句" },
        { "type": "tool_reference", "tool_name": "Figma" }
    ]));

    let hits = ToolReferenceDowngrade.apply(&mut body, &no_params());

    assert_eq!(hits, 1);
    let items = body["messages"][1]["content"][0]["content"]
        .as_array()
        .unwrap();
    // 原有 text 保留在前，降级说明追加在后
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["type"], "text");
    assert_eq!(items[0]["text"].as_str().unwrap(), "先说这句");
    assert_eq!(items[1]["type"], "text");
    assert!(items[1]["text"].as_str().unwrap().contains("Figma"));
}

#[test]
fn leaves_clean_tool_result_untouched() {
    let mut body = tool_result_body(json!([
        { "type": "text", "text": "结果在这里" }
    ]));
    let before = body.clone();

    let hits = ToolReferenceDowngrade.apply(&mut body, &no_params());

    assert_eq!(hits, 0);
    assert_eq!(body, before);
}

#[test]
fn custom_template_takes_effect() {
    let mut body = tool_result_body(json!([
        { "type": "tool_reference", "tool_name": "WebFetch" }
    ]));
    let params = BTreeMap::from([("text_template".to_string(), "TOOLS={names}".to_string())]);

    let hits = ToolReferenceDowngrade.apply(&mut body, &params);

    assert_eq!(hits, 1);
    let text = body["messages"][1]["content"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(text, "TOOLS=WebFetch");
}

#[test]
fn rejects_template_without_placeholder() {
    let config = RuleConfig {
        enabled: true,
        params: BTreeMap::from([("text_template".to_string(), "没有占位符".to_string())]),
    };

    let error = validate_rule_config(&config).expect_err("缺 {names} 应被拒");
    assert!(error.contains("{names}"), "错误信息应指出缺少占位符");
}

#[test]
fn registry_respects_switch() {
    let registry = Registry::with_default_rules();

    let mut body = json!({
        "model": "claude-opus-5",
        "thinking": { "type": "disabled" },
        "messages": [{
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "t1",
                "content": [{ "type": "tool_reference", "tool_name": "WebFetch" }]
            }]
        }]
    });

    // 全部关闭
    let summary = registry.apply(&mut body, |_| disabled());
    assert!(summary.is_empty());
    assert_eq!(body["thinking"]["type"], "disabled");

    // 全部开启
    let summary = registry.apply(&mut body, |_| enabled());
    assert_eq!(summary.total_hits, 2);
    assert_eq!(summary.per_rule.len(), 2);
    assert!(body.get("thinking").is_none());
    assert!(!body.to_string().contains("tool_reference"));
}

#[test]
fn registry_accumulates_stats() {
    let registry = Registry::with_default_rules();

    for _ in 0..3 {
        let mut body = json!({ "thinking": { "type": "disabled" }, "messages": [] });
        registry.apply(&mut body, |_| enabled());
    }

    let stats = registry.stats(|_| enabled());
    let thinking = stats
        .iter()
        .find(|stat| stat.id == "thinking_disable")
        .expect("应存在 thinking_disable 规则");
    assert_eq!(thinking.hits, 3);
    assert!(thinking.enabled);
}

#[test]
fn stats_expose_param_defaults() {
    let registry = Registry::with_default_rules();
    let stats = registry.stats(|_| enabled());

    let tool = stats
        .iter()
        .find(|stat| stat.id == "tool_reference")
        .expect("应存在 tool_reference 规则");
    assert_eq!(tool.param_specs.len(), 1);
    assert_eq!(tool.param_specs[0].key, "text_template");
    assert_eq!(tool.param_specs[0].required_placeholder, Some("{names}"));
    // 未显式配置时 params 为空，由 ParamSpec.default 兜底
    assert!(tool.params.is_empty());
}
