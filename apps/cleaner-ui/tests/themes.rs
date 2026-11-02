mod common;

use cleaner_ui::{
    configured_app,
    controller::ControllerConfig,
    theme::{self, ThemeMode},
};
use common::UiFixture;
use nsh_core::{MountItem, prelude::*};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

static CONFIG: OnceLock<ControllerConfig> = OnceLock::new();

#[component]
fn FixtureApp() -> impl IntoElement {
    configured_app(CONFIG.get().unwrap().clone())
}

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<FixtureApp>());
    config.viewport = nsh_core::Size::new(Points(900.0), Points(620.0));
    config.ui_sets = vec![nsh_base::registry(), cleaner_ui::__nsh_native_module()];
    Headless::start(config).unwrap()
}

#[test]
fn themes_recolor_live_views_preserve_config_and_survive_restart() {
    let fixture = UiFixture::create();
    std::fs::write(
        fixture.state.join("config.toml"),
        "auto_clean = true\nbackup_quota_bytes = 123456\n",
    )
    .unwrap();
    CONFIG
        .set(ControllerConfig {
            initial_scan_root: domain::normalize_path(fixture.tree.to_str().unwrap()).unwrap(),
            data_root: fixture.state.clone(),
        })
        .unwrap();
    for n in 0..2000 {
        std::fs::write(fixture.tree.join(format!("stress-{n}.tmp")), b"sample").unwrap();
    }
    let mut app = start();
    let colors = Arc::new(Mutex::new(Vec::new()));
    let observed = colors.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::ViewStyle { style, .. } = item {
                observed.lock().unwrap().push(style.background_color);
            }
        }
    });
    app.wait_for("label:设置").unwrap();
    app.press("label:设置").unwrap();
    // Do not wait for persistence: exercise clicks while scan progress and saves arrive.
    for n in 0..90 {
        let mode = ThemeMode::ALL[n % ThemeMode::ALL.len()];
        app.press(&format!("label:{}主题", mode.label())).unwrap();
    }
    app.wait_until(
        "scan survives rapid theme changes",
        Duration::from_secs(30),
        |screen| {
            screen
                .texts()
                .iter()
                .any(|text| text.contains("扫描完成") && text.contains("2004 个文件"))
        },
    )
    .unwrap();
    for mode in [
        ThemeMode::Mint,
        ThemeMode::Dark,
        ThemeMode::Light,
        ThemeMode::Mint,
    ] {
        app.press(&format!("label:{}主题", mode.label())).unwrap();
        let expected: nsh_core::Color = rgb(theme::BACKGROUND.value(mode)).into();
        app.wait_until(
            "theme background updates without restart",
            Duration::from_secs(5),
            |_| colors.lock().unwrap().contains(&expected),
        )
        .unwrap();
        colors.lock().unwrap().clear();
        assert!(!cleaner_ui::request_window_close(
            nsh_platform_windows::windows::Win32::Foundation::HWND(1 as _),
        ));
        app.wait_for("label:确认退出 Rusten？").unwrap();
        app.wait_for("label:确认退出").unwrap();
        let panel: nsh_core::Color = rgb(theme::PANEL.value(mode)).into();
        app.wait_until(
            "exit dialog follows the active theme",
            Duration::from_secs(5),
            |_| colors.lock().unwrap().contains(&panel),
        )
        .unwrap();
        app.press("label:取消").unwrap();
        app.wait_until(
            "exit cancellation keeps the application open",
            Duration::from_secs(5),
            |screen| screen.count("label:确认退出").unwrap() == 0,
        )
        .unwrap();
        if std::fs::read_to_string(fixture.state.join("config.toml"))
            .unwrap()
            .contains("theme =")
        {
            app.wait_until("theme persisted", Duration::from_secs(5), |_| {
                let config = std::fs::read_to_string(fixture.state.join("config.toml")).unwrap();
                config.contains(&format!("theme = \"{}\"", mode.key()))
            })
            .unwrap();
        }
        for page in ["首页", "C盘专清", "空间分析", "垃圾清理", "大文件", "设置"]
        {
            app.press(&format!("label:{page}")).unwrap();
            app.wait_for("label:扫描与清理工具栏").unwrap();
        }
    }
    drop(app);
    let saved = std::fs::read_to_string(fixture.state.join("config.toml")).unwrap();
    assert!(saved.contains("theme = \"mint\""));
    assert!(saved.contains("auto_clean = true"));
    assert!(saved.contains("backup_quota_bytes = 123456"));
    let mut app = start();
    app.wait_for("label:设置").unwrap();
    app.press("label:设置").unwrap();
    let mode = ThemeMode::from_key(&application::ConfigService::load_toml(&saved).config.theme);
    assert_eq!(mode, ThemeMode::Mint);
    assert!(app.screen().texts().contains(&"淡绿色"));
}
