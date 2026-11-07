//! 上游转发：改写请求体，透传并流式回传响应

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderName, Method, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::config::{validate_rule_config, Config, RuleConfig};
use crate::logging::{RequestTiming, RewriteStats};
use crate::rewrite::{Registry, RewriteSummary};
use crate::rules_store::{self, RuleState};
use crate::stream_observer::ObservedStream;
use crate::RuleStat;

/// 逐跳 header，不允许透传
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub struct Gateway {
    client: reqwest::Client,
    config: Config,
    /// 当前生效的上游地址。界面改完立即生效，同时写回配置文件。
    /// 单独存一份而不是每次都读 config：Config 是启动时加载的不可变快照。
    upstream: RwLock<String>,
    registry: Registry,
    /// 规则开关与参数。可在运行时修改，改完立即生效，落盘需显式调用。
    rules: Arc<RwLock<RuleState>>,
}

/// 上游探测结果。从控制台手动触发，不参与转发路径。
pub struct UpstreamProbe {
    pub reachable: bool,
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub error: Option<String>,
}

impl Gateway {
    pub fn new(client: reqwest::Client, config: Config) -> Self {
        let rules = RuleState::new(config.rules.clone(), config.source.clone());
        let upstream = config.upstream_base().to_string();
        Self {
            client,
            config,
            upstream: RwLock::new(upstream),
            registry: Registry::with_default_rules(),
            rules: Arc::new(RwLock::new(rules)),
        }
    }

    /// 当前生效的上游地址（不带结尾斜杠）
    pub fn upstream(&self) -> String {
        self.upstream.read().expect("上游地址锁被投毒").clone()
    }

    /// 改上游地址。下一个请求即生效，已在传输中的流不受影响。
    pub fn set_upstream(&self, url: &str) {
        let normalized = url.trim().trim_end_matches('/').to_string();
        *self.upstream.write().expect("上游地址锁被投毒") = normalized;
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn max_body_bytes(&self) -> usize {
        self.config.max_body_bytes
    }

    pub fn rule_stats(&self) -> Vec<RuleStat> {
        let rules = self.rules.read().expect("规则状态锁被投毒");
        self.registry.stats(|id| rules.config(id))
    }

    /// 是否有未落盘的规则改动
    pub fn rules_dirty(&self) -> bool {
        self.rules
            .read()
            .map(|rules| rules.is_dirty())
            .unwrap_or(false)
    }

    /// 改一条规则的开关与参数。只作用于内存，调用 [`Self::save_rules`] 才落盘。
    pub fn update_rule(&self, id: &str, config: RuleConfig) -> anyhow::Result<RuleStat> {
        let spec = self
            .registry
            .spec(id)
            .ok_or_else(|| anyhow::anyhow!("未知规则: {id}"))?;

        let known: Vec<&str> = spec.params.iter().map(|param| param.key).collect();
        if let Some(unknown) = config
            .params
            .keys()
            .find(|key| !known.contains(&key.as_str()))
        {
            anyhow::bail!("规则 {id} 没有参数 {unknown}");
        }
        validate_rule_config(&config).map_err(|error| anyhow::anyhow!(error))?;

        {
            let mut rules = self.rules.write().expect("规则状态锁被投毒");
            rules.set(id, config);
        }

        self.rule_stats()
            .into_iter()
            .find(|stat| stat.id == id)
            .ok_or_else(|| anyhow::anyhow!("规则 {id} 不存在"))
    }

    /// 把当前规则配置写回 config.toml。写前备份，保留注释与顺序。
    pub fn save_rules(&self) -> anyhow::Result<std::path::PathBuf> {
        let mut rules = self.rules.write().expect("规则状态锁被投毒");
        let path = rules
            .source()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("配置不是从文件加载的，无法落盘"))?;
        let snapshot = rules.snapshot();

        let backup = path.with_extension("toml.bak");
        std::fs::copy(&path, &backup).map_err(|error| anyhow::anyhow!("备份配置失败: {error}"))?;

        rules_store::persist(&path, &snapshot)?;
        rules.mark_clean();
        Ok(path)
    }

    /// 丢弃未落盘的改动，从配置文件重新读取
    pub fn discard_rules(&self) -> anyhow::Result<()> {
        let mut rules = self.rules.write().expect("规则状态锁被投毒");
        let path = rules
            .source()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("配置不是从文件加载的，无法回读"))?;
        let config = Config::load(&path)?;
        rules.replace(config.rules);
        Ok(())
    }

    /// 探测上游可达性。
    ///
    /// 只发 HEAD，不进入推理，所以不会消耗生成额度；目的仅是区分
    /// 「连不上上游（DNS / TLS / 端口）」和「上游返回了错误状态码」。
    pub async fn probe(&self) -> UpstreamProbe {
        let url = self.upstream();
        let started = Instant::now();
        let elapsed = |started: Instant| started.elapsed().as_millis() as u64;

        match tokio::time::timeout(Duration::from_secs(5), self.client.head(&url).send()).await {
            Err(_) => UpstreamProbe {
                reachable: false,
                status: None,
                latency_ms: elapsed(started),
                error: Some("探测超时（5s）".to_string()),
            },
            Ok(Err(error)) => UpstreamProbe {
                reachable: false,
                status: None,
                latency_ms: elapsed(started),
                error: Some(error.to_string()),
            },
            Ok(Ok(response)) => UpstreamProbe {
                reachable: true,
                status: Some(response.status().as_u16()),
                latency_ms: elapsed(started),
                error: None,
            },
        }
    }

    /// 处理一个请求：改写 → 转发 → 流式回传
    ///
    /// `rid` 由访问日志中间件生成，只用于把日志串起来，不写进上游请求头。
    pub async fn handle(
        &self,
        method: Method,
        path_and_query: &str,
        headers: HeaderMap,
        body: Bytes,
        rid: &str,
    ) -> Response {
        let (body, summary) = self.rewrite_body(&headers, body, rid);
        let url = format!("{}{}", self.upstream(), path_and_query);

        let mut request = self.client.request(method, &url);
        for (name, value) in headers.iter() {
            if should_skip_request_header(name) {
                continue;
            }
            request = request.header(name, value);
        }
        // 显式要求上游不压缩，这样回传时无需处理 content-encoding
        request = request.header(header::ACCEPT_ENCODING, "identity");
        request = request.body(body);

        // 只测到响应头到达为止（首字节时间）。流式 body 的时长由客户端决定，
        // 计入这里会让 upstream_ms 变成"整个对话的时长"，失去参考价值。
        let upstream_started = Instant::now();
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(rid = %rid, url = %url, %error, "上游请求失败");
                return attach(
                    error_response(
                        StatusCode::BAD_GATEWAY,
                        &format!("upstream request failed: {error}"),
                    ),
                    upstream_started.elapsed().as_millis() as u64,
                    summary,
                );
            }
        };
        let upstream_ms = upstream_started.elapsed().as_millis() as u64;

        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let mut builder = Response::builder().status(status);
        for (name, value) in response.headers().iter() {
            if should_skip_response_header(name) {
                continue;
            }
            builder = builder.header(name, value);
        }

        // 流式回传，不做缓冲，SSE 才能逐块到达客户端。
        // 包一层观测器：访问日志只到响应头，流是否收完只能在这里判断。
        let is_sse = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("text/event-stream"))
            .unwrap_or(false);
        let stream = ObservedStream::new(response.bytes_stream(), path_and_query, is_sse, rid);
        let built = builder
            .body(Body::from_stream(stream))
            .unwrap_or_else(|error| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("build response failed: {error}"),
                )
            });

        attach(built, upstream_ms, summary)
    }

    /// 按规则改写请求体。非 JSON 或解析失败时原样返回，并带上空的命中汇总。
    fn rewrite_body(&self, headers: &HeaderMap, body: Bytes, rid: &str) -> (Bytes, RewriteSummary) {
        if body.is_empty() {
            return (body, RewriteSummary::default());
        }
        let is_json = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("json"))
            .unwrap_or(false);
        if !is_json {
            return (body, RewriteSummary::default());
        }

        let mut value: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(rid = %rid, %error, "请求体不是合法 JSON，跳过改写");
                return (body, RewriteSummary::default());
            }
        };

        // 只在校改期间持读锁，序列化与转发不占锁
        let summary = {
            let rules = self.rules.read().expect("规则状态锁被投毒");
            self.registry.apply(&mut value, |id| rules.config(id))
        };
        if summary.is_empty() {
            return (body, RewriteSummary::default());
        }

        tracing::debug!(
            rid = %rid,
            rules = ?summary.per_rule,
            total = summary.total_hits,
            "请求体已改写"
        );

        match serde_json::to_vec(&value) {
            Ok(bytes) => (Bytes::from(bytes), summary),
            Err(error) => {
                tracing::warn!(rid = %rid, %error, "改写后序列化失败，回退原始请求体");
                (body, RewriteSummary::default())
            }
        }
    }
}

/// 把耗时与命中情况挂到响应上，供访问日志中间件读取
fn attach(mut response: Response, upstream_ms: u64, summary: RewriteSummary) -> Response {
    response
        .extensions_mut()
        .insert(RequestTiming { upstream_ms });
    response.extensions_mut().insert(RewriteStats {
        per_rule: summary.per_rule,
    });
    response
}

fn should_skip_request_header(name: &HeaderName) -> bool {
    let name = name.as_str();
    HOP_BY_HOP.contains(&name) || matches!(name, "host" | "content-length" | "accept-encoding")
}

fn should_skip_response_header(name: &HeaderName) -> bool {
    let name = name.as_str();
    HOP_BY_HOP.contains(&name) || matches!(name, "content-length")
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let payload = serde_json::json!({
        "type": "error",
        "error": { "type": "api_error", "message": format!("claude-gateway: {message}") }
    });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .expect("构造错误响应不应失败")
}
