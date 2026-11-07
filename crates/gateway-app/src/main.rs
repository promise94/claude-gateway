//! 桌面端入口：一个进程里既跑网关，也跑这个界面。
//!
//! 关窗口不退出（窗口只是隐藏），网关继续转发；真正退出走托盘菜单。

mod commands;
mod ctx;
mod tray;

use ctx::AppCtx;
use tauri::Manager;

fn main() {
    let ctx = match AppCtx::bootstrap() {
        Ok(ctx) => ctx,
        Err(error) => {
            eprintln!("claude-gateway 启动失败：{error:#}");
            std::process::exit(1);
        }
    };

    let app = tauri::Builder::default()
        .manage(ctx)
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::status,
            commands::logs,
            commands::log_files,
            commands::log_file,
            commands::log_level,
            commands::log_level_set,
            commands::export_logs,
            commands::open_log_dir,
            commands::config,
            commands::upstream,
            commands::upstream_set,
            commands::rule_update,
            commands::rule_save,
            commands::rule_discard,
        ])
        .setup(|app| {
            // 直接跑二进制（没有 .app 外壳）时，系统不会把它当常规前台应用，
            // 窗口会落在终端后面。显式设成 Regular 并抢一次焦点。
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular);

            app.state::<AppCtx>().spawn_server();
            tray::build(app.handle())?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 关窗 = 收起，别把转发一起停掉
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("tauri 构建失败");

    app.run(|app_handle, event| {
        // macOS 点击 Dock 图标走 applicationShouldHandleReopen。
        // 关窗只是 hide，不在这里把窗口 show 回来就永远出不来。
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            tray::show_main(app_handle);
        }

        // 其他平台没有 Reopen，显式消费参数避免未使用告警
        #[cfg(not(target_os = "macos"))]
        let _ = (app_handle, event);
    });
}
