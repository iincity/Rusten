use std::{cell::RefCell, rc::Rc};

use nsh_core::{Checked, prelude::*};

use crate::{
    commands::{PlanId, UiCommand},
    components::{action, format_bytes, heading, row, section, value},
    controller::{UiController, UiSnapshot},
    theme,
};

pub(crate) fn render(
    snapshot: Signal<UiSnapshot>,
    controller: Rc<RefCell<UiController>>,
) -> impl IntoElement {
    let state = snapshot.get();
    view()
        .flex_col()
        .gap_3()
        .child(heading("垃圾清理", "选择清理来源，点击清理直接执行"))
        .child(value(
            "已选项目",
            state.state.cleanup.selected_count.to_string(),
        ))
        .child(value(
            "预计释放",
            format_bytes(state.state.cleanup.estimated_bytes),
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    action("生成快速清理预览", {
                        let c = controller.clone();
                        move || {
                            c.borrow_mut().dispatch(UiCommand::CreateQuickCleanPreview);
                        }
                    })
                    .disabled(state.cleanup_busy),
                )
                .child(
                    action("清理所选项目", {
                        let c = controller.clone();
                        move || {
                            c.borrow_mut()
                                .dispatch(UiCommand::ExecuteCleanup(PlanId(1)));
                        }
                    })
                    .disabled(state.cleanup_busy || state.state.cleanup.selected_count == 0),
                ),
        )
        .child(if let Some(progress) = &state.cleanup_progress {
            let p = &progress.progress;
            view()
                .flex_col()
                .gap_2()
                .p_3()
                .bg(theme::color(theme::PANEL))
                .child(section(if progress.finished {
                    "本次清理结果"
                } else {
                    "清理进度"
                }))
                .child(
                    text(format!(
                        "{}  ·  {}/{} 项  ·  已释放 {}  ·  跳过 {}  ·  失败 {}",
                        if progress.finished {
                            "清理结束"
                        } else if state.cleanup_stopping {
                            "正在停止清理…"
                        } else {
                            "正在清理"
                        },
                        p.processed,
                        p.total,
                        format_bytes(p.released_bytes),
                        p.skipped,
                        p.failed
                    ))
                    .text_color(theme::color(theme::TEXT)),
                )
                .child(
                    text(format!(
                        "本次计划预计释放 {} · 耗时 {} 秒 · 成功 {} 项",
                        format_bytes(state.cleanup_estimated_bytes),
                        state.cleanup_elapsed_seconds,
                        (p.processed as u64).saturating_sub(p.skipped + p.failed)
                    ))
                    .text_color(theme::color(theme::MUTED)),
                )
                .child(
                    view()
                        .w_full()
                        .h(px(6.0))
                        .bg(theme::color(theme::BORDER))
                        .child(
                            view()
                                .h_full()
                                .w(percentage(
                                    (p.processed as f32 / p.total.max(1) as f32).clamp(0.0, 1.0),
                                ))
                                .bg(theme::color(theme::ACCENT)),
                        ),
                )
                .child(
                    text(p.current_path.clone().unwrap_or_else(|| {
                        if progress.finished {
                            "本次结果已保留，可重新扫描核实剩余空间。".into()
                        } else if state.cleanup_stopping {
                            "停止请求已收到，等待当前文件安全处理完毕。".into()
                        } else {
                            "正在复核并删除所选文件…".into()
                        }
                    }))
                    .text_color(theme::color(theme::TEXT)),
                )
                .child(if progress.finished {
                    let change = state
                        .cleanup_disk_change
                        .map(|bytes| {
                            format!(
                                "{}{}",
                                if bytes < 0 { "−" } else { "+" },
                                format_bytes(bytes.unsigned_abs() as u64)
                            )
                        })
                        .unwrap_or_else(|| "读取失败".into());
                    text(format!("C 盘可用空间变化 {change}（包含其他程序的影响）"))
                        .text_color(theme::color(theme::MUTED))
                } else {
                    text("")
                })
                .children(state.cleanup_issues.iter().map(|item| {
                    text(format!("{}：{}", item.path, issue_reason(&item.result)))
                        .text_color(theme::color(theme::WARNING))
                }))
        } else {
            view()
        })
        .child(categories(state.clone(), controller))
        .child(text(state.error.unwrap_or_default()).text_color(theme::color(theme::DANGER)))
}

pub(crate) fn categories(
    state: UiSnapshot,
    controller: Rc<RefCell<UiController>>,
) -> impl IntoElement {
    let mut groups = std::collections::BTreeMap::<(u32, String), Vec<_>>::new();
    for category in &state.state.cleanup.categories {
        groups
            .entry((category.group_order, category.group.clone()))
            .or_default()
            .push(category);
    }
    view()
        .flex_col()
        .gap_3()
        .child(section("分类清理"))
        .children(groups.into_iter().map(|((_, group), categories)| {
            let collapsed = state.collapsed_cleanup_groups.contains(&group);
            let total = categories.iter().filter_map(|c| c.executable_bytes).sum();
            let selected = categories
                .iter()
                .filter(|c| state.state.cleanup.selected_categories.contains(&c.key))
                .filter_map(|c| c.executable_bytes)
                .sum();
            let keys = categories
                .iter()
                .filter(|c| c.executable_bytes.is_some())
                .map(|c| c.key.clone())
                .collect::<Vec<_>>();
            let toggle = controller.clone();
            let select = controller.clone();
            let group_key = group.clone();
            view()
                .flex_col()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            view()
                                .role(AccessibilityRole::Button)
                                .focusable(true)
                                .on_press(move |_: &()| {
                                    toggle
                                        .borrow_mut()
                                        .dispatch(UiCommand::ToggleCleanupGroup(group_key.clone()));
                                })
                                .label(format!("折叠 {group}"))
                                .child(
                                    text(format!("{} {group}", if collapsed { "▸" } else { "▾" }))
                                        .text_color(theme::color(theme::TEXT)),
                                ),
                        )
                        .child(
                            text(format!(
                                "可处理 {}，已选 {}",
                                format_bytes(total),
                                format_bytes(selected)
                            ))
                            .text_color(theme::color(theme::MUTED)),
                        )
                        .child(
                            action("切换整组选择", move || {
                                select
                                    .borrow_mut()
                                    .dispatch(UiCommand::ToggleCleanupGroupSelection(keys.clone()));
                            })
                            .disabled(state.cleanup_busy),
                        ),
                )
                .children(
                    categories
                        .into_iter()
                        .filter(|_| !collapsed)
                        .map(|category| {
                            let c = controller.clone();
                            let key = category.key.clone();
                            let inspect = controller.clone();
                            let inspect_key = category.key.clone();
                            let selected = state
                                .state
                                .cleanup
                                .selected_categories
                                .contains(&category.key);
                            let availability = match category.executable_bytes {
                                None => "统计中".into(),
                                Some(0) if category.estimated_bytes > 0 => "仅统计占用".into(),
                                Some(bytes) => format!("可清理 {}", format_bytes(bytes)),
                            };
                            row()
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .role(AccessibilityRole::Checkbox)
                                        .label(category.name.clone())
                                        .focusable(true)
                                        .disabled(
                                            state.cleanup_busy
                                                || category.executable_bytes.is_none(),
                                        )
                                        .accessibility_state(move || AccessibilityState {
                                            checked: Some(if selected {
                                                Checked::True
                                            } else {
                                                Checked::False
                                            }),
                                            ..Default::default()
                                        })
                                        .on_press(move |_: &()| {
                                            c.borrow_mut().dispatch(
                                                UiCommand::ToggleCleanupCategory(key.clone()),
                                            );
                                        })
                                        .child(
                                            text(format!(
                                                "{} {}",
                                                if selected { "☑" } else { "☐" },
                                                category.name
                                            ))
                                            .text_size(px(14.0))
                                            .text_color(theme::color(theme::TEXT)),
                                        )
                                        .child(
                                            text(format!(
                                                "{availability} · 占用 {} · {} 项",
                                                format_bytes(category.estimated_bytes),
                                                category.count
                                            ))
                                            .text_size(px(12.0))
                                            .text_color(theme::color(theme::MUTED)),
                                        ),
                                )
                                .child(
                                    action("查看来源明细", move || {
                                        let mut c = inspect.borrow_mut();
                                        c.dispatch(UiCommand::InspectCleanupCategory(
                                            inspect_key.clone(),
                                        ));
                                        c.dispatch(UiCommand::Navigate(
                                            crate::routes::ViewId::SpaceAnalysis,
                                        ));
                                    })
                                    .label(format!("查看{}明细", category.name)),
                                )
                        }),
                )
        }))
}

fn issue_reason(result: &application::CleanupItemResult) -> String {
    use application::CleanupItemResult::*;
    match result {
        SkippedLocked => "文件正在使用，已跳过".into(),
        SkippedPermission => "权限不足，已跳过".into(),
        SkippedChanged => "文件已变化或不满足清理条件，已跳过".into(),
        SkippedForbidden => "受保护的路径，已跳过".into(),
        RestartRequired => "已更新配置，需要重启后释放空间".into(),
        Failed(error) => format!("处理失败：{error}"),
        other => format!("{other:?}"),
    }
}
