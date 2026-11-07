//! claude-gateway
//!
//! 夹在 Claude Code（可经 cc-switch）与上游模型网关之间的兼容层。
//!
//! 存在的理由：客户端字段更新快、网关字段支持慢。每遇到一个网关无法翻译的
//! 字段，就在这里加一条 [`rewrite::RewriteRule`]，而不是去改客户端或等网关。

pub mod config;
pub mod config_store;
pub mod forward;
pub mod local_appender;
pub mod logging;
pub mod panel;
pub mod paths;
pub mod rewrite;
pub mod rules_store;
pub mod server;
pub mod stream_observer;
pub mod telemetry;

pub use config::{Config, RuleConfig};
pub use rewrite::{ParamSpec, Registry, RewriteSummary, RuleStat};
