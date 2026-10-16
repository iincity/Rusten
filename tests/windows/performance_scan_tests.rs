use std::{mem::size_of, sync::mpsc, thread, time::Instant};

use domain::{FileIdentity, FileRecord, ScanEvent, ScanIndex, normalize_path};
use platform_windows::{FileScanner, ScanOptions, WindowsFileScanner};
use windows_sys::Win32::{
    Foundation::FILETIME,
    System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

#[test]
fn scan_index_keeps_top_files_bounded() {
    let root = normalize_path(r"C:\RustenFixture").unwrap();
    let mut index = ScanIndex::new(root);
    let parent = index.root_directory_id();
    for value in 0..(ScanIndex::TOP_FILES_CAPACITY + 250) {
        let file = FileRecord::new(
            normalize_path(&format!(r"C:\RustenFixture\file-{value}.bin")).unwrap(),
            parent,
            FileIdentity {
                volume_serial: 1,
                file_index: value as u128,
            },
            value as u64,
            value as u64,
        );
        assert!(index.add_file(file));
    }
    assert_eq!(
        index.top_files(usize::MAX).len(),
        ScanIndex::TOP_FILES_CAPACITY
    );
}

#[test]
#[ignore = "creates a large disposable NTFS fixture; run explicitly with --ignored"]
fn one_hundred_thousand_file_scan_baseline() {
    scan_fixture("RUSTEN_PERF_FIXTURE_ROOT", 100_000);
}

#[test]
#[ignore = "creates a large disposable NTFS fixture; run explicitly with --ignored"]
fn one_million_file_scan_baseline() {
    scan_fixture("RUSTEN_PERF_FIXTURE_ROOT_1M", 1_000_000);
}

fn scan_fixture(variable: &str, minimum_files: u64) {
    let fixture_root = std::env::var(variable)
        .unwrap_or_else(|_| panic!("{variable} must be set for an explicit performance gate"));
    let root =
        normalize_path(&fixture_root).expect("fixture root must be a normalized Windows directory");
    let (sender, receiver) = mpsc::sync_channel(1024);
    let before = process_metrics().expect("Windows process metrics must be available");
    let start = Instant::now();
    let scan_root = root.clone();
    let scan_thread = thread::spawn(move || {
        WindowsFileScanner.scan(
            scan_root,
            ScanOptions {
                max_depth: 512,
                max_entries: minimum_files.saturating_add(10_000),
                collect_lock_state: true,
            },
            sender,
            Default::default(),
        )
    });
    let mut index = ScanIndex::new_with_path_matcher(root, platform_windows::paths_equal_ordinal);
    for event in receiver {
        match event {
            ScanEvent::Directory(directory) => {
                index.add_directory(directory);
            }
            ScanEvent::File(file) => {
                index.add_file(file);
            }
            ScanEvent::Warning(warning) => {
                index.add_warning(warning);
            }
            ScanEvent::Completed => {
                index.mark_complete();
            }
            ScanEvent::Partial => {
                index.mark_partial();
            }
            ScanEvent::Canceled => {
                index.mark_canceled();
            }
            ScanEvent::ReparsePoint(_)
            | ScanEvent::CleanupCandidate(_)
            | ScanEvent::Progress(_)
            | ScanEvent::ScannerStatus { .. } => {}
        }
    }
    let summary = scan_thread
        .join()
        .expect("scanner thread must not panic")
        .expect("scanner must complete on the disposable NTFS fixture");
    assert!(
        summary.file_count() >= minimum_files,
        "fixture has {} files, expected at least {minimum_files}",
        summary.file_count()
    );
    assert_eq!(index.statistics().file_count(), summary.file_count());
    let after = process_metrics().expect("Windows process metrics must be available");
    eprintln!(
        "{variable}: files={}, directories={}, elapsed_ms={}, cpu_ms={}, peak_working_set_mib={}, warnings={}",
        summary.file_count(),
        summary.directory_count(),
        start.elapsed().as_millis(),
        (after.cpu_time_100ns.saturating_sub(before.cpu_time_100ns)) / 10_000,
        after.peak_working_set_bytes / (1024 * 1024),
        summary.issues().len() + summary.omitted_issues() as usize
    );
}

struct ProcessMetrics {
    cpu_time_100ns: u64,
    peak_working_set_bytes: u64,
}

fn process_metrics() -> Option<ProcessMetrics> {
    unsafe {
        let process = GetCurrentProcess();
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) == 0 {
            return None;
        }
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        if GetProcessMemoryInfo(
            process,
            &mut counters,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ) == 0
        {
            return None;
        }
        Some(ProcessMetrics {
            cpu_time_100ns: filetime_to_u64(kernel) + filetime_to_u64(user),
            peak_working_set_bytes: counters.PeakWorkingSetSize as u64,
        })
    }
}

fn filetime_to_u64(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}
