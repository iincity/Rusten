mod common;

use cleaner_ui::{configured_app, controller::ControllerConfig};
use common::UiFixture;
use nsh_core::{Bounds, MountBatch, MountItem, NodeId, Points, prelude::*};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

static CONFIG: OnceLock<ControllerConfig> = OnceLock::new();

#[component]
fn FixtureApp() -> impl IntoElement {
    configured_app(CONFIG.get().unwrap().clone())
}

#[derive(Default)]
struct Frames {
    labels: HashMap<NodeId, String>,
    frames: HashMap<NodeId, Bounds<Points>>,
}
impl Frames {
    fn apply(&mut self, batch: &MountBatch) {
        for item in &batch.items {
            match item {
                MountItem::Frame { id, frame } => {
                    self.frames.insert(*id, *frame);
                }
                MountItem::ViewProps { id, props } => {
                    if let Some(label) = &props.accessibility.label {
                        self.labels.insert(*id, label.to_string());
                    }
                }
                MountItem::Delete { id } => {
                    self.frames.remove(id);
                    self.labels.remove(id);
                }
                _ => {}
            }
        }
    }
    fn labelled(&self, label: &str) -> Bounds<Points> {
        let id = self
            .labels
            .iter()
            .find(|(_, value)| value.as_str() == label)
            .expect(label)
            .0;
        *self.frames.get(id).expect("labelled view has frame")
    }
}

#[test]
fn explorer_map_and_sidebar_have_contained_nonoverlapping_frames() {
    let fixture = UiFixture::create();
    std::fs::File::create(fixture.tree.join("large.bin"))
        .unwrap()
        .set_len(100 * 1024 * 1024)
        .unwrap();
    CONFIG
        .set(ControllerConfig {
            initial_scan_root: domain::normalize_path(&fixture.tree.to_string_lossy()).unwrap(),
            data_root: fixture.state.clone(),
        })
        .unwrap();
    for (width, height) in [(900.0, 620.0), (1180.0, 760.0), (1600.0, 1000.0)] {
        let mut config = EngineConfig::new(Root::of::<FixtureApp>());
        config.viewport = nsh_core::Size::new(Points(width), Points(height));
        config.ui_sets = vec![nsh_base::registry(), cleaner_ui::__nsh_native_module()];
        let mut app = Headless::start(config).unwrap();
        let frames = Arc::new(Mutex::new(Frames::default()));
        let observed = frames.clone();
        app.observe(move |_, batch| observed.lock().unwrap().apply(batch));
        app.wait_for("label:空间分析").unwrap();
        app.press("label:空间分析").unwrap();
        app.wait_for("label:空间图").unwrap();
        app.wait_for("label:目录侧栏").unwrap();
        let measured = frames.lock().unwrap();
        let map = measured.labelled("空间图");
        let sidebar = measured.labelled("目录侧栏");
        for frame in [map, sidebar] {
            assert!(frame.size.width.0 >= 260.0 && frame.size.height.0 > 0.0);
            assert!(frame.origin.x.0 >= 0.0 && frame.origin.y.0 >= 0.0);
            assert!(
                frame.origin.x.0 + frame.size.width.0 <= width - 208.0,
                "horizontal overflow at {width}x{height}: {frame:?}"
            );
        }
        assert!(
            map.origin.x.0 + map.size.width.0 <= sidebar.origin.x.0 + 0.1
                || map.origin.y.0 + map.size.height.0 <= sidebar.origin.y.0 + 0.1,
            "map/sidebar overlap at {width}x{height}: {map:?} / {sidebar:?}"
        );
        println!("viewport={width}x{height} map={map:?} sidebar={sidebar:?}");
        drop(measured);
        assert_eq!(app.screen().count("label:恢复").unwrap(), 0);
        cleaner_ui::request_window_close(nsh_platform_windows::windows::Win32::Foundation::HWND(
            1 as _,
        ));
        app.wait_for("label:确认退出").unwrap();
        let measured = frames.lock().unwrap();
        let panel = measured.labelled("确认退出 Rusten？");
        let cancel = measured.labelled("取消");
        let confirm = measured.labelled("确认退出");
        assert!((panel.size.width.0 - 440.0).abs() < 1.0);
        assert!(
            panel.size.height.0 >= 140.0 && panel.size.height.0 < 280.0,
            "{panel:?}"
        );
        assert!(cancel.origin.x.0 > 100.0);
        assert!(cancel.origin.x.0 + cancel.size.width.0 + 7.0 <= confirm.origin.x.0);
        assert!((cancel.origin.y.0 - confirm.origin.y.0).abs() < 1.0);
        assert_eq!(confirm.size.height.0, 36.0);
        drop(measured);
        app.press("label:取消").unwrap();
        app.press("label:大文件").unwrap();
        app.wait_for("label:大文件项目").unwrap();
        let measured = frames.lock().unwrap();
        let row = measured.labelled("大文件项目");
        let path = measured.labelled(fixture.tree.join("large.bin").to_str().unwrap());
        assert!(path.size.width.0 >= 300.0, "unreadable path: {path:?}");
        assert!(row.size.height.0 < 250.0, "overgrown file row: {row:?}");
    }
}
