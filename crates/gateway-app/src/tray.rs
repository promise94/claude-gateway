//! 托盘：窗口关掉之后，这里就是唯一的入口。

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Claude Gateway", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::with_id("main")
        // 模板图标：macOS 会按明暗主题自动反色。
        // 用原始 RGBA 而不是 PNG，省掉对图片解码特性的依赖，编译期就能确定尺寸。
        .icon(Image::new(include_bytes!("../icons/tray.rgba"), 44, 44))
        .icon_as_template(true)
        .tooltip("Claude Gateway")
        .menu(&menu)
        // 左键唤回窗口，右键才出菜单
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { .. }) {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// 唤回主窗口。托盘点击与 macOS 点击 Dock 图标共用这一条路径。
pub(crate) fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
