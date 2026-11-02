use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use crate::{
    commands::{PlanId, UiCommand},
    components::{action, format_bytes},
    controller::{UiController, UiSnapshot},
    routes::ViewId,
    state::ScanViewStatus,
    theme,
};

pub(crate) fn render(state: UiSnapshot, controller: Rc<RefCell<UiController>>) -> impl IntoElement {
    let scan = &state.state.scan;
    let scanning = matches!(
        scan.status,
        ScanViewStatus::Scanning | ScanViewStatus::Paused
    );
    let cleaning = state.cleanup_progress.as_ref().is_some_and(|p| !p.finished);
    let cleanup_disabled = scanning
        || state.cleanup_busy
        || state.settings_pending
        || state.state.cleanup.selected_count == 0;
    let (scan_label, scan_command) = match scan.status {
        ScanViewStatus::Scanning => ("暂停扫描", UiCommand::PauseScan),
        ScanViewStatus::Paused => ("继续扫描", UiCommand::ResumeScan),
        ScanViewStatus::Complete | ScanViewStatus::Partial | ScanViewStatus::Canceled => {
            ("重新扫描", UiCommand::StartScan)
        }
        _ => ("开始扫描", UiCommand::StartScan),
    };
    let command = |label: &'static str, command: UiCommand| {
        let c = controller.clone();
        action(label, move || {
            c.borrow_mut().dispatch(command.clone());
        })
    };
    view()
        .flex_col()
        .gap_2()
        .px_5()
        .py_3()
        .bg(theme::color(theme::PANEL))
        .label("扫描与清理工具栏")
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(
                    text("预计可释放")
                        .text_size(px(16.0))
                        .text_color(theme::color(theme::TEXT)),
                )
                .child(
                    text(format_bytes(state.state.cleanup.estimated_bytes))
                        .label("预计可释放")
                        .text_size(px(24.0))
                        .text_color(theme::color(theme::ACCENT)),
                )
                .child(
                    text(format!(
                        "已选 {} 项 · {}",
                        state.state.cleanup.selected_count,
                        scan_status(scan.status)
                    ))
                    .text_color(theme::color(theme::MUTED)),
                )
                .child(
                    if state.cleanup_busy && !cleaning && state.cleanup_confirmation.is_none() {
                        text("正在生成清理计划…").text_color(theme::color(theme::MUTED))
                    } else {
                        text("")
                    },
                ),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(command(scan_label, scan_command).disabled(cleaning || state.cleanup_busy))
                .child(command("取消扫描", UiCommand::RequestCancelScan).disabled(!scanning))
                .child(
                    command("立即清理", UiCommand::ExecuteCleanup(PlanId(1)))
                        .bg(theme::color(if cleanup_disabled {
                            theme::SURFACE
                        } else {
                            theme::BORDER
                        }))
                        .disabled(cleanup_disabled),
                )
                .child(
                    command("快速清理预览", UiCommand::CreateQuickCleanPreview)
                        .disabled(scanning || state.cleanup_busy),
                )
                .child(if cleaning {
                    view()
                        .flex_row()
                        .gap_2()
                        .child(
                            command("停止清理", UiCommand::StopCleanup)
                                .disabled(state.cleanup_stopping),
                        )
                        .child(command(
                            "查看清理进度",
                            UiCommand::Navigate(ViewId::Cleanup),
                        ))
                } else {
                    view()
                }),
        )
        .child(
            view()
                .h(px(4.0))
                .w_full()
                .bg(theme::color(theme::BORDER))
                .label("扫描进度条")
                .child(
                    view()
                        .h_full()
                        .w(percentage(scan.progress_percent as f32 / 100.0))
                        .bg(theme::color(theme::ACCENT)),
                ),
        )
        .child(
            text(format!(
                "{} · {:?} · {} · 已扫描 {} 个文件 · {} · {}% · 警告 {}",
                scan_status(scan.status),
                scan.engine,
                scan_phase(scan.phase),
                scan.files,
                format_bytes(scan.discovered_bytes),
                scan.progress_percent,
                scan.warnings
            ))
            .text_size(px(12.0))
            .text_color(theme::color(theme::MUTED)),
        )
        .child(if let Some(path) = &scan.current_path {
            view().h(px(18.0)).overflow_hidden().child(
                text(format!("当前扫描：{path}"))
                    .text_size(px(12.0))
                    .text_color(theme::color(theme::MUTED)),
            )
        } else {
            view()
        })
        .child(if let Some(progress) = &state.cleanup_progress {
            view().child(
                text(format!(
                    "{} · 已处理 {}/{} 项 · 已释放 {} · 跳过 {} · 失败 {}",
                    if progress.finished {
                        "清理结束"
                    } else if state.cleanup_stopping {
                        "正在停止清理…"
                    } else {
                        "正在清理…"
                    },
                    progress.progress.processed,
                    progress.progress.total,
                    format_bytes(progress.progress.released_bytes),
                    progress.progress.skipped,
                    progress.progress.failed
                ))
                .text_size(px(12.0))
                .text_color(theme::color(theme::TEXT)),
            )
        } else {
            view()
        })
}

pub(crate) fn scan_status(status: ScanViewStatus) -> &'static str {
    match status {
        ScanViewStatus::NotScanned => "准备扫描",
        ScanViewStatus::Scanning => "正在扫描",
        ScanViewStatus::Paused => "扫描已暂停",
        ScanViewStatus::Partial => "扫描完成（部分结果）",
        ScanViewStatus::Complete => "扫描完成",
        ScanViewStatus::Canceled => "扫描已取消",
        ScanViewStatus::Failed => "扫描失败",
    }
}

fn scan_phase(phase: domain::ScanPhase) -> &'static str {
    match phase {
        domain::ScanPhase::Starting => "准备中",
        domain::ScanPhase::LoadingCache => "读取缓存",
        domain::ScanPhase::Indexing => "建立索引",
        domain::ScanPhase::Preparing => "准备扫描",
        domain::ScanPhase::Enumerating => "枚举文件",
        domain::ScanPhase::Finalizing => "整理结果",
        domain::ScanPhase::AnalyzingDuplicates => "分析重复文件",
        domain::ScanPhase::Fallback => "兼容扫描",
    }
}
