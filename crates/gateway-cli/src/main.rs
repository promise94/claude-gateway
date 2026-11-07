use gateway_core::{forward::Gateway, logging, server, telemetry, Config};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 遥测要先于日志初始化：日志会写一份进它的环形缓冲，控制台读的就是这份
    let telemetry = telemetry::init_global();
    let config = Config::discover()?;

    // 日志必须在配置加载之后初始化：级别和落盘目录都来自 config。
    // guard 要活到进程结束，否则后台写盘线程会被提前关闭，尾部日志丢失。
    let log_guard = logging::init(&config, telemetry.clone())?;
    telemetry.set_paths(
        Some(log_guard.log_dir.display().to_string()),
        config
            .source
            .as_ref()
            .map(|path| path.display().to_string()),
    );

    tracing::info!(origin = %config.origin.describe(), "配置来源");
    tracing::info!(dir = %log_guard.log_dir.display(), "日志落盘目录");
    if matches!(config.origin, gateway_core::paths::ConfigOrigin::Created(_)) {
        tracing::warn!(
            upstream = %config.upstream,
            "首次启动使用默认上游地址，请在设置页确认"
        );
    }

    let client_builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90));
    // 不设总超时：流式响应的时长由客户端决定

    let client_builder = if config.use_system_proxy {
        client_builder
    } else {
        // 上游默认是本机 cc-switch，走系统代理会被错误路由，
        // 且请求行会变成 absolute-form，部分上游直接拒绝
        client_builder.no_proxy()
    };
    let client = client_builder.build()?;

    let listen = config.listen.clone();
    let upstream = config.upstream_base().to_string();

    let gateway = Gateway::new(client, config);
    let state = Arc::new(server::AppState {
        gateway,
        telemetry,
        listen: listen.clone(),
    });
    let app = server::router(state);

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(%listen, %upstream, "claude-gateway 已启动");
    tracing::info!("控制面: http://{listen}/_gateway/health , /_gateway/stats");
    tracing::info!("控制台: http://{listen}/_panel");

    // 控制台没有鉴权。绑在回环地址上时只有本机可访问，绑到别处就得提醒一句。
    if !server::is_loopback(&listen) {
        tracing::warn!(
            %listen,
            "监听地址不是回环地址，控制台 /_panel 无鉴权，请确认所在网络可信"
        );
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("收到退出信号，正在关闭");
}
