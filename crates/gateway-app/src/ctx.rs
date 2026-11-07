//! 应用上下文：网关与界面活在同一个进程里。
//!
//! 刻意不在 `main` 里 `?` 掉任何失败：端口被占用时窗口仍要打开并把原因显示出来，
//! 否则用户只会看到 app 闪退。

use gateway_core::config::Config;
use gateway_core::forward::Gateway;
use gateway_core::logging::{self, LogGuard};
use gateway_core::server::{self, AppState};
use gateway_core::telemetry;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct AppCtx {
    pub state: Arc<AppState>,
    pub config: Config,
    /// 端口绑定失败的原因。有值表示网关没起来，界面要把这句话显示出来。
    pub listen_error: Option<String>,
    listener: Mutex<Option<TcpListener>>,
    /// 必须活到进程结束，否则后台写盘线程被提前关闭，尾部日志丢失
    _log_guard: Mutex<Option<LogGuard>>,
}

impl AppCtx {
    pub fn bootstrap() -> anyhow::Result<Self> {
        let telemetry = telemetry::init_global();
        let config = Config::discover()?;
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
        // 上游通常是本机 cc-switch，走系统代理会被错误路由
        let client_builder = if config.use_system_proxy {
            client_builder
        } else {
            client_builder.no_proxy()
        };
        let client = client_builder.build()?;

        let listen = config.listen.clone();
        let upstream = config.upstream_base().to_string();
        let gateway = Gateway::new(client, config.clone());
        let state = Arc::new(AppState {
            gateway,
            telemetry,
            listen: listen.clone(),
        });

        // 先同步 bind：失败时能把原因带进窗口，而不是在后台任务里 panic
        let mut listen_error = None;
        let listener = match TcpListener::bind(&listen) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                Some(listener)
            }
            Err(error) => {
                listen_error = Some(format!("端口 {listen} 绑定失败：{error}"));
                None
            }
        };

        if let Some(error) = &listen_error {
            tracing::error!(%error, "网关未启动");
        } else {
            tracing::info!(%listen, %upstream, "claude-gateway 已启动（内嵌模式）");
            if !server::is_loopback(&listen) {
                tracing::warn!(%listen, "监听地址不是回环地址，控制面与面板均无鉴权");
            }
        }

        Ok(Self {
            state,
            config,
            listen_error,
            listener: Mutex::new(listener),
            _log_guard: Mutex::new(Some(log_guard)),
        })
    }

    /// 把已绑定的 listener 交给 tokio 起服务
    pub fn spawn_server(&self) {
        let Some(listener) = self.listener.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        let state = self.state.clone();
        let listen = self.config.listen.clone();

        tauri::async_runtime::spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::error!(%error, "listener 转换失败");
                    return;
                }
            };
            tracing::info!(%listen, "控制面: /_gateway/health , /_gateway/stats , /_panel");
            if let Err(error) = axum::serve(listener, server::router(state)).await {
                tracing::error!(%error, "网关服务异常退出");
            }
        });
    }
}
