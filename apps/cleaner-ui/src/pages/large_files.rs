use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use crate::{
    commands::UiCommand,
    components::heading,
    controller::{UiController, UiSnapshot},
};

pub(crate) fn render(
    snapshot: Signal<UiSnapshot>,
    controller: Rc<RefCell<UiController>>,
) -> impl IntoElement {
    let state = snapshot.get();
    view()
        .flex_col()
        .gap_3()
        .child(heading("大文件", "扫描后查看占用空间较大的文件"))
        .child(
            view().flex_row().flex_wrap().gap_2().children(
                [
                    (10u64, "至少 10 MB"),
                    (50, "至少 50 MB"),
                    (100, "至少 100 MB"),
                    (500, "至少 500 MB"),
                    (1024, "至少 1 GB"),
                ]
                .into_iter()
                .map(|(mib, label)| {
                    let c = controller.clone();
                    let bytes = mib * 1024 * 1024;
                    crate::components::action(label, move || {
                        c.borrow_mut()
                            .dispatch(UiCommand::SetLargeFileThreshold(bytes));
                    })
                    .accessibility_state(move || AccessibilityState {
                        selected: state.large_file_min_bytes == bytes,
                        ..Default::default()
                    })
                }),
            ),
        )
        .child(
            text(format!(
                "共 {} 个大文件 · 第 {} 页（每页 80 行）",
                state.large_file_total,
                state.large_file_page + 1
            ))
            .text_color(crate::theme::color(crate::theme::MUTED)),
        )
        .child(
            text(format!(
                "总占用 {} · 已选 {} 项 · {}",
                crate::components::format_bytes(state.large_file_bytes),
                state.selected_large_file_count,
                crate::components::format_bytes(state.selected_large_file_bytes)
            ))
            .text_color(crate::theme::color(crate::theme::TEXT)),
        )
        .child(
            crate::components::action("生成所选清理预览", {
                let c = controller.clone();
                move || {
                    c.borrow_mut()
                        .dispatch(UiCommand::ExecuteCleanup(crate::commands::PlanId(2)));
                }
            })
            .disabled(state.cleanup_busy || state.selected_large_file_count == 0),
        )
        .children(state.large_files.into_iter().map(|file| {
            let c = controller.clone();
            let navigation = controller.clone();
            let explorer = controller.clone();
            let revealed_path = file.path.clone();
            let identity = file.identity;
            let copied_path = file.path.clone();
            let directory = std::path::Path::new(&file.path)
                .parent()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            view()
                .flex_col()
                .label("大文件项目")
                .gap_2()
                .px_3()
                .py_2()
                .bg(crate::theme::color(if file.selected {
                    crate::theme::SURFACE
                } else {
                    crate::theme::PANEL
                }))
                .child(
                    text(file.path.clone())
                        .w_full()
                        .label(file.path)
                        .text_color(crate::theme::color(crate::theme::TEXT)),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            text(crate::components::format_bytes(file.bytes))
                                .text_color(crate::theme::color(crate::theme::MUTED)),
                        )
                        .child(
                            text(format!(
                                "修改时间 {}",
                                crate::components::format_modified(file.modified)
                            ))
                            .text_color(crate::theme::color(crate::theme::MUTED)),
                        )
                        .child(crate::components::action("复制路径", move || {
                            nsh_core::element::ambient::set_clipboard(copied_path.clone())
                        }))
                        .child(crate::components::action(
                            "打开所在文件夹",
                            move || {
                                explorer
                                    .borrow_mut()
                                    .dispatch(UiCommand::RevealFile(revealed_path.clone()));
                            },
                        ))
                        .child(crate::components::action(
                            "在空间分析定位",
                            move || {
                                let mut c = navigation.borrow_mut();
                                c.dispatch(UiCommand::OpenAnalysisDirectory(directory.clone()));
                                c.dispatch(UiCommand::Navigate(
                                    crate::routes::ViewId::SpaceAnalysis,
                                ));
                            },
                        ))
                        .child(crate::components::action(
                            if file.selected {
                                "取消选择"
                            } else {
                                "选择"
                            },
                            move || {
                                c.borrow_mut()
                                    .dispatch(UiCommand::ToggleLargeFileSelection(identity));
                            },
                        )),
                )
        }))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(crate::components::action("上一页大文件", {
                    let c = controller.clone();
                    move || {
                        c.borrow_mut().dispatch(UiCommand::SetLargeFilePage(
                            state.large_file_page.saturating_sub(1),
                        ));
                    }
                }))
                .child(crate::components::action("下一页大文件", {
                    let c = controller.clone();
                    move || {
                        c.borrow_mut().dispatch(UiCommand::SetLargeFilePage(
                            (state.large_file_page + 1)
                                .min(state.large_file_total.saturating_sub(1) / 80),
                        ));
                    }
                })),
        )
}
