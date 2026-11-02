mod common;

use cleaner_ui::{
    commands::UiCommand,
    controller::{ControllerConfig, UiController},
};
use common::UiFixture;
use std::time::{Duration, Instant};

fn settle(
    controller: &mut UiController,
    ready: impl Fn(&cleaner_ui::controller::UiSnapshot) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        controller.drain();
        if ready(&controller.snapshot()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "worker timed out: {:?}",
            controller.snapshot()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn cleanup_removes_owned_files_without_copying_or_recovery_records() {
    let fixture = UiFixture::create();
    for index in 0..256 {
        std::fs::write(
            fixture.tree.join(format!("cache-{index}.tmp")),
            vec![0; 65536],
        )
        .unwrap();
    }
    let large = fixture.tree.join("large.bin");
    std::fs::File::create_new(&large)
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    fixture.assert_owned_path(&large);
    // An old configuration must not re-enable copies after removing recovery.
    std::fs::write(fixture.state.join("config.toml"), "backup_enabled = true\n").unwrap();
    let mut controller = UiController::with_config(ControllerConfig {
        initial_scan_root: domain::normalize_path(fixture.tree.to_str().unwrap()).unwrap(),
        data_root: fixture.state.clone(),
    });
    controller.dispatch(UiCommand::StartScan);
    settle(&mut controller, |s| {
        s.state.scan.status == cleaner_ui::state::ScanViewStatus::Complete && !s.cleanup_busy
    });
    println!(
        "scan_files={} selected={}",
        controller.snapshot().state.scan.files,
        controller.snapshot().state.cleanup.selected_count
    );
    let started = Instant::now();
    controller.dispatch(UiCommand::ExecuteCleanup(domain::PlanId(1)));
    assert!(controller.snapshot().cleanup_confirmation.is_none());
    assert_eq!(
        controller
            .snapshot()
            .cleanup_progress
            .unwrap()
            .progress
            .total,
        261
    );
    settle(&mut controller, |s| {
        s.cleanup_progress.as_ref().is_some_and(|p| p.finished)
    });
    let elapsed = started.elapsed();
    let result = controller.snapshot();
    let progress = result.cleanup_progress.unwrap().progress;
    println!(
        "cleanup_files={} elapsed_ms={} failed={} skipped={} bytes={}",
        progress.processed,
        elapsed.as_millis(),
        progress.failed,
        progress.skipped,
        progress.released_bytes
    );
    assert_eq!(progress.processed, 261);
    assert_eq!(progress.failed, 0);
    assert_eq!(progress.skipped, 0);
    assert!(
        elapsed < Duration::from_secs(5),
        "261-file local cleanup exceeded 5s: {elapsed:?}"
    );
    assert!(!large.exists());
    assert_eq!(std::fs::read_dir(&fixture.tree).unwrap().count(), 1);
    assert!(
        !fixture.state.join("backups.jsonl").exists(),
        "cleanup must not create recovery records"
    );
    assert!(
        !fixture.state.join("backups").exists(),
        "cleanup must not copy files"
    );
    assert_eq!(
        persistence::OperationDb::open(fixture.state.join("operations.sqlite"))
            .unwrap()
            .count()
            .unwrap(),
        261
    );
}
