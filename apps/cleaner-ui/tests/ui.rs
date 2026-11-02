mod common;

use cleaner_ui::{configured_app, controller::ControllerConfig};
use common::UiFixture;
use domain::normalize_path;
use nsh_core::prelude::*;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use std::{sync::OnceLock, time::Duration};

static CONFIG: OnceLock<ControllerConfig> = OnceLock::new();

#[component]
fn FixtureApp() -> impl IntoElement {
    configured_app(CONFIG.get().unwrap().clone())
}

fn start(width: f32, height: f32) -> Headless {
    let mut config = EngineConfig::new(Root::of::<FixtureApp>());
    config.viewport = nsh_core::Size::new(nsh_core::Points(width), nsh_core::Points(height));
    config.ui_sets = vec![nsh_base::registry(), cleaner_ui::__nsh_native_module()];
    Headless::start(config).unwrap()
}

fn start_scan(app: &mut Headless) {
    let label = if app.screen().count("label:开始扫描").unwrap() > 0 {
        "label:开始扫描"
    } else {
        "label:重新扫描"
    };
    app.press(label).unwrap();
}

#[test]
fn production_shell_scans_and_cleans_only_owned_fixture() {
    let fixture = UiFixture::create();
    fixture.assert_owned_path(&fixture.tree.join("victim.bin"));
    CONFIG
        .set(ControllerConfig {
            initial_scan_root: normalize_path(&fixture.tree.to_string_lossy()).unwrap(),
            data_root: fixture.state.clone(),
        })
        .unwrap();
    println!("fixture={}", fixture.root.display());
    {
        let mut app = start(1180.0, 760.0);
        app.wait_until("scan complete", Duration::from_secs(30), |screen| {
            screen.texts().contains(&"Complete")
        })
        .unwrap();
        app.wait_for("4 个文件  ·  100%").unwrap();
        app.wait_for(&format!("扫描逻辑字节：{} B", fixture.expected_bytes))
            .unwrap();
        println!(
            "expected_files={} observed_files=4 expected_bytes={} observed_bytes=48",
            fixture.expected_files, fixture.expected_bytes
        );
        app.press("label:设置").unwrap();
        app.press("label:暗黑主题").unwrap();
        app.wait_until("settings saved", Duration::from_secs(5), |_| {
            std::fs::read_to_string(fixture.state.join("config.toml"))
                .is_ok_and(|s| s.contains("theme = \"dark\""))
        })
        .unwrap();
        app.press("label:空间分析").unwrap();
        app.wait_for("label:victim.bin").unwrap();
        app.press("label:victim.bin").unwrap();
        app.press_key("space").unwrap();
        app.press("label:审查标记项目").unwrap();
        app.wait_for("label:确认清理").unwrap();
        app.press_key("escape").unwrap();
        app.wait_for_gone("label:确认清理", Duration::from_secs(5))
            .unwrap();
        assert!(fixture.tree.join("victim.bin").exists());
        app.press("label:审查标记项目").unwrap();
        app.wait_for("label:确认清理").unwrap();
        let victim = fixture.tree.join("victim.bin");
        fixture.assert_owned_path(&victim);
        app.press("label:确认清理").unwrap();
        app.wait_until("cleanup complete", Duration::from_secs(30), |_| {
            !victim.exists()
        })
        .unwrap();
        assert_eq!(
            std::fs::read(fixture.tree.join("keep.bin")).unwrap(),
            b"never-change"
        );
        app.wait_until("cleanup result", Duration::from_secs(10), |screen| {
            screen.texts().iter().any(|text| {
                text.starts_with("清理结束  ·  1/1 项") && text.ends_with("跳过 0  ·  失败 0")
            })
        })
        .unwrap();
        app.press("label:空间分析").unwrap();
        app.wait_until(
            "removed file disappears from projection",
            Duration::from_secs(5),
            |screen| screen.count("label:victim.bin").unwrap_or(1) == 0,
        )
        .unwrap();
        drop(app);
        let mut app = start(900.0, 620.0);
        app.wait_for("label:设置").unwrap();
        app.press("label:设置").unwrap();
        app.wait_for("暗黑").unwrap();
        assert_eq!(app.screen().count("label:恢复").unwrap(), 0);
        assert_eq!(app.screen().count("label:清理前备份开关").unwrap(), 0);
        assert!(!victim.exists());
        assert!(!fixture.state.join("backups").exists());
        assert!(!fixture.state.join("backups.jsonl").exists());
        drop(app);
        let mut app = start(1180.0, 760.0);
        app.wait_for("label:首页").unwrap();
        app.press("label:首页").unwrap();
        app.wait_until("rescan complete", Duration::from_secs(30), |screen| {
            screen.texts().contains(&"Complete")
        })
        .unwrap();
        app.press("label:空间分析").unwrap();
        app.wait_for("label:changed.bin").unwrap();
        app.press("label:changed.bin").unwrap();
        app.press_key("space").unwrap();
        app.press("label:审查标记项目").unwrap();
        app.wait_for("label:确认清理").unwrap();
        let changed = fixture.tree.join("changed.bin");
        fixture.assert_owned_path(&changed);
        std::fs::rename(&changed, fixture.tree.join("identity-original.bin")).unwrap();
        std::fs::write(&changed, b"identity-after!").unwrap();
        fixture.assert_owned_path(&changed);
        app.press("label:确认清理").unwrap();
        app.wait_until("identity rejected", Duration::from_secs(15), |screen| {
            screen
                .texts()
                .iter()
                .any(|t| t.contains("文件已变化或不满足清理条件"))
        })
        .unwrap();
        assert_eq!(std::fs::read(&changed).unwrap(), b"identity-after!");
        println!("identity=SkippedChanged replacement_retained=true");
        app.press("label:空间分析").unwrap();
        app.wait_for("label:changed.bin").unwrap();
        app.press("label:changed.bin").unwrap();
        app.press_key("space").unwrap();
        app.wait_for("label:nested").unwrap();
        app.press("label:nested").unwrap();
        app.press_key("enter").unwrap();
        app.wait_for("label:readme.txt").unwrap();
        assert_eq!(
            app.screen().input_text("label:目录路径").unwrap(),
            Some(fixture.tree.join("nested").to_str().unwrap())
        );
        app.press("label:readme.txt").unwrap();
        app.press_key("space").unwrap();
        app.press("label:审查标记项目").unwrap();
        app.wait_for("label:确认清理").unwrap();
        let recycled = fixture.tree.join("nested/readme.txt");
        fixture.assert_owned_path(&recycled);
        app.press("label:确认清理").unwrap();
        app.wait_until("recycle source removed", Duration::from_secs(30), |_| {
            !recycled.exists()
        })
        .unwrap();
        app.wait_until("recycle result", Duration::from_secs(10), |screen| {
            screen.texts().iter().any(|text| {
                text.starts_with("清理结束  ·  1/1 项") && text.ends_with("跳过 0  ·  失败 0")
            })
        })
        .unwrap();
        assert!(!fixture.state.join("backups.jsonl").exists());
        assert_eq!(
            std::fs::read(fixture.tree.join("keep.bin")).unwrap(),
            b"never-change"
        );
        let db = persistence::OperationDb::open(fixture.state.join("operations.sqlite")).unwrap();
        assert!(db.count().unwrap() >= 3);
        println!("permanent_cleanup=pass persistence=pass no_backup=true unselected_retained=true");
        let large = fixture.tree.join("large.bin");
        std::fs::File::create_new(&large)
            .unwrap()
            .set_len(100 * 1024 * 1024)
            .unwrap();
        fixture.assert_owned_path(&large);
        app.press("label:首页").unwrap();
        start_scan(&mut app);
        app.wait_until("large file scan", Duration::from_secs(30), |screen| {
            screen.texts().contains(&"Complete")
        })
        .unwrap();
        app.wait_for("扫描逻辑字节：104857642 B").unwrap();
        app.press("label:大文件").unwrap();
        app.wait_for("共 1 个大文件 · 第 1 页（每页 80 行）")
            .unwrap();
        app.wait_for("100 MB").unwrap();
        app.press("label:至少 500 MB").unwrap();
        app.wait_for("共 0 个大文件 · 第 1 页（每页 80 行）")
            .unwrap();
        app.press("label:至少 100 MB").unwrap();
        app.wait_for("共 1 个大文件 · 第 1 页（每页 80 行）")
            .unwrap();
        app.press("label:复制路径").unwrap();
        assert_eq!(app.screen().clipboard(), Some(large.to_str().unwrap()));
        app.press("label:在空间分析定位").unwrap();
        app.wait_for("label:large.bin").unwrap();
        assert_eq!(
            app.screen().input_text("label:目录路径").unwrap(),
            Some(fixture.tree.to_str().unwrap())
        );
        app.press("label:大文件").unwrap();
        app.press("label:选择").unwrap();
        app.press("label:生成所选清理预览").unwrap();
        app.wait_for("label:确认清理").unwrap();
        app.press_key("escape").unwrap();
        app.wait_for_gone("label:确认清理", Duration::from_secs(5))
            .unwrap();
        assert_eq!(std::fs::metadata(large).unwrap().len(), 100 * 1024 * 1024);
        println!("large_files=1 logical_bytes=104857642 review_cancel_preserves_source=true");
        for number in 0..85 {
            std::fs::write(fixture.tree.join(format!("item-{number:03}.txt")), b"x").unwrap();
        }
        app.press("label:首页").unwrap();
        start_scan(&mut app);
        app.wait_until("paged scan", Duration::from_secs(30), |screen| {
            screen.texts().contains(&"Complete")
        })
        .unwrap();
        app.press("label:空间分析").unwrap();
        app.wait_for("目录项目 1-80 / 90").unwrap();
        app.press("label:下一页").unwrap();
        app.wait_for("目录项目 81-90 / 90").unwrap();
        app.press("label:上一页").unwrap();
        app.type_text("label:筛选目录", "item-084").unwrap();
        app.submit("label:筛选目录").unwrap();
        app.wait_for("目录项目 1-1 / 1").unwrap();
        assert_eq!(app.screen().count("label:nested").unwrap(), 0);
        app.press("label:放大").unwrap();
        app.wait_for("缩放 125% · 标记 0 项").unwrap();
        app.press_key("ctrl+0").unwrap();
        app.wait_for("缩放 100% · 标记 0 项").unwrap();
        app.drag(
            "label:空间图",
            nsh_core::Point::new(Points(30.0), Points(20.0)),
        )
        .unwrap();
        app.press("label:适应窗口").unwrap();
        app.press("label:文件数量").unwrap();
        app.press("label:分配大小").unwrap();
        app.press("label:逻辑大小").unwrap();
        app.type_text("label:目录路径", "C:\\not-in-the-fixture")
            .unwrap();
        app.submit("label:目录路径").unwrap();
        app.wait_for("目录不在扫描索引中，请检查路径或重新扫描")
            .unwrap();
        println!(
            "directory_pages=80+10 filter_beyond_first_page=pass zoom_fit_pan_metrics=driven missing_path=reported"
        );
    }
    for (width, height) in [(900.0, 620.0), (1600.0, 1000.0)] {
        let mut app = start(width, height);
        app.wait_for("label:首页").unwrap();
        for label in ["空间分析", "垃圾清理", "大文件", "C盘专清", "设置", "首页"]
        {
            app.press(&format!("label:{label}")).unwrap();
        }
    }
}
