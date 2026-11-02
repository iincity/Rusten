mod common;

use cleaner_ui::{configured_app, controller::ControllerConfig};
use common::UiFixture;
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use std::sync::OnceLock;

static CONFIG: OnceLock<ControllerConfig> = OnceLock::new();

#[component]
fn FixtureApp() -> impl IntoElement {
    configured_app(CONFIG.get().unwrap().clone())
}

#[test]
fn primary_scan_cleanup_actions_and_estimate_remain_visible_on_every_page() {
    let fixture = UiFixture::create();
    CONFIG
        .set(ControllerConfig {
            initial_scan_root: domain::normalize_path(fixture.tree.to_str().unwrap()).unwrap(),
            data_root: fixture.state.clone(),
        })
        .unwrap();
    for (width, height) in [(900.0, 620.0), (1180.0, 760.0), (1600.0, 1000.0)] {
        let mut config = EngineConfig::new(Root::of::<FixtureApp>());
        config.viewport = nsh_core::Size::new(Points(width), Points(height));
        config.ui_sets = vec![nsh_base::registry(), cleaner_ui::__nsh_native_module()];
        let mut app = Headless::start(config).unwrap();
        app.wait_for("label:首页").unwrap();
        app.wait_for("label:重新扫描").unwrap();
        assert_eq!(app.screen().count("label:恢复").unwrap(), 0);
        assert_eq!(app.screen().count("label:自动清理加速").unwrap(), 0);
        for page in ["首页", "C盘专清", "空间分析", "垃圾清理", "大文件", "设置"]
        {
            app.press(&format!("label:{page}")).unwrap();
            for label in ["扫描与清理工具栏", "预计可释放", "重新扫描", "立即清理"]
            {
                assert_eq!(
                    app.screen().count(&format!("label:{label}")).unwrap(),
                    1,
                    "{label} must stay visible on {page} at {width}x{height}"
                );
            }
        }
    }
    let mut config = EngineConfig::new(Root::of::<FixtureApp>());
    config.viewport = nsh_core::Size::new(Points(900.0), Points(620.0));
    config.ui_sets = vec![nsh_base::registry(), cleaner_ui::__nsh_native_module()];
    let mut app = Headless::start(config).unwrap();
    app.wait_for("label:设置").unwrap();
    app.press("label:设置").unwrap();
    app.wait_until(
        "toolbar scan and executable estimate",
        std::time::Duration::from_secs(30),
        |screen| {
            screen.texts().iter().any(|s| {
                s.starts_with("已选 ") && !s.starts_with("已选 0 ") && s.contains("扫描完成")
            }) && !screen.texts().contains(&"正在生成清理计划…")
        },
    )
    .unwrap();
    app.press("label:垃圾清理").unwrap();
    app.press("label:折叠 系统专清").unwrap();
    assert_eq!(app.screen().count("label:缓存文件").unwrap(), 0);
    app.press("label:折叠 系统专清").unwrap();
    app.wait_for("label:缓存文件").unwrap();
    app.press("label:空间分析").unwrap();
    app.wait_for("label:全部已选来源").unwrap();
    app.press("label:全部已选来源").unwrap();
    app.wait_until(
        "all selected sources remain inspectable",
        std::time::Duration::from_secs(5),
        |screen| {
            screen
                .texts()
                .iter()
                .any(|s| s.starts_with("全部已选来源 · 4 项"))
        },
    )
    .unwrap();
    let scope = format!("限定目录 {}", fixture.tree.join("nested").display());
    app.wait_for(&format!("label:{scope}")).unwrap();
    app.press(&format!("label:{scope}")).unwrap();
    app.wait_until(
        "directory scope limits estimate",
        std::time::Duration::from_secs(5),
        |screen| {
            screen
                .texts()
                .iter()
                .any(|text| text.starts_with("已选 1 项"))
        },
    )
    .unwrap();
    fixture.assert_owned_path(&fixture.tree.join("nested/readme.txt"));
    app.press("label:立即清理").unwrap();
    app.wait_until(
        "direct cleanup respects selected directory",
        std::time::Duration::from_secs(30),
        |screen| {
            assert_eq!(screen.count("label:确认清理").unwrap(), 0);
            screen
                .texts()
                .iter()
                .any(|s| s.starts_with("清理结束  ·  1/1 项"))
        },
    )
    .unwrap();
    assert!(!fixture.tree.join("nested/readme.txt").exists());
    assert!(fixture.tree.join("victim.bin").exists());
    assert!(fixture.tree.join("changed.bin").exists());
    assert!(fixture.tree.join("keep.bin").exists());
    app.press("label:空间分析").unwrap();
    app.wait_for(&format!("label:{scope}")).unwrap();
    app.press(&format!("label:{scope}")).unwrap();
    app.wait_until(
        "remaining files are selectable without rescanning",
        std::time::Duration::from_secs(30),
        |screen| {
            screen
                .texts()
                .iter()
                .any(|text| text.starts_with("已选 3 项") && text.contains("扫描完成"))
                && !screen.texts().contains(&"正在生成清理计划…")
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::read(fixture.tree.join("victim.bin")).unwrap(),
        b"restore-me"
    );
    app.press("label:垃圾清理").unwrap();
    app.press("label:清理所选项目").unwrap();
    app.wait_until(
        "toolbar cleanup executes owned plan",
        std::time::Duration::from_secs(30),
        |screen| {
            assert_eq!(screen.count("label:确认清理").unwrap(), 0);
            screen
                .texts()
                .iter()
                .any(|s| s.starts_with("清理结束  ·  3/3 项"))
        },
    )
    .unwrap();
    assert!(!fixture.tree.join("victim.bin").exists());
    assert!(!fixture.tree.join("changed.bin").exists());
    assert!(!fixture.tree.join("keep.bin").exists());
    assert!(!fixture.tree.join("nested/readme.txt").exists());
    assert!(fixture.root.join(".owner").exists());
    println!(
        "all_pages=primary_actions_visible startup_scan=pass estimate=nonzero direct_cleanup=pass directory_scope=preserved"
    );
}
