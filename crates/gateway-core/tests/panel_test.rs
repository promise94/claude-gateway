//! 控制台路由的端到端测试：不启服务，直接打 Router。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use gateway_core::config::Config;
use gateway_core::forward::Gateway;
use gateway_core::server::AppState;
use gateway_core::telemetry::Telemetry;
use gateway_core::{panel, telemetry};
use std::sync::Arc;
use tower::ServiceExt;

fn app(telemetry: Arc<Telemetry>) -> Router {
    let state = Arc::new(AppState {
        gateway: Gateway::new(reqwest::Client::new(), Config::default()),
        telemetry,
        listen: "127.0.0.1:0".to_string(),
    });
    panel::router().with_state(state)
}

async fn get(router: Router, uri: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("构造请求"),
        )
        .await
        .expect("路由不应返回错误");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("读取响应体");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn post_json(router: Router, uri: &str, payload: serde_json::Value) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .expect("构造请求"),
        )
        .await
        .expect("路由不应返回错误");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("读取响应体");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// 带 source 的配置，用于验证落盘与回读
/// 测试并行执行，`SystemTime::now()` 在同一纳秒内可能取到相同值；
/// 追加自增序号，避免两个用例写同一个临时配置文件互相踩。
static FIXTURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unique_config_path() -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时间早于 1970")
        .as_nanos();
    let seq = FIXTURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("cg-panel-rules-{stamp}-{seq}.toml"))
}

fn app_with_file() -> (std::path::PathBuf, Router) {
    let path = unique_config_path();
    std::fs::write(
        &path,
        "# 顶部注释\nlisten = \"127.0.0.1:0\"\n\n[rules.tool_reference]\nenabled = true\n",
    )
    .expect("写临时配置");

    let config = Config::load(&path).expect("加载临时配置");
    let state = Arc::new(AppState {
        gateway: Gateway::new(reqwest::Client::new(), config),
        telemetry: Arc::new(Telemetry::new()),
        listen: "127.0.0.1:0".to_string(),
    });
    (path, panel::router().with_state(state))
}

#[tokio::test]
async fn panel_page_is_embedded_and_served() {
    let (status, body) = get(app(Arc::new(Telemetry::new())), "/_panel").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("claude-gateway 控制台"),
        "页面应内嵌在二进制里"
    );
    assert!(body.contains("/_panel/api/status"), "页面要能取到数据");
}

#[tokio::test]
async fn status_endpoint_reports_runtime_facts() {
    let telemetry = Arc::new(Telemetry::new());
    telemetry.record_request(telemetry::RequestRecord {
        at: "2026-11-03 20:00:00.000".into(),
        rid: "00a1f3".into(),
        method: "POST".into(),
        path: "/v1/messages".into(),
        status: 200,
        elapsed_ms: 120,
        upstream_ms: 100,
        rewritten: "thinking_disable:1".into(),
    });

    let (status, body) = get(app(telemetry), "/_panel/api/status").await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    assert_eq!(json["counters"]["requests"], 1);
    assert_eq!(json["requests"][0]["rewritten"], "thinking_disable:1");
    assert_eq!(json["listen"], "127.0.0.1:0");
    // 两条默认规则都要出现在状态里
    assert_eq!(json["rules"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn logs_endpoint_filters_by_level() {
    let telemetry = Arc::new(Telemetry::new());
    telemetry.push_log("2026-11-03 20:00:00.000  INFO 起".into());
    telemetry.push_log("2026-11-03 20:00:01.000  WARN 停".into());

    let (status, body) = get(app(telemetry), "/_panel/api/logs?level=warn").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    let lines = json["lines"].as_array().expect("lines 应为数组");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].as_str().unwrap().contains("停"));
}

#[tokio::test]
async fn logfile_rejects_names_outside_the_rotation_pattern() {
    let telemetry = Arc::new(Telemetry::new());
    telemetry.set_paths(Some("/tmp".to_string()), None);

    let (status, body) = get(app(telemetry), "/_panel/api/logfile?name=../etc/passwd").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "越界文件名必须被拒");
    assert!(body.contains("文件名不合法"));
}

#[tokio::test]
async fn logfile_reports_missing_file_without_panicking() {
    let telemetry = Arc::new(Telemetry::new());
    telemetry.set_paths(Some("/tmp".to_string()), None);

    let (status, _) = get(
        app(telemetry),
        "/_panel/api/logfile?name=gateway.1999-01-01.log",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn config_endpoint_never_leaks_credentials() {
    let (status, body) = get(app(Arc::new(Telemetry::new())), "/_panel/api/config").await;
    assert_eq!(status, StatusCode::OK);
    // 配置里本来就没有凭证字段，这条断言防止以后有人手滑加进去
    for key in ["token", "key", "secret", "authorization"] {
        assert!(
            !body.to_lowercase().contains(key),
            "控制台不应出现 {key} 相关字段"
        );
    }
}

#[tokio::test]
async fn rule_update_rejects_unknown_param() {
    let (path, router) = app_with_file();

    let (status, body) = post_json(
        router,
        "/_panel/api/rules",
        serde_json::json!({
            "id": "thinking_disable",
            "enabled": true,
            "params": { "nope": "x" }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "未知参数必须被拒");
    assert!(body.contains("没有参数"), "错误信息应说明是哪个参数");

    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn rule_update_takes_effect_and_persists() {
    let (path, router) = app_with_file();

    let (status, body) = post_json(
        router.clone(),
        "/_panel/api/rules",
        serde_json::json!({ "id": "tool_reference", "enabled": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    assert_eq!(json["rule"]["enabled"], false);
    assert_eq!(json["persisted"], true, "改完应立即写进配置文件");
    assert_eq!(json["dirty"], false, "自动落盘后不应留未落盘标记");

    // 界面没有保存按钮，文件必须当场同步
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("enabled = false"), "文件应同步改写");
    assert!(on_disk.contains("# 顶部注释"), "改写时注释应保留");

    // status 也要反映新状态
    let (status, body) = get(router, "/_panel/api/status").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    let rule = json["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rule| rule["id"] == "tool_reference")
        .expect("应存在 tool_reference");
    assert_eq!(rule["enabled"], false);
    assert_eq!(json["rules_dirty"], false);

    let _ = std::fs::remove_file(path.with_extension("toml.bak"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn rule_save_persists_and_keeps_comments() {
    let (path, router) = app_with_file();

    post_json(
        router.clone(),
        "/_panel/api/rules",
        serde_json::json!({
            "id": "tool_reference",
            "enabled": false,
            "params": { "text_template": "已加载 {names}" }
        }),
    )
    .await;

    let (status, body) = post_json(router, "/_panel/api/rules/save", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    assert_eq!(json["saved"], true);
    assert_eq!(json["dirty"], false, "保存后不应再有未落盘标记");

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("# 顶部注释"), "注释应保留");
    assert!(written.contains("enabled = false"), "开关应写入");
    assert!(written.contains("已加载 {names}"), "参数应写入");

    // 备份文件也应存在
    let backup = path.with_extension("toml.bak");
    assert!(backup.exists(), "保存前应备份原文件");

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(backup);
}

#[tokio::test]
async fn rule_discard_reloads_from_file() {
    let (path, router) = app_with_file();

    // 自动落盘后文件里是 false
    post_json(
        router.clone(),
        "/_panel/api/rules",
        serde_json::json!({ "id": "tool_reference", "enabled": false }),
    )
    .await;

    // 绕过程序手工改文件，模拟用户直接编辑配置
    let raw = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, raw.replace("enabled = false", "enabled = true")).unwrap();

    let (status, body) =
        post_json(router, "/_panel/api/rules/discard", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    assert_eq!(json["dirty"], false);
    let rule = json["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rule| rule["id"] == "tool_reference")
        .expect("应存在 tool_reference");
    assert_eq!(rule["enabled"], true, "应重新读到文件里手工改过的值");

    let _ = std::fs::remove_file(path.with_extension("toml.bak"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn upstream_update_persists_and_takes_effect() {
    let (path, router) = app_with_file();

    let (status, body) = post_json(
        router.clone(),
        "/_panel/api/upstream",
        serde_json::json!({ "upstream": "http://127.0.0.1:19999/" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    assert_eq!(
        json["upstream"], "http://127.0.0.1:19999",
        "尾部斜杠应被去掉"
    );
    assert_eq!(json["persisted"], true);

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("upstream = \"http://127.0.0.1:19999\""),
        "应写回配置文件：{written}"
    );

    // 配置接口读运行时值，应立即反映新地址，不用重启
    let (status, body) = get(router, "/_panel/api/config").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).expect("应为 JSON");
    assert_eq!(json["upstream"], "http://127.0.0.1:19999");

    let _ = std::fs::remove_file(path.with_extension("toml.bak"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn upstream_update_rejects_invalid_url() {
    let (path, router) = app_with_file();

    let (status, body) = post_json(
        router,
        "/_panel/api/upstream",
        serde_json::json!({ "upstream": "127.0.0.1:15721" }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("地址格式不正确"), "应说明格式问题：{body}");

    let _ = std::fs::remove_file(path);
}
