//! 请求体改写规则
//!
//! 设计目标：上游网关的字段支持总是落后于 Claude Code 客户端。
//! 每条规则实现 [`RewriteRule`]：`spec()` 声明元数据与可配参数，
//! `apply()` 执行改写。参数由 `config.toml` 的 `[rules.<id>.params]` 提供，
//! 运行时可热更新，不需要重新编译。

pub mod thinking_disable;
pub mod tool_reference;

use crate::config::RuleConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// 一个规则参数的声明。面板据此渲染表单，加载时据此校验。
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    /// 参数键名，也是 TOML `params` 表里的键
    pub key: &'static str,
    /// 面向人的名称
    pub label: &'static str,
    /// 默认值
    pub default: &'static str,
    /// 模板中必须包含的占位符，如 `{names}`
    pub required_placeholder: Option<&'static str>,
}

/// 规则元数据
#[derive(Debug)]
pub struct RuleSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub params: &'static [ParamSpec],
}

/// 单条改写规则
pub trait RewriteRule: Send + Sync {
    /// 元数据与参数声明
    fn spec(&self) -> &'static RuleSpec;

    /// 原地改写请求体，返回本次命中次数
    fn apply(&self, body: &mut Value, params: &BTreeMap<String, String>) -> usize;
}

struct Slot {
    rule: Box<dyn RewriteRule>,
    hits: AtomicU64,
}

/// 规则注册表，按注册顺序执行
pub struct Registry {
    slots: Vec<Slot>,
}

/// 一次请求经过改写链之后的汇总
#[derive(Debug, Default, Clone)]
pub struct RewriteSummary {
    /// (规则 id, 本次命中次数)，只包含命中数大于 0 的规则
    pub per_rule: Vec<(&'static str, usize)>,
    pub total_hits: usize,
}

impl RewriteSummary {
    pub fn is_empty(&self) -> bool {
        self.total_hits == 0
    }
}

/// 单条规则的统计快照，供面板读取
#[derive(Debug, Clone)]
pub struct RuleStat {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub enabled: bool,
    pub hits: u64,
    pub params: BTreeMap<String, String>,
    pub param_specs: &'static [ParamSpec],
}

impl Registry {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// 内置规则集。新增规则在这里注册一行。
    pub fn with_default_rules() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(thinking_disable::ThinkingDisable));
        registry.register(Box::new(tool_reference::ToolReferenceDowngrade));
        registry
    }

    pub fn register(&mut self, rule: Box<dyn RewriteRule>) {
        self.slots.push(Slot {
            rule,
            hits: AtomicU64::new(0),
        });
    }

    /// 按 id 查规则元数据。未知 id 返回 None，用于拒绝非法配置。
    pub fn spec(&self, id: &str) -> Option<&'static RuleSpec> {
        self.slots
            .iter()
            .map(|slot| slot.rule.spec())
            .find(|spec| spec.id == id)
    }

    /// 所有已注册规则的 id，按注册顺序
    pub fn ids(&self) -> Vec<&'static str> {
        self.slots.iter().map(|slot| slot.rule.spec().id).collect()
    }

    /// 依次执行已启用的规则。`config_for` 返回该规则的当前配置。
    pub fn apply<F>(&self, body: &mut Value, config_for: F) -> RewriteSummary
    where
        F: Fn(&str) -> RuleConfig,
    {
        let mut summary = RewriteSummary::default();
        for slot in &self.slots {
            let spec = slot.rule.spec();
            let config = config_for(spec.id);
            if !config.enabled {
                continue;
            }
            let hits = slot.rule.apply(body, &config.params);
            if hits > 0 {
                slot.hits.fetch_add(hits as u64, Ordering::Relaxed);
                summary.per_rule.push((spec.id, hits));
                summary.total_hits += hits;
            }
        }
        summary
    }

    /// 统计快照
    pub fn stats<F>(&self, config_for: F) -> Vec<RuleStat>
    where
        F: Fn(&str) -> RuleConfig,
    {
        self.slots
            .iter()
            .map(|slot| {
                let spec = slot.rule.spec();
                let config = config_for(spec.id);
                RuleStat {
                    id: spec.id,
                    label: spec.label,
                    description: spec.description,
                    enabled: config.enabled,
                    hits: slot.hits.load(Ordering::Relaxed),
                    params: config.params,
                    param_specs: spec.params,
                }
            })
            .collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_default_rules()
    }
}
