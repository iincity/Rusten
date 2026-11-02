mod common;

use common::UiFixture;
use nsh_core::prelude::*;
use nsh_platform_windows as platform;

static FIXTURE_ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

#[component]
fn NativeAcceptanceApp() -> impl IntoElement {
    let root = FIXTURE_ROOT.get().expect("owned fixture supplied by test");
    assert_eq!(
        std::fs::read_to_string(root.join(".owner")).unwrap(),
        root.file_name().unwrap().to_str().unwrap()
    );
    cleaner_ui::configured_app(cleaner_ui::controller::ControllerConfig {
        initial_scan_root: domain::normalize_path(root.join("tree").to_str().unwrap()).unwrap(),
        data_root: root.join("state"),
    })
}

#[test]
#[ignore = "interactive Windows acceptance: closes when the native window is closed"]
fn native_owned_fixture() {
    let fixture = UiFixture::create();
    fixture.assert_owned_path(&fixture.tree);
    let size = match std::env::var("RUSTEN_NATIVE_SIZE").as_deref() {
        Ok("900x620") => (900, 620),
        Ok("1600x1000") => (1600, 1000),
        Ok("1180x760") | Err(_) => (1180, 760),
        Ok(other) => panic!("unsupported acceptance viewport: {other}"),
    };
    FIXTURE_ROOT.set(fixture.root.clone()).unwrap();
    println!(
        "native_fixture={} expected_files={} expected_bytes={} viewport={}x{}",
        fixture.root.display(),
        fixture.expected_files,
        fixture.expected_bytes,
        size.0,
        size.1
    );
    platform::set_ui_sets(vec![
        nsh_base::registry(),
        cleaner_ui::__nsh_native_module(),
    ]);
    assert_eq!(
        platform::run_app(
            platform::AppSource::Rust(
                platform::Root::of::<NativeAcceptanceApp>().app_id("com.rusten.acceptance")
            ),
            platform::AppOptions {
                app_id: "com.rusten.acceptance".into(),
                title: "Rusten — owned fixture acceptance".into(),
                size,
                appearance: platform::Appearance::Dark,
                close_handler: Some(cleaner_ui::request_window_close),
                ..Default::default()
            },
        ),
        std::process::ExitCode::SUCCESS
    );
}
