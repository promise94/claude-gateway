//! HTTP 入口：控制面（health / stats）+ 数据面（兜底转发）

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::forward::Gateway;
use crate::telemetry::Telemetry;

pub struct AppState {
    pub gateway: Gateway,
    pub telemetry: Arc<Telemetry>,
    /// 实际监听地址，面板上展示
    pub listen: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/_gateway/health", get(health))
        .route("/_gateway/stats", get(stats))
        .merge(crate::panel::router())
        // 其余路径全部转发给上游，不区分 /v1/messages 等具体端点
        .fallback(proxy)
        // 访问日志必须包在最外层，才能测到包含转发在内的完整耗时
        .layer(axum::middleware::from_fn(crate::logging::access_log))
        .with_state(state)
}

/// 监听地址是否只对本机开放。
///
/// 控制台没有任何鉴权，绑到回环之外等于把运行状态和日志暴露给整个网段，
/// 启动时要能提醒一句。
pub fn is_loopback(listen: &str) -> bool {
    let host = listen
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(listen);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "upstream": state.gateway.upstream(),
    }))
}

async fn stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rules: Vec<Value> = state
        .gateway
        .rule_stats()
        .into_iter()
        .map(|stat| {
            json!({
                "id": stat.id,
                "label": stat.label,
                "enabled": stat.enabled,
                "hits": stat.hits,
            })
        })
        .collect();
    Json(json!({ "rules": rules }))
}

async fn proxy(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();

    let body = match axum::body::to_bytes(body, state.gateway.max_body_bytes()).await {
        Ok(bytes) => bytes,
        Err(_) => {
            tracing::warn!(
                limit = state.gateway.max_body_bytes(),
                path = %parts.uri.path(),
                "请求体超过上限，已拒绝"
            );
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({
                    "type": "error",
                    "error": {
                        "type": "invalid_request_error",
                        "message": "claude-gateway: 请求体超过 max_body_bytes 上限"
                    }
                })),
            )
                .into_response();
        }
    };

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    // 正常路径一定有 ID；直接调用 router 的测试或异常组装可能没有，用 "-" 兜底
    let rid = parts
        .extensions
        .get::<crate::logging::RequestId>()
        .map(|id| id.0.clone())
        .unwrap_or_else(|| "-".to_string());

    state
        .gateway
        .handle(parts.method, &path_and_query, parts.headers, body, &rid)
        .await
}
